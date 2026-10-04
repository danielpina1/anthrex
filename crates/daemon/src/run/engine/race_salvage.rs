//! Milestone 9.5 decision 22 (rulings RR-1, RR-2), task M9.5.17b: a lane that left its
//! race (`Out`, or `Lost` to the winner) is salvaged. Its `RemoveWorktree` waits until
//! every op the lane sent has returned and its racer has exited (`ProcessExited`, ruling
//! RR-2's "finished"), then salvages the checkout with `keep_head` (a clean loser keeps
//! its commits too), clears the stale locks a stopped racer can leave, and removes it.
//! A racer that has not exited `LANE_EXIT_WAIT_SECS` after its stop keeps its checkout:
//! salvage still runs, with `keep_path` and no lock cleared, and the checkout goes with
//! the run's others at accept or discard. No kill code: the stop is `race_end`'s
//! `KillWindow`. Pure (design decision 1).

use proto::{AgentRole, LaneState, RaceLane};

use super::dispatch::{history, salvage_ref};
use super::race_end::LANE_EXIT_WAIT_SECS;
use super::{Effect, OpKind, OpResult, emit_op, merge, next_op, requests};
use crate::run::model::{Lane, Run, Task};

/// A lane that left the race and whose checkout is still to be salvaged.
fn due(lane: &Lane) -> bool {
    matches!(lane.state, LaneState::Out | LaneState::Lost) && !lane.removed && !lane.kept
}

/// The final fix wave's m7: a stopped lane's salvage is still due somewhere in `run`
/// (its racer may still be running in the checkout). The run neither completes nor is
/// discarded meanwhile.
pub(crate) fn pending(run: &Run) -> bool {
    (run.tasks
        .iter()
        .flat_map(|t| t.race.iter().flat_map(|r| &r.lanes)))
    .any(|l| due(l) && l.gates.worktree_live)
}

/// Every scheduler pass, whatever the task's state (a cancelled task's lanes too): each
/// stopped lane whose ops have returned is salvaged and removed once its racer has
/// exited, or salvaged and kept once its wait is over.
pub(super) fn pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        let lanes: Vec<RaceLane> = (task.race.iter().flat_map(|r| &r.lanes))
            .filter(|l| due(l) && l.gates.worktree_live)
            .map(|l| l.lane)
            .filter(|l| !op_in_flight(run, task, *l))
            .collect();
        for lane in lanes {
            salvage(run, i, lane, now, fx);
        }
    }
}

/// An op lane `lane` of `task` sent (its gates, its removal) has not returned.
fn op_in_flight(run: &Run, task: &Task, lane: RaceLane) -> bool {
    (run.pending_ops.values())
        .any(|p| p.task_id.as_deref() == Some(task.id()) && p.lane == Some(lane))
}

/// Why lane `lane`'s checkout must be kept, or `None` when its racer is known to have
/// finished; `Err(())` while it may still exit in time. Ruling T17b-2 (RR-2): a racer has
/// finished only by its `ProcessExited` (a real exit, or the engine's for a window with
/// no process), or when its launch failed. A round `orphaned` by the restart has not:
/// one the restore ended, one whose launch the restart lost (ruling T17b-3, N1: the
/// old daemon may have started it), or one whose resume failed (N2).
fn kept_because(task: &Task, lane: &Lane, now: u64) -> Result<Option<&'static str>, ()> {
    let racers =
        (task.rounds.iter()).filter(|r| r.lane == Some(lane.lane) && r.role == AgentRole::Racer);
    let open = racers.clone().any(|r| !r.ended && r.window_id.is_some());
    let orphaned = racers.clone().any(|r| r.orphaned);
    let waited = now >= lane.kill_sent_at.unwrap_or(now) + LANE_EXIT_WAIT_SECS;
    match (open, orphaned) {
        (true, _) if waited => Ok(Some("its racer did not exit")),
        (true, _) => Err(()),
        (false, true) => Ok(Some("its racer had no process after the restart")),
        (false, false) => Ok(None),
    }
}

