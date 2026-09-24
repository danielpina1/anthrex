//! Decision 37's completion, engine side (M8a.14): the ref guard before `complete`
//! (`VerifyRefs`), the final check on a run head no candidate check covered, and the
//! report; the `finish` edit and `run cancel`, which end a run early; and `run accept`
//! and `run discard` on a complete run (decision 20), answered with their op's result.
//! Pure (design decision 2).

use proto::{BlockInfo, BlockReason, FinishAction, RunState, TaskState};

use super::dispatch::{finishing_as, history, salvage_ref};
use super::merge::{candidate_in_flight, halt, next_salvage_seq};
use super::requests::log;
use super::{Effect, EngineState, OpKind, OpResult, ReplyId, emit_op, ladder, next_op, review};
use crate::run::contract::accept_conflict_message;
use crate::run::env::profile_env;
use crate::run::model::Run;

/// A task still moving toward `merged` or `blocked` on its own: the `finish` edit waits
/// for these.
fn live(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Preparing
            | TaskState::Working
            | TaskState::Proof
            | TaskState::Check
            | TaskState::Review
            | TaskState::MergeQueue
    )
}

/// The reply's note for a cancel deferred behind task `id`'s in-flight merge.
pub(super) fn deferred_note(id: &str) -> String {
    format!("; {id}'s merge is in flight: it is cancelled only if that merge does not land")
}

/// Task `i` is cancelled as the run ends, or, while its `MergeCandidate` runs, marked to
/// be once that merge does not land (ruling T14-I1). Returns whether it was deferred.
fn cancel_task(run: &mut Run, i: usize, why: &str, now: u64, fx: &mut Vec<Effect>) -> bool {
    if candidate_in_flight(run, i) {
        if !std::mem::replace(&mut run.tasks[i].cancel_deferred, true) {
            history(run, i, now, "cancel deferred: its merge is in flight");
        }
        return true;
    }
    cancel_now(run, i, why, now, fx);
    false
}

/// Task `i` is cancelled: its sessions are killed (a worker's claim answered, a
/// reviewer given up), its messages dropped, and it leaves the merge queue; every
/// unfinished task that declared it becomes `blocked(dep_cancelled)`, as decision 13's
/// `cancel_task` edit does. Its worktree is salvaged and removed once no session is
/// left (`dispatch::remove_cancelled_worktrees`).
pub(super) fn cancel_now(run: &mut Run, i: usize, why: &str, now: u64, fx: &mut Vec<Effect>) {
    ladder::kill_worker(run, i, fx);
    review::stop_reviewers(run, i, now, fx);
    let task = &mut run.tasks[i];
    task.state = TaskState::Cancelled;
    task.block = None;
    task.awaiting_deps = false;
    task.held_answered = false;
    task.fresh_session = None;
    task.merge_op = None;
    task.cancel_deferred = false;
    task.handback_due = false;
    task.resolving = false;
    task.ready_from = None;
    let id = task.id().to_string();
    run.outbox.retain(|m| m.task_id != id);
    run.merge_queue.retain(|q| *q != id);
    history(run, i, now, format!("cancelled: {why}"));
    for j in 0..run.tasks.len() {
        let dependent = &run.tasks[j];
        if dependent.state.is_finished() || !dependent.spec.deps.contains(&id) {
            continue;
        }
        let text = format!("dependency {id} was cancelled");
        history(run, j, now, format!("blocked: {text}"));
        let dependent = &mut run.tasks[j];
        dependent.state = TaskState::Blocked;
        dependent.awaiting_deps = false;
        dependent.held_answered = false;
        dependent.block = Some(BlockInfo {
            reason: BlockReason::DepCancelled,
            text,
        });
    }
}

/// The `finish` edit (decision 37), every scheduler pass of a running run: every task
/// that has not started is cancelled at once; once no task is live, the blocked ones
/// are cancelled too, and completion follows.
pub(super) fn finish_pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    if !run.finish_edit {
        return;
    }
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        if !task.state.is_finished() && task.start_commit.is_none() {
            cancel_task(run, i, "the finish edit (not started)", now, fx);
        }
    }
    if run.tasks.iter().any(|t| live(t.state)) {
        return;
    }
    for i in 0..run.tasks.len() {
        if run.tasks[i].state == TaskState::Blocked {
            cancel_task(run, i, "the finish edit (blocked)", now, fx);
        }
    }
}

/// Decision 37: a running run whose tasks are all `merged` or `cancelled`, with an
/// empty merge queue, no op pending and no cancelled task's session still ending, runs
/// the ref guard first.
pub(super) fn complete_pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let finished = run
        .tasks
        .iter()
        .all(|t| matches!(t.state, TaskState::Merged | TaskState::Cancelled));
    let ending = run
        .tasks
        .iter()
        .any(|t| t.state == TaskState::Cancelled && t.rounds.iter().any(|r| !r.ended));
    if !finished || ending || !run.merge_queue.is_empty() || !run.pending_ops.is_empty() {
        return;
    }
    let kind = OpKind::VerifyRefs {
        root: run.root.clone(),
        base_branch: run.base_branch.clone(),
        expected_base: run.base_sha.clone(),
        run_branch: run.run_branch(),
        expected_run_head: run.run_head.clone(),
    };
    let op = next_op(run);
    emit_op(run, op, None, kind, fx);
    log(run, now, "every task is finished; verifying the refs");
}

