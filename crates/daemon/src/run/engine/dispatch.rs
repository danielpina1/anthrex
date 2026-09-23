//! What the scheduler starts, and the results of the ops it emits: the plan gate's
//! pre-warm (decision 14), writer dispatch into task worktrees (decision 19), reviewer
//! dispatch into reader slots (decision 41), worker and reviewer windows (decisions
//! 24–26, 49), the window limit (decision 16), the resume of a started task that waited
//! for new dependencies (M8a.6 ruling N5), and the clean-up of a cancelled task's
//! worktree when no session is left (M8a.6's F5). Pure (design decision 2).

use proto::{AgentRole, BlockInfo, BlockReason, RunState, Runtime, TaskState};

use super::outbox;
use super::schedule::{
    deps_done, dispatch_order, hub_holds_slot, needs_reviewer, op_in_flight, readers_busy,
    writers_busy,
};
use super::{Effect, OpKind, OpResult, emit_op, next_op};
use crate::run::contract::{conflict_message, reviewer_prompt, worker_prompt};
use crate::run::env::profile_env;
use crate::run::model::{AgentRound, OpId, Run, TaskEvent};
use crate::run::role_launch::{jitter_ms, reviewer_spec, session_uuid, worker_spec};
use crate::run::roster::pick_reviewer;

/// The scheduler, run after every event: runnability, then whatever the run's state
/// allows to start, then clean-up and delivery.
pub(super) fn schedule(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if run.state.is_terminal() || finishing(run) {
        return;
    }
    requeue(run);
    if integration_ready(run) {
        match run.state {
            RunState::AwaitingApproval => prewarm(run, now, fx),
            RunState::Running => {
                resume_held(run, fx);
                dispatch_writers(run, now, fx);
                dispatch_reviewers(run, fx);
            }
            _ => {}
        }
    }
    remove_cancelled_worktrees(run, now, fx);
    if run.state == RunState::Running {
        outbox::deliver(run, now, fx);
    }
}

/// A `Discard` or `Accept` is in flight: nothing more starts.
fn finishing(run: &Run) -> bool {
    run.pending_ops
        .values()
        .any(|p| matches!(p.kind, OpKind::Discard { .. } | OpKind::Accept { .. }))
}

/// The integration worktree exists (its `CreateRunBranch` has come back).
fn integration_ready(run: &Run) -> bool {
    !run.pending_ops
        .values()
        .any(|p| matches!(p.kind, OpKind::CreateRunBranch { .. }))
}

