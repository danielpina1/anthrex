//! The worker launches of `dispatch.rs`, split out of it (milestone 9.3 task 4b, a
//! move-only split): the plan gate's pre-warm (decision 14), writer dispatch into task
//! worktrees (decision 19), the launch of a worktree prepared while the run was not
//! running (ruling T14-I2), and worker sessions, first or fresh (decisions 24–26, 30).
//! Pure (design decision 2).

use crate::run::phases::set_state;

use proto::{AgentRole, Runtime, TaskState};

use super::{history, new_round, window_limit_reached};
use crate::run::contract::{handover_prompt, worker_prompt};
use crate::run::engine::schedule::{
    dispatch_order, held_hub_waits_for, hub_started, is_reader_task, may_return_to_working,
    op_in_flight, size_check_pending, writers_busy,
};
use crate::run::engine::{Effect, OpKind, done, emit_op, gate_holds, ladder, next_op};
use crate::run::env::profile_env;
use crate::run::model::{FreshSession, Run, Task};
use crate::run::orch::contract::notes_section;
use crate::run::role_launch::{jitter_ms, session_uuid_of, worker_spec};

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
pub(super) fn dispatch_writers(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    // F3 review N1: a held hub task lets only what it waits for start.
    let only = held_hub_waits_for(run);
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
        if run.tasks[i].hub
            && (busy > 0 || run.tasks.iter().any(|t| may_return_to_working(t.state)))
        {
            continue;
        }
        set_state(&mut run.tasks[i], TaskState::Preparing, now);
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
pub(super) fn launch_ready(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        if run.tasks[i].state != TaskState::Preparing || prepare_in_flight(run, i) {
            continue;
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

/// A new worker session for task `i`, starting at `start` (decisions 24–26, 30); a
/// sync task's merge is handed back into its worktree first (milestone 9.1 decision 51).
pub(in crate::run::engine) fn launch_worker(
    run: &mut Run,
    i: usize,
    start: String,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if super::super::propagate::hand_back_first(run, i, &start, now, fx) {
        return;
    }
    let prompt =
        |run: &Run, task: &Task| worker_prompt(run, task, "", &notes_section(&task.orch.messages));
    launch(run, i, Some(start), prompt, now, fx);
}

/// A fresh worker session for a started task (rung 2, or a resume that failed): its
/// first turn is decision 30's hand-over prompt, ending with the messages a failed
/// resume carried (decision 29).
pub(in crate::run::engine) fn launch_fresh(
    run: &mut Run,
    i: usize,
    fresh: &FreshSession,
    stat: &str,
    patch: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let prompt = |run: &Run, task: &Task| {
        let mut text = handover_prompt(
            run,
            task,
            &fresh.reason,
            stat,
            patch,
            "",
            &notes_section(&task.orch.messages),
        );
        if let Some(append) = &fresh.append {
            text.push_str("\n\n");
            text.push_str(append);
        }
        text
    };
    launch(run, i, None, prompt, now, fx);
}

/// Session `n + 1` of task `i`, its first turn built once the session number is known;
/// `start` is set on the first (a task blocked by the window limit has not started).
fn launch(
    run: &mut Run,
    i: usize,
    start: Option<String>,
    first_turn: impl FnOnce(&Run, &Task) -> String,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if window_limit_reached(run, i, now) {
        return;
    }
    // Ruling T12-I1: a new round ends any claim of an earlier one, and (ruling T12-N)
    // every op the earlier ones awaited.
    done::drop_claim(
        run,
        i,
        "this session was replaced; its task_done no longer applies",
        fx,
    );
    ladder::supersede(run, i);
    let op = next_op(run);
    if let Some(start) = start {
        run.tasks[i].start_commit = Some(start);
    }
    run.tasks[i].session += 1;
    // M8b decision 33a: the route is fixed; decided before the session-start op.
    crate::run::routing::record_worker(run, i, now);
    let task = &run.tasks[i];
    let spec = worker_spec(run, task);
    let first_turn = first_turn(run, task);
    // Milestone 9 decision 42d: the first turn carries every recorded message.
    super::super::worker_messages::launched(run, i);
    let task = &run.tasks[i];
    let extract = crate::run::orch::extract::worker_slot(run, task);
    let name = format!("{}/{}.w{}", run.short(), task.id(), task.session);
    let uuid = (task.route.runtime == Runtime::Claude).then(|| session_uuid_of(run, op));
    let jitter = jitter_ms(&run.id, task.id(), task.session);
    let round = new_round(
        AgentRole::Worker,
        task.session,
        task.route.clone(),
        op,
        uuid.clone(),
        now,
    );
    let (id, worktree, session) = (task.id().to_string(), task.worktree.clone(), task.session);
    run.tasks[i].rounds.push(round);
    run.windows_created += 1;
    history(run, i, now, format!("worker session {session} starting"));
    let kind = OpKind::CreateWindow {
        name,
        spec: Box::new(spec),
        session_uuid: uuid,
        first_turn,
        project: run.project.clone(),
        worktree,
        jitter_ms: jitter,
        extract,
    };
    emit_op(run, op, Some(&id), kind, fx);
}
