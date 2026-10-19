//! What a worker session is charged for. Rulings T15-I2 and T15-I3: its minutes
//! (decision 40) and its stall silence (decision 32) count while its task is working
//! (or its session launching) in a running run; ruling T15-R3: and whenever a worker
//! turn is open, whatever the task's or the run's state. Otherwise (blocked, held, in
//! a gate, or the run paused or halted, with no turn open) the task's clock is stopped
//! (`Task.clock`); when it runs again, every worker round is excused the stopped span
//! it overlapped. Ruling T15-C1: `run retry` starts a new budget epoch, and rung 4
//! counts from it; `spent_total` keeps the whole for the report.

use proto::{RunState, Spend, TaskState};
use serde::{Deserialize, Serialize};

use super::dispatch::history;
use super::ladder::{breached, round_spend, worker_round};
use super::race_end::LANE_EXIT_WAIT_SECS;
use super::{Effect, INTERRUPT_GRACE_SECS, outbox};
use crate::run::contract::stall_nudge;
use crate::run::design::state::DesignAgentState;
use crate::run::model::{AgentRound, FailedTurn, Run, StallState, Task};

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

/// When a round's open turn has been silent for `stall_after` seconds (decision 32):
/// at least that long after its last event, and after the end of a rate-limit wait
/// (`rate_limited_until`, itself already a whole-seconds lower bound). Ruling
/// T24-clock.
pub(super) fn stall_due(round: &AgentRound, stall_after: u64) -> u64 {
    let after_event = not_before(round.last_event, stall_after);
    round
        .rate_limited_until
        .map_or(after_event, |until| after_event.max(until + stall_after))
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
        let why = if let Some(what) = breached(spend, task.budget) {
            format!("its open turn passed its budget ({what})")
        } else if now >= stall_due(round, stall_after) && task.claim.is_none() {
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
                deadline: not_before(now, INTERRUPT_GRACE_SECS),
            };
            let id = run.tasks[i].id().to_string();
            outbox::queue(run, &id, stall_nudge(stall_after), now);
        }
        history(run, i, now, format!("{why} while {state}; interrupting it"));
    }
}

/// `EventKind::Slept`: the host slept `secs` up to `now`. Asleep, no session ran, so
/// none is charged for it or waits through it: every duration still running at the
/// sleep's start moves by `secs`. A round that has not ended has its stall clock's
/// anchor (`last_event`, as a restore re-arms it) moved, and, while its clock runs, the
/// span excused from its minutes (decision 40 counts awake time); a wait still to come
/// at the sleep's start (a rate limit, a delivery or count retry, the interrupt's grace,
/// a failed turn's continue, a stopped racer's exit wait) ends `secs` later; a running
/// design agent's start and a design phase's budget clock are shifted, as
/// `design::restored` shifts the latter for a restart. A stopped clock excuses the span
/// already. Timestamps of what happened stay as they were: wall times.
pub(super) fn slept(run: &mut Run, secs: u64, now: u64) {
    let asleep = now.saturating_sub(secs);
    let later = |at: &mut u64| {
        if *at > asleep {
            *at = at.saturating_add(secs);
        }
    };
    for task in run.tasks.iter_mut() {
        let stopped: Vec<bool> = (task.rounds.iter())
            .map(|r| round_clock(task, r).stopped.is_some())
            .collect();
        for (round, stopped) in task.rounds.iter_mut().zip(stopped) {
            if round.ended_at.is_none() && !round.ended {
                round.last_event = round.last_event.saturating_add(secs).min(now);
                if !stopped {
                    let elapsed = now.saturating_sub(round.started_at);
                    round.excused_secs = round.excused_secs.saturating_add(secs).min(elapsed);
                }
            }
            waits(round).into_iter().for_each(later);
        }
    }
    for task in run.tasks.iter_mut() {
        // A stopped lane's exit wait (review M1): a time input of the salvage only.
        let lanes = task.race.iter_mut().flat_map(|r| r.lanes.iter_mut());
        for at in lanes.filter_map(|l| l.kill_sent_at.as_mut()) {
            if at.saturating_add(LANE_EXIT_WAIT_SECS) > asleep {
                *at = at.saturating_add(secs).min(now);
            }
        }
    }
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    // A running design agent's session seconds (review M4, `design_spend`).
    let agents = design
        .brainstormers
        .iter_mut()
        .chain(design.reviewer.as_mut());
    for agent in agents.filter(|a| a.state == DesignAgentState::Running) {
        if let Some(started) = agent.started.as_mut() {
            *started = started.saturating_add(secs).min(now);
        }
    }
    if !matches!(run.state, RunState::Paused | RunState::Halted)
        && let Some(started) = design.phase_started.as_mut()
    {
        *started = started.saturating_add(secs).min(now);
    }
}