fn salvage(run: &mut Run, i: usize, lane: RaceLane, now: u64, fx: &mut Vec<Effect>) {
    let task = &run.tasks[i];
    let id = task.id().to_string();
    let Some(stopped) = (task.race.iter().flat_map(|r| &r.lanes)).find(|l| l.lane == lane) else {
        return;
    };
    let Ok(why_kept) = kept_because(task, stopped, now) else {
        return;
    };
    let path = run.task_path(&stopped.checkout);
    let kept = why_kept.is_some();
    // Task 17b's review, m4: the number is reserved now, from the task's one counter;
    // m3: the lane records the ref only once the salvage has succeeded.
    let seq = merge::reserve_salvage_seq(&mut run.tasks[i]);
    let reference = salvage_ref(run, &id, seq);
    if let Some(stopped) = lane_mut(&mut run.tasks[i], lane) {
        stopped.kept = kept;
    }
    if let Some(why) = why_kept {
        let line = format!("race {id}: kept {} checkout: {why}", lane.label());
        history(run, i, now, line.clone());
        requests::log(run, now, line);
    }
    fx.push(Effect::UnwatchWorktree { root: path.clone() });
    let kind = OpKind::RemoveWorktree {
        root: run.root.clone(),
        path,
        salvage_ref: reference,
        keep_head: true,
        clear_locks: !kept,
        keep_path: kept,
    };
    let op = next_op(run);
    emit_op(run, op, Some(&id), kind, fx);
    if let Some(pending) = run.pending_ops.get_mut(&op) {
        pending.lane = Some(lane);
    }
}

fn lane_mut(task: &mut Task, lane: RaceLane) -> Option<&mut Lane> {
    (task.race.as_mut()).and_then(|r| r.lanes.iter_mut().find(|l| l.lane == lane))
}

/// Whether `RemoveWorktree`'s result for lane `lane` of task `i` is a stopped lane's
/// salvage ([`removed`]), answered outside any view.
pub(super) fn owns(run: &Run, i: usize, lane: Option<RaceLane>) -> bool {
    let task = &run.tasks[i];
    let lanes = task.race.iter().flat_map(|r| &r.lanes);
    lanes
        .filter(|l| Some(l.lane) == lane)
        .any(|l| matches!(l.state, LaneState::Out | LaneState::Lost))
}

/// A stopped lane's salvage result (decision 22): its salvage ref and the locks it
/// cleared land on the lane and in the task's history (each removal recorded `removed a
/// stale <file> left by the stopped racer`); the ref joins the task's. A removal that
/// failed leaves the checkout for accept or discard (kept), never tried in a loop.
pub(super) fn removed(run: &mut Run, i: usize, lane: RaceLane, result: OpResult, now: u64) {
    let label = lane.label();
    let Some(stopped) = lane_mut(&mut run.tasks[i], lane) else {
        return;
    };
    match result {
        OpResult::Removed {
            salvage_ref,
            cleared_locks,
        } => {
            stopped.salvage_ref = salvage_ref.clone();
            stopped.cleared_locks = cleared_locks.clone();
            let kept = stopped.kept;
            if !kept {
                stopped.removed = true;
                stopped.gates.worktree_live = false;
            }
            for lock in cleared_locks {
                let text =
                    format!("racer {label}: removed a stale {lock} left by the stopped racer");
                history(run, i, now, text);
            }
            let what = match kept {
                true => "checkout kept",
                false => "checkout removed",
            };
            let text = match salvage_ref {
                Some(reference) => {
                    run.tasks[i].salvage_refs.push(reference.clone());
                    format!("racer {label}: {what}; work salvaged at {reference}")
                }
                None => format!("racer {label}: {what}"),
            };
            history(run, i, now, text);
        }
        OpResult::Failed { message } => {
            stopped.kept = true;
            stopped.salvage_ref = None;
            let text = format!("racer {label}: could not salvage its checkout: {message}");
            history(run, i, now, text);
        }
        _ => {}
    }
}
