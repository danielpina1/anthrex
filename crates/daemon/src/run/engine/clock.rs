//! What a worker session is charged for. Rulings T15-I2 and T15-I3: its minutes
//! (decision 40) and its stall silence (decision 32) count only while its task is
//! working (or its session launching) in a running run. Blocked, held, in the gates,
//! or with the run paused or halted, the task's clock is stopped (`Task.clock`); when both hold again,
//! every worker round is excused the stopped span it overlapped. Ruling T15-C1: `run
//! retry` starts a new budget epoch, and rung 4 counts from it; `spent_total` keeps the
//! whole for the report.

use proto::{AgentRole, RunState, Spend, TaskState};
use serde::{Deserialize, Serialize};

use super::ladder::{round_spend, worker_round};
use crate::run::model::{Run, Task};

/// The spend before a `run retry` (ruling T15-C1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetEpoch {
    /// The index of the epoch's first round.
    pub round: usize,
    /// `spent_total`'s tool calls and tokens at the retry.
    pub tool_calls: u32,
    pub tokens: u64,
}

/// A task's clock: since when it is stopped, if it is, and when it last restarted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskClock {
    pub stopped: Option<u64>,
    /// The end of the last stop the rounds were excused (ruling T15-R2).
    pub restarted: u64,
}

/// Each scheduler pass, before and after its work: a task's clock stops when it stops
/// working or the run stops running, and restarts when both hold again.
pub(super) fn sync(run: &mut Run, now: u64) {
    let running = run.state == RunState::Running;
    for task in run.tasks.iter_mut() {
        if task.state.is_finished() {
            task.clock.stopped = None;
            continue;
        }
        // Blocked (held included), in a gate or in the merge queue: stopped. A session
        // being launched is the worker's already.
        let charging = running
            && !matches!(
                task.state,
                TaskState::Blocked
                    | TaskState::Proof
                    | TaskState::Check
                    | TaskState::Review
                    | TaskState::MergeQueue
            );
        match task.clock.stopped {
            None if !charging => task.clock.stopped = Some(now),
            Some(since) if charging => {
                task.clock = TaskClock {
                    stopped: None,
                    restarted: now,
                };
                restart(task, since, now);
            }
            _ => {}
        }
    }
}

/// A restore stops a working task's clock at its session's last sign of life: the
/// downtime before the restore is no session time either. Ruling T15-R2 (N-2): never
/// before the round ended or the clock last restarted, so no span is excused twice.
pub(super) fn stop_at_restore(task: &mut Task) {
    if task.state != TaskState::Working || task.clock.stopped.is_some() {
        return;
    }
    let restarted = task.clock.restarted;
    let at = worker_round(task).map(|r| {
        let round = &task.rounds[r];
        let ended = round.ended_at.filter(|_| round.ended).unwrap_or(0);
        round
            .last_event
            .max(round.started_at)
            .max(ended)
            .max(restarted)
    });
    task.clock.stopped = at;
}

/// The clock restarts at `now` after a stop at `since`.
fn restart(task: &mut Task, since: u64, now: u64) {
    let latest = worker_round(task);
    for (r, round) in task.rounds.iter_mut().enumerate() {
        if round.role != AgentRole::Worker {
            continue;
        }
        let from = since.max(round.started_at);
        let resumable = round.ended && !round.retiring && round.session_id.is_some();
        if resumable && Some(r) == latest {
            // A resumed session is charged from its start, so the whole stop is excused,
            // and the round counts as ended now until it is resumed.
            round.excused_secs += now.saturating_sub(from);
            round.ended_at = Some(now);
        } else {
            let end = round.ended_at.unwrap_or(now);
            round.excused_secs += end.saturating_sub(from);
        }
        // Ruling T15-R2: never more than the round has lasted, so no spend is negative
        // and no excused time is banked as credit.
        let elapsed = round
            .ended_at
            .unwrap_or(now)
            .saturating_sub(round.started_at);
        round.excused_secs = round.excused_secs.min(elapsed);
        // The silence before the stop still counts; the stop's does not.
        if !round.ended {
            round.last_event += now.saturating_sub(since.max(round.last_event));
        }
    }
}

/// The spend rung 4 weighs (decision 38): the task's since its last retry.
pub(super) fn epoch_spend(task: &Task, now: u64) -> Spend {
    let epoch = task.epoch.unwrap_or_default();
    let secs = task
        .rounds
        .iter()
        .enumerate()
        .filter(|(r, round)| *r >= epoch.round && round.role == AgentRole::Worker)
        .map(|(_, round)| round_spend(round, task.clock.stopped, now).secs)
        .sum();
    Spend {
        tool_calls: task.spent_total.tool_calls.saturating_sub(epoch.tool_calls),
        secs,
        tokens: task.spent_total.tokens.saturating_sub(epoch.tokens),
    }
}

/// `run retry` starts a new epoch at the fresh session's round.
pub(super) fn new_epoch(task: &mut Task) {
    task.epoch = Some(BudgetEpoch {
        round: task.rounds.len(),
        tool_calls: task.spent_total.tool_calls,
        tokens: task.spent_total.tokens,
    });
}