/// `EventKind::SteppedBack`: the wall clock was set `secs` back at `now`. Every anchor
/// and deadline [`slept`] moves keeps the awake time it had left on the clock before
/// the step: an anchor (a stall clock's last event, a session's or a phase's start, a
/// stopped racer's kill) moves back by `secs`, so the span since it is unchanged; a
/// wait still to come ends at `now` plus what remained of it, and one that had come is
/// clamped to `now`, so it stays come. A brief forward step (which the driver measures
/// as a sleep) and its correction so leave every wait as it was. Every round's elapsed
/// minutes shrink by the step, so a round not ended has its excused minutes shrink too,
/// down to none (an undercount past that: late, never early).
///
/// Review round 1 of this fix: anchors `slept` leaves are moved back too, since one
/// stamped while the clock was ahead would otherwise wait out the step. A task's and a
/// racing lane's clock (`stopped`, so a stop keeps its span, which is why a stopped
/// round's excused minutes shrink as a running one's do; `restarted`, so a restore
/// stops the clock no later than `now`, M2), the orchestrator's wait (`orch_stall`'s
/// `waiting_since`) and each runtime's writer-cap anchors (`concurrency`'s
/// `last_rate_limit_at`, `last_change_at`, M3). A paused design phase keeps its start:
/// its pause and its age shrink alike, unless the pause so far is shorter than the step,
/// where the pause saturates and the phase undercounts (late, never early).
///
/// Timestamps of what happened stay as they were: wall times.
pub(super) fn stepped_back(run: &mut Run, secs: u64, now: u64) {
    let before = now.saturating_add(secs);
    let back = |at: &mut u64| *at = (*at).min(now + at.saturating_sub(before));
    let anchor = |at: &mut u64| *at = at.saturating_sub(secs).min(now);
    let clock = |c: &mut TaskClock| {
        c.stopped.as_mut().into_iter().for_each(anchor);
        anchor(&mut c.restarted);
    };
    for task in run.tasks.iter_mut() {
        for round in task.rounds.iter_mut() {
            if round.ended_at.is_none() && !round.ended {
                anchor(&mut round.last_event);
                round.excused_secs = round.excused_secs.saturating_sub(secs);
            }
            waits(round).into_iter().for_each(back);
        }
        clock(&mut task.clock);
        for lane in task.race.iter_mut().flat_map(|r| r.lanes.iter_mut()) {
            lane.kill_sent_at.as_mut().into_iter().for_each(anchor);
            clock(&mut lane.gates.clock);
        }
    }
    run.orch.waiting_since.as_mut().into_iter().for_each(anchor);
    for c in run.concurrency.values_mut() {
        let at = [c.last_rate_limit_at.as_mut(), c.last_change_at.as_mut()];
        at.into_iter().flatten().for_each(anchor);
    }
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    let agents = design
        .brainstormers
        .iter_mut()
        .chain(design.reviewer.as_mut());
    let running = agents.filter(|a| a.state == DesignAgentState::Running);
    running.filter_map(|a| a.started.as_mut()).for_each(anchor);
    if !matches!(run.state, RunState::Paused | RunState::Halted)
        && let Some(started) = design.phase_started.as_mut()
    {
        anchor(started);
    }
}