/// `VerifyRefs`' result: the refs as recorded lead to the final check when the run head
/// is neither the last green candidate nor the base (only after a rebaseline), else to
/// `complete`. A moved run ref or a rewritten base halts (decision 21). Refs that could
/// not be read are read again by the next pass; a second failure in a row halts, with
/// a reason a plain `run resume` retries (review m1, invented), so completion is never
/// decided on refs that were not read.
pub(super) fn refs_verified(run: &mut Run, result: OpResult, now: u64, fx: &mut Vec<Effect>) {
    if !matches!(result, OpResult::Failed { .. }) {
        run.verify_failures = 0;
    }
    match result {
        OpResult::RefsOk if run.state == RunState::Running => {
            let green = run.last_green_candidate.as_deref() == Some(run.run_head.as_str());
            match run.profile.check.clone() {
                Some(command) if !green && run.run_head != run.base_sha => {
                    let dir = run.integration_path();
                    let kind = OpKind::Check {
                        env: profile_env(&run.profile, &dir),
                        dir,
                        command,
                        timeout_secs: run.profile.check_timeout_secs,
                        scratch: None,
                    };
                    let op = next_op(run);
                    emit_op(run, op, None, kind, fx);
                    log(run, now, "running the final check on the run head");
                }
                _ => complete(run, now, fx),
            }
        }
        OpResult::RefMoved { reason } => halt(run, reason, now),
        OpResult::Failed { message } => {
            run.verify_failures = run.verify_failures.saturating_add(1);
            if run.verify_failures < 2 {
                log(
                    run,
                    now,
                    format!("could not verify the refs: {message}; reading them again"),
                );
                return;
            }
            run.verify_failures = 0;
            halt(run, format!("could not verify the refs: {message}"), now);
            run.halt_retryable = true;
        }
        _ => {}
    }
}

/// The final check's result (decision 37): red is an attention line, and the run
/// completes either way.
pub(super) fn final_checked(run: &mut Run, result: OpResult, now: u64, fx: &mut Vec<Effect>) {
    if run.state != RunState::Running {
        return;
    }
    match result {
        OpResult::Check { ok, .. } => {
            run.final_check_failed = !ok;
            if !ok {
                log(run, now, "final check failed on the run head");
            }
        }
        OpResult::SetupFailed { output: message } | OpResult::Failed { message } => {
            run.final_check_failed = true;
            log(
                run,
                now,
                format!("could not run the final check: {message}"),
            );
        }
        _ => return,
    }
    complete(run, now, fx);
}

fn complete(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let count = |state| run.tasks.iter().filter(|t| t.state == state).count();
    let (merged, cancelled) = (count(TaskState::Merged), count(TaskState::Cancelled));
    run.state = RunState::Complete;
    log(
        run,
        now,
        format!("complete: {merged} merged, {cancelled} cancelled"),
    );
    fx.push(Effect::WriteReport {
        run_id: run.id.clone(),
    });
}

/// `run cancel` (decision 37): every run session is killed and every unmerged task
/// cancelled (salvaged once its session has exited), and the run completes. A paused
/// run runs again to complete; a halted one completes after `resume --rebaseline`.
pub(super) fn cancel(
    state: &mut EngineState,
    reply: ReplyId,
    run_id: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let answer = |fx: &mut Vec<Effect>, result| fx.push(Effect::Reply { reply, result });
    let Some(run) = state.runs.get_mut(run_id) else {
        return answer(fx, Err(format!("unknown run {run_id}")));
    };
    if let Some(how) = finishing_as(run) {
        return answer(fx, Err(format!("run {run_id} is being {how}")));
    }
    match run.state {
        RunState::Running | RunState::Halted => {}
        RunState::Paused => {
            run.state = RunState::Running;
            run.paused_from = None;
        }
        other => return answer(fx, Err(format!("run {run_id} is {}", other.label()))),
    }
    // T15-minors (M-4): nothing resumes a cancelled run.
    run.restored = None;
    let mut merging = Vec::new();
    for i in 0..run.tasks.len() {
        if !run.tasks[i].state.is_finished() && cancel_task(run, i, "run cancel", now, fx) {
            merging.push(run.tasks[i].id().to_string());
        }
    }
    run.cancelled = true;
    log(run, now, "cancelled by the user");
    let mut text = format!("run {run_id} cancelled; it completes once its sessions have ended");
    for id in merging {
        text.push_str(&deferred_note(&id));
    }
    answer(fx, Ok(text));
}

