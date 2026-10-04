//! The worker sessions of `dispatch_launch.rs`, split out of it (milestone 9.5 task
//! M9.5.17a, a move-only split): a task's first worker session, a paired task's
//! implementer, a fresh session at rung 2, and the launch they share. Pure (design
//! decision 2).

use proto::{AgentRole, Runtime};

use super::{history, new_round, window_limit_reached};
use crate::run::contract::{handover_prompt, worker_prompt};
use crate::run::contract_patterns::{test_writer_handover, test_writer_prompt};
use crate::run::engine::{Effect, OpKind, done, emit_op, ladder, next_op, pair};
use crate::run::model::{FreshSession, Run, Task};
use crate::run::orch::contract::notes_section;
use crate::run::role_launch::{jitter_ms, session_uuid_of, worker_spec};
use crate::run::role_launch_patterns::test_writer_spec;

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
    // Milestone 9.5 decision 25: a paired task's first session is its test writer's.
    if pair::writing(&run.tasks[i]) {
        return launch(run, i, Some(start), test_writer_prompt, now, fx);
    }
    let prompt =
        |run: &Run, task: &Task| worker_prompt(run, task, "", &notes_section(&task.orch.messages));
    launch(run, i, Some(start), prompt, now, fx);
}

/// Decision 26: a paired task's implementer, once its red is confirmed, in the same
/// checkout; its first turn is the worker prompt with the pair's note before the brief.
pub(in crate::run::engine) fn launch_implementer(
    run: &mut Run,
    i: usize,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let prompt =
        |run: &Run, task: &Task| worker_prompt(run, task, "", &notes_section(&task.orch.messages));
    launch(run, i, None, prompt, now, fx);
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
        // Milestone 9.5 decision 25: a fresh test writer while the test is being written.
        let mut text = match pair::writing(task) {
            true => test_writer_handover(run, task, &fresh.reason, stat, patch),
            false => handover_prompt(
                run,
                task,
                &fresh.reason,
                stat,
                patch,
                "",
                &notes_section(&task.orch.messages),
            ),
        };
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
/// Milestone 9.5 decision 25: a paired task's session while its test is being written is
/// a test writer (`<task>.t<n>`, on the writer's route, with no scout extract and its
/// messages left for the implementer's notes); every other is a worker.
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
    let (role, route) = pair::next_session(&run.tasks[i]);
    let writer = role == AgentRole::TestWriter;
    // M8b decision 33a: the route is fixed; decided before the session-start op.
    match writer {
        true => {
            pair::writer_launched(run, i);
            crate::run::routing::record_test_writer(run, i, &route, now);
        }
        false => crate::run::routing::record_worker(run, i, now),
    }
    let task = &run.tasks[i];
    let spec = match writer {
        true => test_writer_spec(run, task, &route),
        false => worker_spec(run, task),
    };
    let first_turn = first_turn(run, task);
    // Milestone 9 decision 42d: the first turn carries every recorded message.
    if !writer {
        super::super::worker_messages::launched(run, i);
    }
    let task = &run.tasks[i];
    let extract = (!writer)
        .then(|| crate::run::orch::extract::worker_slot(run, task))
        .flatten();
    let letter = if writer { "t" } else { "w" };
    let name = format!("{}/{}.{letter}{}", run.short(), task.id(), task.session);
    let uuid = (route.runtime == Runtime::Claude).then(|| session_uuid_of(run, op));
    let jitter = jitter_ms(&run.id, task.id(), task.session);
    let round = new_round(role, task.session, route, op, uuid.clone(), now);
    let (id, worktree, session) = (task.id().to_string(), task.worktree.clone(), task.session);
    run.tasks[i].rounds.push(round);
    run.windows_created += 1;
    let who = if writer { "test writer" } else { "worker" };
    history(run, i, now, format!("{who} session {session} starting"));
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
