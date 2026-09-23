//! Decision 32's turn-end fallback: at a completed turn with no accepted `task_done`,
//! `CountCommits`, then `DONE_NUDGE` or `NO_COMMIT_NUDGE`, then the fallback's own claim
//! or a stall; a failed count is retried, and blocks the task at the third failure in a
//! row (M8a.12 fix round 3, ruling T12-A2). Split out of `done.rs` for size. Pure
//! (design decision 2).

use proto::{BlockReason, DoneSignal, TaskState};

use super::dispatch::block;
use super::done::claim;
use super::ladder::{self, worker_round};
use super::tools::DoneArgs;
use super::{Effect, OpId, OpKind, OpResult, emit_op, next_op, outbox};
use crate::run::contract::{DONE_NUDGE, NO_COMMIT_NUDGE};
use crate::run::messages::{DELIVERY_MAX_FAILURES, DELIVERY_RETRY_SECS};
use crate::run::model::{FallbackState, Run};

/// Decision 32's turn-end fallback, at a completed turn with no accepted `task_done`
/// (or at the last `SubagentStop` it waited for): count commits first; after
/// `DONE_NUDGE`'s turn, proceed as if `task_done` had been called; after
/// `NO_COMMIT_NUDGE`'s, count again.
pub(super) fn fallback(run: &mut Run, i: usize, fx: &mut Vec<Effect>) {
    let task = &run.tasks[i];
    if task.state != TaskState::Working || task.claim.is_some() {
        return;
    }
    // One count at a time: none while one is in flight or waiting to be retried.
    let Some(r) = worker_round(task).filter(|&r| {
        let round = &task.rounds[r];
        round.count_op.is_none() && round.count_retry_at.is_none()
    }) else {
        return;
    };
    match task.rounds[r].fallback {
        FallbackState::Nudged { had_commits: true } => {
            run.tasks[i].rounds[r].fallback = FallbackState::None;
            let args = DoneArgs::default();
            claim(run, i, None, args, DoneSignal::TurnEndFallback, fx);
        }
        FallbackState::Counting => {}
        state => {
            if state == FallbackState::None {
                run.tasks[i].rounds[r].fallback = FallbackState::Counting;
            }
            count(run, i, r, fx);
        }
    }
}

/// Sends round `r`'s `CountCommits`, the op it then awaits.
fn count(run: &mut Run, i: usize, r: usize, fx: &mut Vec<Effect>) {
    let task = &run.tasks[i];
    let kind = OpKind::CountCommits {
        worktree: task.worktree.clone(),
        start: task
            .start_commit
            .clone()
            .unwrap_or_else(|| run.run_head.clone()),
        run_head: run.run_head.clone(),
    };
    let id = task.id().to_string();
    let op = next_op(run);
    run.tasks[i].rounds[r].count_op = Some(op);
    emit_op(run, op, Some(&id), kind, fx);
}

/// Ruling T12-A2: a failed count is sent again once its retry time has come, unless
/// the worker's own claim has taken over meanwhile.
pub(super) fn retry_count(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let task = &run.tasks[i];
    let Some(r) = worker_round(task).filter(|&r| {
        let round = &task.rounds[r];
        round.count_retry_at.is_some_and(|at| now >= at)
    }) else {
        return;
    };
    run.tasks[i].rounds[r].count_retry_at = None;
    if run.tasks[i].claim.is_some() {
        run.tasks[i].rounds[r].fallback = FallbackState::None;
        return;
    }
    count(run, i, r, fx);
}

/// Ruling T12-A2: a failed count is retried `DELIVERY_RETRY_SECS` later, as a failed
/// delivery is; the `DELIVERY_MAX_FAILURES`th in a row blocks the task as
/// `blocked(environment)`.
fn count_failed(run: &mut Run, i: usize, r: usize, error: String, now: u64) {
    let round = &mut run.tasks[i].rounds[r];
    round.count_failures = round.count_failures.saturating_add(1);
    if round.count_failures < DELIVERY_MAX_FAILURES {
        round.count_retry_at = Some(now + DELIVERY_RETRY_SECS);
        return;
    }
    round.count_failures = 0;
    round.fallback = FallbackState::None;
    let text = format!("could not count the task's commits: {error}");
    block(run, i, BlockReason::Environment, text, now);
}

/// `CountCommits`' result: the nudge that becomes the next turn, or, when the
/// no-commit nudge's turn also ended with no commit, a stall. Only the count the
/// current worker round awaits counts (ruling T12-N).
pub(super) fn counted(
    run: &mut Run,
    i: usize,
    op: OpId,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let task = &run.tasks[i];
    let Some(r) = worker_round(task).filter(|&r| task.rounds[r].count_op == Some(op)) else {
        return;
    };
    run.tasks[i].rounds[r].count_op = None;
    let task = &run.tasks[i];
    let fallback = task.rounds[r].fallback;
    // Review m-2: no nudge while the worker's own claim is being checked.
    if task.state != TaskState::Working || task.claim.is_some() {
        run.tasks[i].rounds[r].fallback = FallbackState::None;
        return;
    }
    let count = match result {
        OpResult::Commits { count, .. } => count,
        OpResult::Failed { message } => return count_failed(run, i, r, message, now),
        _ => {
            run.tasks[i].rounds[r].fallback = FallbackState::None;
            return;
        }
    };
    run.tasks[i].rounds[r].count_failures = 0;
    let task = &run.tasks[i];
    let id = task.id().to_string();
    let (text, next) = match (fallback, count) {
        (FallbackState::Counting | FallbackState::Nudged { .. }, 1..) => {
            (DONE_NUDGE, FallbackState::Nudged { had_commits: true })
        }
        (FallbackState::Counting, 0) => (
            NO_COMMIT_NUDGE,
            FallbackState::Nudged { had_commits: false },
        ),
        (FallbackState::Nudged { had_commits: false }, 0) => {
            run.tasks[i].rounds[r].fallback = FallbackState::None;
            let reason = "its turn ended twice with no commit and no task_done".to_string();
            return ladder::stall(run, i, reason, now, fx);
        }
        _ => return,
    };
    run.tasks[i].rounds[r].fallback = next;
    outbox::queue(run, &id, text.to_string(), now);
}
