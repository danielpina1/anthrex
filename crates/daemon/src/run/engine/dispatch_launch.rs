//! The worker launches of `dispatch.rs`, split out of it (milestone 9.3 task 4b, a
//! move-only split): the plan gate's pre-warm (decision 14), writer dispatch into task
//! worktrees (decision 19), the launch of a worktree prepared while the run was not
//! running (ruling T14-I2). The worker sessions themselves are `dispatch_session.rs`'s
//! (decisions 24–26, 30). Pure (design decision 2).

use crate::run::phases::set_state;

use proto::TaskState;

use super::history;
use super::session::launch_worker;
use crate::run::engine::brakes::Gate;
use crate::run::engine::failover::{self, Admit, Starter};
use crate::run::engine::race::Start;
use crate::run::engine::schedule::{
    dispatch_order, held_hub_waits_for, hub_started, is_reader_task, may_return_to_working,
    op_in_flight, size_check_pending, writers_busy,
};
use crate::run::engine::{Effect, OpKind, concurrency, emit_op, gate_holds, next_op, pair};
use crate::run::env::profile_env;
use crate::run::model::Run;
use crate::run::model_roles::RunModels;

fn prepare_in_flight(run: &Run, i: usize) -> bool {
    op_in_flight(run, run.tasks[i].id(), |k| {
        matches!(k, OpKind::PrepareWorktree { .. })
    })
}

/// A `PrepareWorktree` for task `i` from `from`, with the profile's `setup`. A sync
/// task not yet started needs its merge handed back again after it.
pub(super) fn prepare(run: &mut Run, i: usize, from: String, fx: &mut Vec<Effect>) {
    let started = run.tasks[i].start_commit.is_some();
    if let Some(sync) = run.tasks[i].sync.as_mut() {
        sync.handed_back &= started;
    }
    let op = next_op(run);
    let task = &run.tasks[i];
    let kind = OpKind::PrepareWorktree {
        root: run.root.clone(),
        branch: task.branch.clone(),
        from,
        path: task.worktree.clone(),
        setup: run.profile.setup.clone(),
        env: profile_env(&run.profile, &task.worktree),
    };
    let id = task.id().to_string();
    emit_op(run, op, Some(&id), kind, fx);
}

/// Decision 14's pre-warm: while the gate is open, worktrees (with `setup`) for up to
/// `max_writers` tasks with neither declared nor implicit dependencies, in dispatch
/// order, from `base_sha`. No session starts.
pub(super) fn prewarm(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    // A pre-warm place is held by a pending pre-warm, or by a pre-warmed task that is
    // still a root; one that has gained a dependency releases it (review minor 2).
    let mut held = (0..run.tasks.len())
        .filter(|&i| {
            let t = &run.tasks[i];
            let root = t.state == TaskState::Queued
                && t.spec.deps.is_empty()
                && t.implicit_deps.is_empty();
            !t.state.is_finished() && ((t.prewarmed && root) || prepare_in_flight(run, i))
        })
        .count();
    for i in dispatch_order(run) {
        if held >= usize::from(run.limits.max_writers) {
            break;
        }
        let task = &run.tasks[i];
        // Milestone 9: a research or review task has no worktree to pre-warm.
        if task.state != TaskState::Queued
            || is_reader_task(task)
            || !gate_holds::released(run, task)
            || task.prewarmed
            || task.worktree_live
            || !task.spec.deps.is_empty()
            || !task.implicit_deps.is_empty()
            || prepare_in_flight(run, i)
            // Milestone 9.5 decision 18: a racing task's checkouts are its lanes'.
            || task.spec.race
        {
            continue;
        }
        let base = run.base_sha.clone();
        prepare(run, i, base, fx);
        history(run, i, now, "pre-warming its worktree");
        held += 1;
    }
}