/// Decision 31: `queued` is runnable, `pending` waits for dependencies.
fn requeue(run: &mut Run) {
    for i in 0..run.tasks.len() {
        let state = run.tasks[i].state;
        if !matches!(state, TaskState::Pending | TaskState::Queued) {
            continue;
        }
        let next = if deps_done(run, &run.tasks[i]) {
            TaskState::Queued
        } else {
            TaskState::Pending
        };
        run.tasks[i].state = next;
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
    task.state = TaskState::Blocked;
    task.block = Some(BlockInfo { reason, text });
}

fn prepare_in_flight(run: &Run, i: usize) -> bool {
    op_in_flight(run, run.tasks[i].id(), |k| {
        matches!(k, OpKind::PrepareWorktree { .. })
    })
}

/// A `PrepareWorktree` for task `i` from `from`, with the profile's `setup`.
fn prepare(run: &mut Run, i: usize, from: String, fx: &mut Vec<Effect>) {
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
fn prewarm(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let mut held = (0..run.tasks.len())
        .filter(|&i| {
            let t = &run.tasks[i];
            !t.state.is_finished() && (t.prewarmed || prepare_in_flight(run, i))
        })
        .count();
    for i in dispatch_order(run) {
        if held >= usize::from(run.limits.max_writers) {
            break;
        }
        let task = &run.tasks[i];
        if task.state != TaskState::Queued
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
fn dispatch_writers(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for i in dispatch_order(run) {
        if run.tasks[i].state != TaskState::Queued {
            continue;
        }
        let busy = writers_busy(run);
        if busy >= usize::from(run.limits.max_writers) || hub_holds_slot(run) {
            break;
        }
        if run.tasks[i].hub && busy > 0 {
            continue;
        }
        run.tasks[i].state = TaskState::Preparing;
        history(run, i, now, "dispatched");
        if prepare_in_flight(run, i) {
            // A pre-warm still running: its result continues the dispatch.
            continue;
        }
        let task = &run.tasks[i];
        if task.prewarmed && task.start_commit.is_none() && run.base_sha == run.run_head {
            let start = run.run_head.clone();
            launch_worker(run, i, start, now, fx);
        } else {
            // Decision 19: from the run head. A pre-warmed branch that is now stale is
            // re-pointed by the git layer, and its setup runs again (ruling T8-I4).
            let head = run.run_head.clone();
            prepare(run, i, head, fx);
        }
    }
}

/// A new worker session for task `i`, starting at `start` (decisions 24–26, 30).
fn launch_worker(run: &mut Run, i: usize, start: String, now: u64, fx: &mut Vec<Effect>) {
    if window_limit_reached(run, i, now) {
        return;
    }
    let op = next_op(run);
    {
        let task = &mut run.tasks[i];
        task.start_commit = Some(start);
        task.session += 1;
    }
    let task = &run.tasks[i];
    let spec = worker_spec(run, task);
    let first_turn = worker_prompt(run, task);
    let name = format!("{}/{}.w{}", run.short(), task.id(), task.session);
    let uuid = (task.route.runtime == Runtime::Claude).then(|| session_uuid(&run.id, op));
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
    };
    emit_op(run, op, Some(&id), kind, fx);
}

/// Decision 16: every run window counts toward `max_windows`; a task that would pass it
/// is `blocked(environment)`.
fn window_limit_reached(run: &mut Run, i: usize, now: u64) -> bool {
    if run.windows_created < run.limits.max_windows {
        return false;
    }
    let text = format!("run window limit ({}) reached", run.limits.max_windows);
    block(run, i, BlockReason::Environment, text, now);
    true
}

/// A round whose first turn is open from its launch (decision 27: the reducer marks the
/// turn open when it delivers one).
fn new_round(
    role: AgentRole,
    session: u32,
    route: proto::Route,
    op: OpId,
    session_id: Option<String>,
    now: u64,
) -> AgentRound {
    AgentRound {
        role,
        session,
        round: session,
        window_id: None,
        route,
        launch_op: op,
        session_id,
        pid: None,
        ended: false,
        started_at: now,
        ended_at: None,
        turn_open: true,
        turns: 1,
        turn_had_task_done: false,
        last_event: now,
        tool_calls: 0,
        rate_limited_until: None,
        in_retry_streak: false,
        open_subagents: Default::default(),
        denials: 0,
        usage: Default::default(),
        deaths: 0,
        fallback: Default::default(),
        stall: Default::default(),
        failed_turn: Default::default(),
        review_nudged: false,
        wrap_up_sent: false,
        retiring: false,
        delivery_failures: 0,
    }
}

/// Reader dispatch: a `PrepareReview` for each task in `review` without a reviewer,
/// while reader slots are free. A hub task holding a writer slot blocks every dispatch,
/// reviews included (decision 41, read literally).
fn dispatch_reviewers(run: &mut Run, fx: &mut Vec<Effect>) {
    if hub_holds_slot(run) {
        return;
    }
    for i in dispatch_order(run) {
        if readers_busy(run) >= usize::from(run.limits.max_readers) {
            break;
        }
        if !needs_reviewer(run, &run.tasks[i]) {
            continue;
        }
        let op = next_op(run);
        let task = &run.tasks[i];
        let kind = OpKind::PrepareReview {
            root: run.root.clone(),
            head_ref: task.branch.clone(),
            base_ref: task
                .start_commit
                .clone()
                .unwrap_or_else(|| run.run_head.clone()),
            path: run.review_path(task.id()),
        };
        let id = task.id().to_string();
        emit_op(run, op, Some(&id), kind, fx);
    }
}

/// M8a.6 ruling N5: a started task holding a message until its dependencies finished
/// gets the run head merged into its worktree first (decision 36's hand-back), and
/// resumes only when that comes back.
fn resume_held(run: &mut Run, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        let question = task
            .block
            .as_ref()
            .is_some_and(|b| b.reason == BlockReason::Question);
        if task.state != TaskState::Blocked
            || !task.awaiting_deps
            || !question
            || !deps_done(run, task)
            || op_in_flight(run, task.id(), |k| matches!(k, OpKind::HandBack { .. }))
        {
            continue;
        }
        let (id, worktree) = (task.id().to_string(), task.worktree.clone());
        let op = next_op(run);
        let kind = OpKind::HandBack {
            worktree,
            run_head: run.run_head.clone(),
        };
        emit_op(run, op, Some(&id), kind, fx);
    }
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
        let salvage_ref = salvage_ref(run, task.id(), task.salvage_refs.len() + 1);
        let (id, path) = (task.id().to_string(), task.worktree.clone());
        fx.push(Effect::UnwatchWorktree { root: path.clone() });
        let op = next_op(run);
        let kind = OpKind::RemoveWorktree {
            root: run.root.clone(),
            path,
            salvage_ref,
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
        OpResult::Worktree { .. } if state == TaskState::Preparing => {
            if from == run.run_head {
                launch_worker(run, i, from, now, fx);
            } else {
                let head = run.run_head.clone();
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
    match result {
        OpResult::Window { window_id } => {
            let round = &mut run.tasks[i].rounds[r];
            round.window_id = Some(window_id);
            if state.is_finished() {
                round.retiring = true;
                fx.push(Effect::KillWindow { window_id });
            } else if state == TaskState::Preparing && round.role == AgentRole::Worker {
                run.tasks[i].state = TaskState::Working;
            }
        }
        OpResult::Failed { message } => {
            let round = &mut run.tasks[i].rounds[r];
            round.ended = true;
            round.turn_open = false;
            round.ended_at = Some(now);
            if !state.is_finished() {
                let text = format!("could not start the session: {message}");
                block(run, i, BlockReason::Environment, text, now);
            }
        }
        _ => {}
    }
}

/// The result of a `PrepareReview`: a fresh reviewer session (decision 35).
pub(super) fn review_ready(
    run: &mut Run,
    i: usize,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if run.tasks[i].state != TaskState::Review {
        return;
    }
    let (base, head, patch) = match result {
        OpResult::Review { base, head, patch } => (base, head, patch),
        OpResult::Failed { message } => {
            let text = format!("could not prepare the review worktree: {message}");
            return block(run, i, BlockReason::Environment, text, now);
        }
        _ => return,
    };
    if window_limit_reached(run, i, now) {
        return;
    }
    let op = next_op(run);
    let task = &run.tasks[i];
    let level = task
        .review_level
        .unwrap_or(crate::run::model::ReviewLevel::Medium);
    let route = task
        .review_route
        .clone()
        .unwrap_or_else(|| pick_reviewer(&run.roster, &task.route, level));
    let spec = reviewer_spec(run, task, &route);
    let round_no = spec.run_ref.as_ref().map_or(1, |r| r.session);
    let first_turn = reviewer_prompt(run, task, round_no, &base, &head, &patch);
    let name = format!("{}/{}.r{round_no}", run.short(), task.id());
    let uuid = (route.runtime == Runtime::Claude).then(|| session_uuid(&run.id, op));
    let jitter = jitter_ms(&run.id, &format!("{}.r", task.id()), round_no);
    let mut round = new_round(AgentRole::Reviewer, round_no, route, op, uuid.clone(), now);
    round.round = round_no;
    let id = task.id().to_string();
    let worktree = run.review_path(&id);
    run.tasks[i].rounds.push(round);
    run.windows_created += 1;
    history(run, i, now, format!("review round {round_no} starting"));
    let kind = OpKind::CreateWindow {
        name,
        spec: Box::new(spec),
        session_uuid: uuid,
        first_turn,
        project: run.project.clone(),
        worktree,
        jitter_ms: jitter,
    };
    emit_op(run, op, Some(&id), kind, fx);
}

/// The result of an N5 `HandBack`: the task resumes, with any conflict to resolve.
/// A hand-back from the merge queue (decision 36) is M8a.14's.
pub(super) fn handed_back(run: &mut Run, i: usize, result: OpResult, now: u64) {
    let task = &run.tasks[i];
    if task.state != TaskState::Blocked || !task.awaiting_deps {
        return;
    }
    match result {
        OpResult::HandedBack { files } => {
            let task = &mut run.tasks[i];
            task.awaiting_deps = false;
            task.state = TaskState::Working;
            task.block = None;
            history(run, i, now, "dependencies merged; resuming");
            if !files.is_empty() {
                let id = run.tasks[i].id().to_string();
                outbox::queue(run, &id, conflict_message(&files), now);
            }
        }
        OpResult::Failed { message } => {
            let text = format!("could not merge the run head into its worktree: {message}");
            block(run, i, BlockReason::Environment, text, now);
        }
        _ => {}
    }
}

/// The result of a `RemoveWorktree`.
pub(super) fn removed(run: &mut Run, i: usize, result: OpResult, now: u64) {
    match result {
        OpResult::Removed { salvage_ref } => {
            let task = &mut run.tasks[i];
            task.worktree_live = false;
            task.prewarmed = false;
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
