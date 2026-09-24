//! What a worker session is charged for. Rulings T15-I2 and T15-I3: its minutes
//! (decision 40) and its stall silence (decision 32) count while its task is working
//! (or its session launching) in a running run; ruling T15-R3: and whenever a worker
//! turn is open, whatever the task's or the run's state. Otherwise (blocked, held, in
//! a gate, or the run paused or halted, with no turn open) the task's clock is stopped
//! (`Task.clock`); when it runs again, every worker round is excused the stopped span
//! it overlapped. Ruling T15-C1: `run retry` starts a new budget epoch, and rung 4
//! counts from it; `spent_total` keeps the whole for the report.

use proto::{AgentRole, RunState, Spend, TaskState};
use serde::{Deserialize, Serialize};

use super::dispatch::history;
use super::ladder::{breached, round_spend, worker_round};
use super::{Effect, INTERRUPT_GRACE_SECS, outbox};
use crate::run::contract::stall_nudge;
use crate::run::model::{Run, StallState, Task};

/// The spend before a `run retry` (ruling T15-C1).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetEpoch {
    /// The index of the epoch's first round.
    pub round: usize,
    /// `spent_total`'s tool calls and tokens at the retry.
    pub tool_calls: u32,
    pub tokens: u64,
}

/// The engine second at which at least `secs` real seconds have passed since `now`
/// (M8a.24). The engine's `now` is the unix time truncated to a whole second, so
/// `now + secs` can come as little as `secs - 1` real seconds later; one second more
/// makes a wait that decision 32 promises (`rate_limit_retry_secs`) a lower bound.
pub(super) fn not_before(now: u64, secs: u64) -> u64 {
    now + secs + 1
}

/// A task's clock: since when it is stopped, if it is, and when it last restarted.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskClock {
    pub stopped: Option<u64>,
    /// The end of the last stop the rounds were excused (ruling T15-R2).
    pub restarted: u64,
}

/// At the end of each scheduler pass: a task's clock stops when it stops working or
/// the run stops running and no worker turn is open, and restarts when either holds
/// again. The stall silence needs no shift: the watchdog watches only open turns, and
/// an open turn keeps the clock running (ruling T15-R3).
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
            )
            || open_turn(task).is_some();
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

/// Ruling T15-R3: the index of the task's latest worker round if it is live with a turn
/// open. Its clock runs whatever the task's or the run's state, and the watchdog
/// watches it (`signals::watch_open_turns`).
pub(super) fn open_turn(task: &Task) -> Option<usize> {
    worker_round(task).filter(|&r| {
        let round = &task.rounds[r];
        round.turn_open && round.window_id.is_some() && !round.ended && !round.retiring
    })
}

/// Ruling T15-R3: the watchdog on an open worker turn that `signals::watch` does not
/// see (its task not working, or the run not running). A turn past its session's hard
/// budget, or silent for `stall_after_secs`, is interrupted once; the task keeps its
/// state, and the ladder applies once it works again. A working task's silence (its run
/// paused or halted) is the watchdog's first stage, as in `signals::watch`: its
/// `stall_nudge` is queued for the resume. A blocked or gated task gets none: its
/// answer or its gate's result re-engages it.
pub(super) fn watch_open_turns(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let running = run.state == RunState::Running;
    let stall_after = run.limits.stall_after_secs;
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        if running && task.state == TaskState::Working {
            continue;
        }
        let Some(r) = open_turn(task) else {
            continue;
        };
        let round = &task.rounds[r];
        if round.interrupted {
            continue;
        }
        let spend = round_spend(round, task.clock.stopped, now);
        let quiet = round.last_event.max(round.rate_limited_until.unwrap_or(0));
        let why = if let Some(what) = breached(spend, task.budget) {
            format!("its open turn passed its budget ({what})")
        } else if now >= quiet + stall_after && task.claim.is_none() {
            // Ruling T12-later (T15-R4): no stall while its `task_done` is being checked.
            "its open turn made no progress".to_string()
        } else {
            continue;
        };
        let state = if running {
            task.state.label()
        } else {
            run.state.label()
        };
        let silent = breached(spend, task.budget).is_none();
        let working = task.state == TaskState::Working;
        let window_id = round.window_id.unwrap_or_default();
        fx.push(Effect::Interrupt { window_id });
        let round = &mut run.tasks[i].rounds[r];
        round.interrupted = true;
        if silent && working {
            round.stall = StallState::Interrupted {
                deadline: now + INTERRUPT_GRACE_SECS,
            };
            let id = run.tasks[i].id().to_string();
            outbox::queue(run, &id, stall_nudge(stall_after / 60), now);
        }
        history(run, i, now, format!("{why} while {state}; interrupting it"));
    }
}

/// A restore stops a running task clock at its session's last sign of life: the
/// downtime before the restore is no session time either. Ruling T15-R2 (N-2): never
/// before the round ended or the clock last restarted, so no span is excused twice.
pub(super) fn stop_at_restore(task: &mut Task) {
    if task.clock.stopped.is_some() {
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
    }
}

/// The spend rung 4 weighs (decision 38): the task's since its last retry.
pub(crate) fn epoch_spend(task: &Task, now: u64) -> Spend {
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