/// Every engine-owned worktree of the run, each with its next salvage ref: every
/// task's own, review and proof worktrees (the executor skips one that no longer
/// exists), then the integration worktree.
fn run_worktrees(run: &Run) -> Vec<(std::path::PathBuf, String)> {
    let mut out = Vec::new();
    for task in &run.tasks {
        let id = task.id();
        let seq = next_salvage_seq(task);
        let paths = [
            task.worktree.clone(),
            run.review_path(id),
            run.proof_path(id),
        ];
        for (k, path) in paths.into_iter().enumerate() {
            out.push((path, salvage_ref(run, id, seq + k)));
        }
    }
    out.push((run.integration_path(), salvage_ref(run, "integration", 1)));
    out
}

/// `run accept` and `run discard` (decision 20) on a complete run: one `Accept` or
/// `Discard` op, answered when its result comes. The driver has already checked the
/// confirmation and, for accept, the base (decision 20's moved base); `expected_base` is
/// the base head the user confirmed.
pub(super) fn finish(
    state: &mut EngineState,
    reply: ReplyId,
    run_id: &str,
    action: FinishAction,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let answer = |fx: &mut Vec<Effect>, result| fx.push(Effect::Reply { reply, result });
    let verb = match action {
        FinishAction::Accept => "accept",
        FinishAction::Discard => "discard",
    };
    let Some(run) = state.runs.get_mut(run_id) else {
        return answer(fx, Err(format!("unknown run {run_id}")));
    };
    if let Some(how) = finishing_as(run) {
        return answer(fx, Err(format!("run {run_id} is being {how}")));
    }
    // Review m2: a cancelled run that is halted has nothing left to verify for a
    // discard, so it needs no rebaseline first.
    let discardable = action == FinishAction::Discard
        && run.state == RunState::Halted
        && run.cancelled
        && run.tasks.iter().all(|t| t.state.is_finished())
        && run.pending_ops.is_empty();
    if run.state != RunState::Complete && !discardable {
        let label = run.state.label();
        let text = format!("run {run_id} is {label}; {verb} applies only to a complete run");
        return answer(fx, Err(text));
    }
    let (root, worktrees) = (run.root.clone(), run_worktrees(run));
    let branch_prefix = format!("anthrex/{run_id}/");
    let kind = match action {
        FinishAction::Accept => OpKind::Accept {
            root,
            base_branch: run.base_branch.clone(),
            expected_base: run
                .base_moved
                .as_ref()
                .map_or_else(|| run.base_sha.clone(), |m| m.to.clone()),
            run_branch: run.run_branch(),
            message: format!("anthrex: accept run {run_id}: {}", run.goal),
            worktrees,
            branch_prefix,
        },
        FinishAction::Discard => OpKind::Discard {
            root,
            worktrees,
            branch_prefix,
        },
    };
    let op = next_op(run);
    emit_op(run, op, None, kind, fx);
    run.finish_reply = Some(reply);
    log(run, now, format!("{verb} requested by the user"));
}

/// An `Accept`'s or a `Discard`'s result (decision 20), for `run accept`/`discard` and
/// for `run reject` alike. `Finished` makes the run `accepted` or `discarded`, removes
/// its windows and logs any branch kept because it is checked out (carry T9). An
/// accept that conflicts leaves the run `complete` with its branches, and is refused
/// with decision 20's text.
pub(super) fn finished(
    run: &mut Run,
    kind: &OpKind,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let accept = matches!(kind, OpKind::Accept { .. });
    let reply = run.finish_reply.take();
    let answer = |fx: &mut Vec<Effect>, result| {
        if let Some(reply) = reply {
            fx.push(Effect::Reply { reply, result });
        }
    };
    let id = run.id.clone();
    match result {
        OpResult::Finished {
            outcome,
            kept_branches,
        } => {
            if !kept_branches.is_empty() {
                let text = format!(
                    "branches kept because a worktree has them checked out: {}",
                    kept_branches.join(", ")
                );
                log(run, now, text);
            }
            let (state, done) = if accept {
                (RunState::Accepted, "accepted")
            } else {
                (RunState::Discarded, "discarded")
            };
            run.state = state;
            log(run, now, format!("{done}: {outcome}"));
            run.outcome = Some(outcome.clone());
            let mut windows: Vec<u32> = run
                .tasks
                .iter()
                .flat_map(|t| t.rounds.iter())
                .filter_map(|r| r.window_id)
                .collect();
            windows.sort_unstable();
            windows.dedup();
            fx.extend(
                windows
                    .into_iter()
                    .map(|window_id| Effect::RemoveWindow { window_id }),
            );
            fx.push(Effect::WriteReport { run_id: id.clone() });
            answer(fx, Ok(format!("run {id} {done}: {outcome}")));
        }
        OpResult::AcceptConflict { files } if accept => {
            let commits = run.base_moved.as_ref().map_or(0, |m| m.commits);
            let text = accept_conflict_message(&id, &run.base_branch, commits, &files);
            log(run, now, text.clone());
            answer(fx, Err(text));
        }
        OpResult::Failed { message } => {
            let verb = if accept { "accept" } else { "discard" };
            log(run, now, format!("{verb} failed: {message}"));
            answer(fx, Err(message));
        }
        _ => {}
    }
}
