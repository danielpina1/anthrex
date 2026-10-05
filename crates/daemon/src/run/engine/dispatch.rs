//! What the scheduler starts, and the results of the ops it emits: the plan gate's
//! pre-warm (decision 14), writer dispatch into task worktrees (decision 19), reviewer
//! dispatch into reader slots (decision 41), worker and reviewer windows (decisions
//! 24–26, 49), the window limit (decision 16), the resume of a started task that waited
//! for new dependencies (M8a.6 ruling N5), and the clean-up of a cancelled task's
//! worktree when no session is left (M8a.6's F5). Pure (design decision 2).

use crate::run::phases::set_state;
use std::path::Path;

use proto::{AgentRole, BlockInfo, BlockReason, RunState, TaskState};

use super::schedule::{deps_done, op_in_flight};
use super::{Effect, OpKind, OpResult, emit_op, next_op};
use super::{
    clock, complete, deciders, gates, holds, kinds, ladder, merge, outbox, restore, review,
    signals, stages,
};
use crate::run::contract::is_stall_nudge;
use crate::run::model::{OpId, Run, StallState, TaskEvent};

pub(super) use super::rounds::new_round;

// The worker launches, split out (milestone 9.3 task 4b, move-only).
#[path = "dispatch_launch.rs"]
mod launch;
use launch::{dispatch_writers, launch_ready, prepare, prewarm};
// The worker sessions, split out of `dispatch_launch.rs` (task M9.5.17a, move-only).
#[path = "dispatch_session.rs"]
mod session;
pub(super) use session::{launch_fresh, launch_implementer, launch_worker};

/// The scheduler, run after every event: runnability, then whatever the run's state
/// allows to start, then clean-up and delivery.
pub(super) fn schedule(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if run.state.is_terminal() || finishing(run) {
        return;
    }
    // Ruling T15-18: a round's due documents commit with no task of the round left.
    super::design_commit::abandon(run, now);
    clock::watch_open_turns(run, now, fx);
    super::race::each_lane(run, now, fx, |run, fx| {
        clock::watch_open_turns(run, now, fx)
    });
    holds::enforce_holds(run, now, fx);
    requeue(run, now);
    if integration_ready(run) {
        // Milestone 9.6 decision 23: nothing branches from the run head before the
        // documents commit lands (Review focus 3); ruling T1-O2: nor is it pre-warmed.
        let held = super::design_commit::hold(run, now, fx);
        match run.state {
            RunState::AwaitingApproval if super::design_commit::skips_prewarm(run) => {}
            RunState::AwaitingApproval => prewarm(run, now, fx),
            RunState::Running if held => {}
            RunState::Running => {
                restore::relaunch(run, now, fx);
                holds::resume_held(run, now, fx);
                signals::watch(run, now, fx);
                ladder::recover_sessionless(run, now);
                ladder::start_fresh_sessions(run, fx);
                complete::finish_pass(run, now, fx);
                // Milestone 9.1 decision 48: stage branches before what runs in them.
                stages::create_pass(run, now, fx);
                // Milestone 9 decision 37: an epic merged gets its integration review.
                kinds::integration_pass(run, now);
                kinds::watch(run, now, fx);
                gates::start_gates(run, now, fx);
                merge::start_due_hand_backs(run, now, fx);
                merge::start_merge(run, now, fx);
                // Milestone 9.2 decisions 19-20: stage pull requests, before 17(b).
                super::delivery::pass(run, now, fx);
                // Milestone 9.1 decision 17(b): tier 3 when the queue is idle.
                super::full::idle_pass(run, now, fx);
                // Decision 36: a lost or backed-off bisect probe (task M9.1.15).
                super::bisect::pass(run, now, fx);
                // Ruling C-27 (4): a bisect fix task that ended without merging.
                super::full::fix_ended_pass(run);
                review::watch(run, now, fx);
                // Milestone 9.5 ruling T16-3: a confirmed red's implementer.
                super::pair::launch_due(run, now, fx);
                launch_ready(run, now, fx);
                dispatch_writers(run, now, fx);
                // M8b decision 18: queued deciders take free reader slots first.
                fx.extend(deciders::dispatch(run, now));
                review::dispatch_reviewers(run, fx);
                // Milestone 9.5 decision 20: each lane of a race, in its lane's view.
                lane_passes(run, now, fx);
                super::race_end::crown_pass(run, now, fx);
                // Milestone 9 decision 31: integration reviews go with the reviewers.
                kinds::dispatch(run, now, true, fx);
            }
            _ => {}
        }
        // Milestone 9 decision 31: sub-planners, then run scouts, in free reader slots.
        super::planners::dispatch(run, now, fx);
        // Then research and review tasks (decisions 35, 36), never before a documents
        // commit lands (a later round's too, which holds no pass: task M9.6.15).
        if !held && !super::design_commit::due(run) {
            kinds::dispatch(run, now, false, fx);
        }
    }
    remove_cancelled_worktrees(run, now, fx);
    // Milestone 9.5 decision 22: a lane that left its race, once its racer has exited.
    super::race_salvage::pass(run, now, fx);
    if run.state == RunState::Running {
        super::worker_messages::refresh_pass(run, now, fx);
        // Ruling RR-4: a racing task's refresh, in each live lane.
        super::race::each_lane(run, now, fx, |run, fx| {
            super::worker_messages::refresh_pass(run, now, fx)
        });
        outbox::deliver(run, now, fx);
        super::race::each_lane(run, now, fx, |run, fx| outbox::deliver(run, now, fx));
        complete::complete_pass(run, now, fx);
    }
    // Rulings T15-I2, T15-I3, T15-R3: the task clocks, after the pass's changes. A
    // stop still open is subtracted wherever spend is read (`ladder::round_spend`).
    clock::sync(run, now);
    super::race::each_lane(run, now, fx, |run, _| clock::sync(run, now));
}