/// Writer dispatch in critical-path order while writer slots are free (decision 41).
/// Milestone 9.16 decision 11: a task whose runtime `gate` refuses stays `Queued`.
pub(super) fn dispatch_writers(run: &mut Run, now: u64, gate: &mut Gate, fx: &mut Vec<Effect>) {
    // F3 review N1: a held hub task lets only what it waits for start.
    let only = held_hub_waits_for(run);
    // Milestone 9.5 decision 18: a race waiting for its second slot at the head of the
    // line holds it.
    let mut first = true;
    for i in dispatch_order(run) {
        // M8b decision 19: a task waiting for its size cross-check is not runnable;
        // milestone 9 decision 28: nor is one whose approval hold is not approved.
        // Milestone 9 decisions 35, 36: research and review tasks take reader slots.
        if run.tasks[i].state != TaskState::Queued
            || is_reader_task(&run.tasks[i])
            || size_check_pending(&run.tasks[i])
            || !gate_holds::released(run, &run.tasks[i])
            || only
                .as_ref()
                .is_some_and(|waits| !waits.iter().any(|id| id == run.tasks[i].id()))
        {
            continue;
        }
        let busy = writers_busy(run);
        if busy >= usize::from(run.limits.max_writers) || hub_started(run) {
            break;
        }
        // Milestone 9.16 ruling C5: before the race is decided, so a refused task
        // latches no `race_decision`; decisions 36–41: a quota-braked one may start on
        // its row's fallback instead.
        let runtime = pair::dispatch_runtime(run, i);
        let (starter, failover) = match writer_admit(run, i, gate) {
            (_, Admit::Wait) => continue,
            (starter, Admit::Go) => (starter, None),
            (starter, Admit::FailOver(f)) => (starter, Some(f)),
        };
        let runtime = failover.as_ref().map_or(runtime, |f| f.to.runtime);
        let head_of_line = std::mem::replace(&mut first, false);
        match super::super::race::start(run, i, now, gate) {
            Start::Single => {}
            Start::Wait if head_of_line => break,
            Start::Wait => continue,
            Start::Race(peer) => {
                gate.started(run, runtime, now);
                gate.started(run, peer.runtime, now);
                history(run, i, now, "dispatched");
                super::super::race::dispatch_race(run, i, peer, now, fx, prepare);
                continue;
            }
        }
        // Milestone 9.5 decision 16: a task whose runtime is at its cap is skipped; a
        // paired task's is its test writer's (the final fix wave's A-I3); a failover's
        // the fallback's (milestone 9.16 decision 37 (h)).
        if !concurrency::has_room(run, runtime) {
            continue;
        }
        if run.tasks[i].hub
            && (busy > 0 || run.tasks.iter().any(|t| may_return_to_working(t.state)))
        {
            continue;
        }
        gate.started(run, runtime, now);
        set_state(&mut run.tasks[i], TaskState::Preparing, now);
        // Milestone 9.5 decision 25: a paired task starts with its test writer.
        pair::begin(run, i);
        if let Some(f) = failover {
            failover::record(run, starter, &f, now);
        }
        history(run, i, now, "dispatched");
        if prepare_in_flight(run, i) {
            // A pre-warm still running: its result continues the dispatch.
            continue;
        }
        let task = &run.tasks[i];
        // Milestone 9.1 decision 47: the task's stage head (decision 51: a sync task's
        // conflict head).
        let head = super::super::propagate::start_of(run, task);
        if task.prewarmed && task.start_commit.is_none() && run.base_sha == head {
            launch_worker(run, i, head, now, fx);
        } else {
            // Decision 19: from the run head. A pre-warmed branch that is now stale is
            // re-pointed by the git layer, and its setup runs again (ruling T8-I4).
            prepare(run, i, head, fx);
        }
    }
}

/// Ruling T14-I2: a task whose worktree was prepared while the run was not running is
/// launched now, from that commit when it is still the run head, else re-pointed.
/// Milestone 9.16 decision 11: on a runtime `gate` refuses it keeps its `ready_from`.
pub(super) fn launch_ready(run: &mut Run, now: u64, gate: &mut Gate, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        if run.tasks[i].state != TaskState::Preparing || prepare_in_flight(run, i) {
            continue;
        }
        if run.tasks[i].ready_from.is_none() {
            continue;
        }
        let (starter, failover) = match writer_admit(run, i, gate) {
            (_, Admit::Wait) => continue,
            (starter, Admit::Go) => (starter, None),
            (starter, Admit::FailOver(f)) => (starter, Some(f)),
        };
        let runtime = match &failover {
            // Decision 37 (h): the fallback's runtime has room.
            Some(f) if !concurrency::has_room(run, f.to.runtime) => continue,
            Some(f) => f.to.runtime,
            None => pair::dispatch_runtime(run, i),
        };
        gate.started(run, runtime, now);
        if let Some(f) = failover {
            failover::record(run, starter, &f, now);
        }
        let Some(from) = run.tasks[i].ready_from.take() else {
            continue;
        };
        let head = super::super::propagate::start_of(run, &run.tasks[i]);
        if from == head {
            launch_worker(run, i, from, now, fx);
        } else {
            prepare(run, i, head, fx);
        }
    }
}

/// Milestone 9.16 decisions 36–38: the gate's answer for task `i`'s first session at
/// dispatch, its test writer on the `test_writer` row or its worker on its own row. A
/// racing task never fails over (decision 38): lane 2 is the row's fallback already.
fn writer_admit(run: &Run, i: usize, gate: &Gate) -> (Starter<'static>, Admit) {
    let (writer, current) = pair::dispatch_route(run, i);
    let (starter, role) = match writer {
        true => (Starter::TestWriter(i), proto::models::Role::TestWriter),
        false => (Starter::Worker(i), RunModels::task_role(&run.tasks[i])),
    };
    let admit = match failover::admit(gate, run, starter, role, &current) {
        Admit::FailOver(_) if run.tasks[i].spec.race => Admit::Wait,
        admit => admit,
    };
    (starter, admit)
}
