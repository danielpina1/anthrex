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

fn salvage(run: &mut Run, i: usize, lane: RaceLane, now: u64, fx: &mut Vec<Effect>) {
    let task = &run.tasks[i];
    let id = task.id().to_string();
    let exited = !(task.rounds.iter())
        .any(|r| r.lane == Some(lane) && r.role == AgentRole::Racer && !r.ended);
    let Some(stopped) = (task.race.iter().flat_map(|r| &r.lanes)).find(|l| l.lane == lane) else {
        return;
    };
    let waited = now >= stopped.kill_sent_at.unwrap_or(now) + LANE_EXIT_WAIT_SECS;
    if !exited && !waited {
        return;
    }
    let path = run.task_path(&stopped.checkout);
    let reference = salvage_ref(run, &id, merge::next_salvage_seq(task));
    let kept = !exited;
    if let Some(stopped) = lane_mut(&mut run.tasks[i], lane) {
        stopped.exited = exited;
        stopped.kept = kept;
        // Reserved now, so the next salvage of the task takes the next number.
        stopped.salvage_ref = Some(reference.clone());
    }
    if kept {
        let line = format!(
            "race {id}: kept {} checkout: its racer did not exit",
            lane.label()
        );
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
            let text = format!("racer {label}: could not salvage its checkout: {message}");
            history(run, i, now, text);
        }
        _ => {}
    }
}