/// Milestone 9.5 decision 20 (ruling RR-3): the scheduler passes a race's lanes take,
/// each in its lane's view, so each lane relaunches, is watched, and runs every
/// pre-merge gate on its own. Each pass is idempotent at one `now`, so the tasks that do
/// not race, which each view shows as they are, see nothing new.
fn lane_passes(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    super::race::each_lane(run, now, fx, |run, fx| {
        restore::relaunch(run, now, fx);
        signals::watch(run, now, fx);
        ladder::recover_sessionless(run, now);
        ladder::start_fresh_sessions(run, fx);
        gates::start_gates(run, now, fx);
        review::watch(run, now, fx);
        launch_ready(run, now, fx);
        review::dispatch_reviewers(run, fx);
    });
}

/// A `Discard` or `Accept` in flight, named as a reply says it (`discarded`,
/// `accepted`): nothing more starts, and the gate's requests are refused.
pub(super) fn finishing_as(run: &Run) -> Option<&'static str> {
    run.pending_ops.values().find_map(|p| match p.kind {
        OpKind::Discard { .. } => Some("discarded"),
        OpKind::Accept { .. } => Some("accepted"),
        _ => None,
    })
}

fn finishing(run: &Run) -> bool {
    finishing_as(run).is_some()
}

/// The integration worktree exists (its `CreateRunBranch` has come back).
fn integration_ready(run: &Run) -> bool {
    !run.pending_ops
        .values()
        .any(|p| matches!(p.kind, OpKind::CreateRunBranch { .. }))
}

/// Decision 31: `queued` is runnable, `pending` waits for dependencies.
fn requeue(run: &mut Run, now: u64) {
    for i in 0..run.tasks.len() {
        let state = run.tasks[i].state;
        if !matches!(state, TaskState::Pending | TaskState::Queued) {
            continue;
        }
        // Milestone 9.1 decision 48: and its stage holds what it needs.
        let next = if deps_done(run, &run.tasks[i]) && stages::ready_in_stage(run, i) {
            TaskState::Queued
        } else {
            TaskState::Pending
        };
        set_state(&mut run.tasks[i], next, now);
    }
}