/// A round's deadlines that [`slept`] and [`stepped_back`] move: a rate limit's end, a
/// delivery or count retry, the interrupt's grace and a failed turn's continue.
fn waits(round: &mut AgentRound) -> Vec<&mut u64> {
    let mut waits: Vec<&mut u64> = [
        round.rate_limited_until.as_mut(),
        round.delivery_retry_at.as_mut(),
        round.count_retry_at.as_mut(),
    ]
    .into_iter()
    .flatten()
    .collect();
    if let StallState::Interrupted { deadline } = &mut round.stall {
        waits.push(deadline);
    }
    if let FailedTurn::WaitingContinue { at, .. } = &mut round.failed_turn {
        waits.push(at);
    }
    waits
}

/// The clock a round is charged on: its racing lane's, out of the lane's view
/// (`race_view`), or its task's.
fn round_clock<'a>(task: &'a Task, round: &AgentRound) -> &'a TaskClock {
    let lane = super::race_view::view_lane(task, round.lane);
    let lane = lane.and_then(|l| task.race.as_ref()?.lanes.iter().find(|x| x.lane == l));
    lane.map_or(&task.clock, |l| &l.gates.clock)
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
    let writing: Vec<bool> = (task.rounds.iter())
        .map(|r| crate::run::model::writes(task, r))
        .collect();
    for (r, round) in task.rounds.iter_mut().enumerate() {
        if !writing[r] {
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

/// The spend rung 4 weighs (decision 38): the task's since its last retry. After the
/// crown, the winning lane's only (the final fix wave's m5): its seconds are the
/// crowned racer's rounds (`model::writes`), and the race's cost is left out of its
/// tool calls and tokens ([`own_spend`]).
pub(crate) fn epoch_spend(task: &Task, now: u64) -> Spend {
    let epoch = task.epoch.unwrap_or_default();
    let secs = task
        .rounds
        .iter()
        .enumerate()
        .filter(|(r, round)| *r >= epoch.round && crate::run::model::writes(task, round))
        .map(|(_, round)| round_spend(round, task.clock.stopped, now).secs)
        .sum();
    let own = own_spend(task);
    Spend {
        tool_calls: own.tool_calls.saturating_sub(epoch.tool_calls),
        secs,
        tokens: own.tokens.saturating_sub(epoch.tokens),
    }
}

/// The task's total tool calls and tokens, less a crowned race's cost ([`race_cost`]).
/// In a lane's view the total is the lane's own already.
fn own_spend(task: &Task) -> Spend {
    let crowned = task.lane_view.is_none() && task.race.as_ref().is_some_and(|r| r.crowned);
    let cost = match crowned {
        true => race_cost(task),
        false => Spend::default(),
    };
    Spend {
        tool_calls: task.spent_total.tool_calls.saturating_sub(cost.tool_calls),
        secs: 0,
        tokens: task.spent_total.tokens.saturating_sub(cost.tokens),
    }
}

/// The final fix wave's m5: the race's cost, what every lane but the winner spent
/// (tool calls and tokens). The task's total keeps it; rung 4 does not weigh it once
/// the winner is crowned (see `epoch_spend`), and `REPORT.md`'s race line shows it.
pub(crate) fn race_cost(task: &Task) -> Spend {
    let Some(race) = task.race.as_ref().filter(|r| r.winner.is_some()) else {
        return Spend::default();
    };
    (race.lanes.iter())
        .filter(|l| Some(l.lane) != race.winner)
        .fold(Spend::default(), |sum, l| Spend {
            tool_calls: sum.tool_calls.saturating_add(l.spent.tool_calls),
            secs: 0,
            tokens: sum.tokens.saturating_add(l.spent.tokens),
        })
}

/// `run retry` starts a new epoch at the fresh session's round, from the task's own
/// spend ([`own_spend`]).
pub(super) fn new_epoch(task: &mut Task) {
    let own = own_spend(task);
    task.epoch = Some(BudgetEpoch {
        round: task.rounds.len(),
        tool_calls: own.tool_calls,
        tokens: own.tokens,
    });
}