pub(super) fn history(run: &mut Run, i: usize, now: u64, text: impl Into<String>) {
    run.tasks[i].history.push(TaskEvent {
        at: now,
        text: text.into(),
    });
}

pub(super) fn block(run: &mut Run, i: usize, reason: BlockReason, text: String, now: u64) {
    let label = serde_json::to_value(reason)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    history(run, i, now, format!("blocked ({label}): {text}"));
    let task = &mut run.tasks[i];
    set_state(task, TaskState::Blocked, now);
    task.block = Some(BlockInfo { reason, text });
    // Ruling T12-I4b: a pending interrupt no longer applies to a blocked task; its
    // stale nudge is dropped, so whatever unblocks the task is delivered.
    let mut cleared = false;
    for round in task.rounds.iter_mut() {
        if matches!(round.stall, StallState::Interrupted { .. }) {
            round.stall = StallState::Watching;
            cleared = true;
        }
    }
    if cleared {
        let id = task.spec.id.clone();
        run.outbox
            .retain(|m| m.task_id != id || m.delivered_at.is_some() || !is_stall_nudge(&m.text));
    }
}

/// Decision 16: every run window counts toward `max_windows`; a task that would pass it
/// is `blocked(environment)`. Milestone 9.3 decision 14: counted from the round's start.
pub(super) fn window_limit_reached(run: &mut Run, i: usize, now: u64) -> bool {
    let before = run.current_round().map_or(0, |r| r.windows_before);
    if run.windows_created.saturating_sub(before) < run.limits.max_windows {
        return false;
    }
    // Milestone 9.5 decision 46: a later round's text names the round.
    let limit = run.limits.max_windows;
    let text = match run.round() {
        r if r > 1 => format!("round {r}'s window limit ({limit}) reached"),
        _ => format!("run window limit ({limit}) reached"),
    };
    block(run, i, BlockReason::Environment, text, now);
    true
}

/// M8a.6's F5 and decision 14: a cancelled task whose worktree still exists and that has
/// no live session and no op in flight has its work salvaged and its worktree removed.
fn remove_cancelled_worktrees(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        if task.state != TaskState::Cancelled
            || !task.worktree_live
            || task.rounds.iter().any(|r| !r.ended)
            || op_in_flight(run, task.id(), |_| true)
        {
            continue;
        }
        // Review m4: one numbering rule for every salvage (decision 20's `<seq>`),
        // reserved as the removal is sent (task 17b's review, m4).
        let (id, path) = (task.id().to_string(), task.worktree.clone());
        let seq = merge::reserve_salvage_seq(&mut run.tasks[i]);
        let salvage_ref = salvage_ref(run, &id, seq);
        fx.push(Effect::UnwatchWorktree { root: path.clone() });
        let op = next_op(run);
        let kind = OpKind::RemoveWorktree {
            root: run.root.clone(),
            path,
            salvage_ref,
            keep_head: false,
            clear_locks: false,
            keep_path: false,
        };
        emit_op(run, op, Some(&id), kind, fx);
        history(run, i, now, "removing its worktree (salvaged if dirty)");
    }
}

/// Decision 20: `refs/anthrex/salvage/<run>/<task>/<seq>`.
pub(super) fn salvage_ref(run: &Run, task: &str, seq: usize) -> String {
    format!("refs/anthrex/salvage/{}/{task}/{seq}", run.id)
}

/// The result of a task's `PrepareWorktree`.
pub(super) fn worktree_done(
    run: &mut Run,
    i: usize,
    from: String,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let created = matches!(
        result,
        OpResult::Worktree { .. } | OpResult::SetupFailed { .. }
    );
    if created && !run.tasks[i].worktree_live {
        run.tasks[i].worktree_live = true;
        fx.push(Effect::WatchWorktree {
            root: run.tasks[i].worktree.clone(),
        });
    }
    let state = run.tasks[i].state;
    match result {
        // Ruling T14-I2: while the run is not running (halted), the worktree is
        // recorded as ready and the first running pass carries on the dispatch.
        OpResult::Worktree { .. }
            if state == TaskState::Preparing && run.state != RunState::Running =>
        {
            run.tasks[i].ready_from = Some(from);
            history(
                run,
                i,
                now,
                "worktree ready; the worker starts once the run runs",
            );
        }
        OpResult::Worktree { .. } if state == TaskState::Preparing => {
            let head = super::propagate::start_of(run, &run.tasks[i]);
            if from == head {
                launch_worker(run, i, from, now, fx);
            } else {
                prepare(run, i, head, fx);
            }
        }
        OpResult::Worktree { .. } => {
            if matches!(state, TaskState::Pending | TaskState::Queued) && from == run.base_sha {
                run.tasks[i].prewarmed = true;
                history(run, i, now, "worktree pre-warmed");
            }
        }
        // N4 (M8a.6 fix round 2): a task blocked in setup has no start commit, so it
        // does not count as started; no agent has written in its worktree.
        OpResult::SetupFailed { output } if !state.is_finished() => {
            run.tasks[i].prewarmed = false;
            let text = format!("setup failed:\n{output}");
            block(run, i, BlockReason::Environment, text, now);
        }
        OpResult::Failed { message } if !state.is_finished() => {
            // Ruling FW-4: the task branch may have been made before the step that
            // failed; a lane's own failure (in its view) is the race's, not the task's.
            if run.tasks[i].race.is_none() {
                run.tasks[i].prepare_failed = true;
            }
            let text = format!("could not prepare the worktree: {message}");
            block(run, i, BlockReason::Environment, text, now);
        }
        _ => {}
    }
}

/// The result of a `CreateWindow` (worker or reviewer).
pub(super) fn window_done(
    run: &mut Run,
    i: usize,
    op: OpId,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(r) = run.tasks[i].rounds.iter().position(|r| r.launch_op == op) else {
        return;
    };
    let state = run.tasks[i].state;
    // M8a.13: a reviewer whose task left `review` (overridden) or whose round was given
    // up before its window came is not wanted.
    let round = &run.tasks[i].rounds[r];
    let stale_reviewer =
        round.role == AgentRole::Reviewer && (state != TaskState::Review || round.retiring);
    match result {
        OpResult::Window { window_id, pid } => {
            let round = &mut run.tasks[i].rounds[r];
            round.window_id = Some(window_id);
            // M8a.25: the first process's `ProcessStarted` came before the window.
            if round.pid.is_none() {
                round.pid = pid;
            }
            if state.is_finished() || stale_reviewer {
                round.retiring = true;
                fx.push(Effect::KillWindow { window_id });
            } else if state == TaskState::Preparing
                && crate::run::model::writes(&run.tasks[i], &run.tasks[i].rounds[r])
            {
                set_state(&mut run.tasks[i], TaskState::Working, now);
            }
        }
        OpResult::Failed { message } => {
            let round = &mut run.tasks[i].rounds[r];
            round.ended = true;
            round.turn_open = false;
            round.ended_at = Some(now);
            if !state.is_finished() && !stale_reviewer {
                let text = format!("could not start the session: {message}");
                block(run, i, BlockReason::Environment, text, now);
            }
        }
        _ => {}
    }
}

/// The result of a `RemoveWorktree`.
pub(super) fn removed(run: &mut Run, i: usize, path: &Path, result: OpResult, now: u64) {
    match result {
        OpResult::Removed { salvage_ref, .. } => {
            let task = &mut run.tasks[i];
            // M8a.14: a merged task's review and proof worktrees are removed too.
            if path == task.worktree {
                task.worktree_live = false;
                task.prewarmed = false;
            }
            if let Some(reference) = salvage_ref {
                task.salvage_refs.push(reference.clone());
                history(
                    run,
                    i,
                    now,
                    format!("worktree removed; work salvaged at {reference}"),
                );
            } else {
                history(run, i, now, "worktree removed");
            }
        }
        OpResult::Failed { message } => {
            history(
                run,
                i,
                now,
                format!("could not remove its worktree: {message}"),
            );
        }
        _ => {}
    }
}
