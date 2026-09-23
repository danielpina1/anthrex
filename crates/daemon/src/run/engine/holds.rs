//! M8a.6 ruling N5 as task state (M8a.11 fix round 1, ruling T11-I1..I3): the hold of a
//! started task that gains an unfinished dependency, the one hand-back that ends it, and
//! the hand-back's result. Pure (design decision 2).

use proto::{BlockInfo, BlockReason, TaskState};

use super::dispatch::{block, history};
use super::schedule::{deps_done, op_in_flight, unfinished_deps};
use super::{Effect, OpKind, OpResult, emit_op, next_op, outbox};
use crate::run::contract::{conflict_message, is_conflict_message};
use crate::run::model::Run;

/// M8a.6 ruling N5 as task state (ruling T11-I1..I3): a started task that has an
/// unfinished dependency carries the hold (`awaiting_deps`) from the moment it gains
/// one. While held it is never `working`: an answer that un-blocked it is taken back,
/// and its messages wait in the outbox. Only `handed_back` clears the hold; a
/// `dep_cancelled` block replaces it (review minor 8).
pub(super) fn enforce_holds(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    for i in 0..run.tasks.len() {
        let task = &run.tasks[i];
        if task.state.is_finished() || task.start_commit.is_none() {
            continue;
        }
        if task
            .block
            .as_ref()
            .is_some_and(|b| b.reason == BlockReason::DepCancelled)
        {
            run.tasks[i].awaiting_deps = false;
            run.tasks[i].held_answered = false;
            continue;
        }
        let waiting = unfinished_deps(run, task);
        if !waiting.is_empty() && !task.awaiting_deps {
            run.tasks[i].awaiting_deps = true;
            history(
                run,
                i,
                now,
                format!("held until {} finish", waiting.join(", ")),
            );
            abort_untold_conflict(run, i, now, fx);
        }
        if run.tasks[i].awaiting_deps && run.tasks[i].state == TaskState::Working {
            let text = if waiting.is_empty() {
                "answered; resumes once the run head is merged into its worktree".to_string()
            } else {
                format!("answered; resumes once {} merged", waiting.join(", "))
            };
            let task = &mut run.tasks[i];
            task.held_answered = true;
            task.state = TaskState::Blocked;
            task.block = Some(BlockInfo {
                reason: BlockReason::Question,
                text,
            });
        }
    }
}

/// Re-review 2 M2: a task held again whose last hand-back conflicted but whose worker
/// was never told (the conflict message is still undelivered, as for an unanswered
/// task) gets that merge undone and the message dropped, as ruling T11-N1(b) does for
/// a conflict during a hold. The next hand-back brings the conflict again.
fn abort_untold_conflict(run: &mut Run, i: usize, now: u64, fx: &mut Vec<Effect>) {
    let id = run.tasks[i].id().to_string();
    let untold = |m: &crate::run::model::Outgoing| {
        m.task_id == id && m.delivered_at.is_none() && is_conflict_message(&m.text)
    };
    if !run.outbox.iter().any(untold)
        || op_in_flight(run, &id, |k| {
            matches!(k, OpKind::HandBack { .. } | OpKind::AbortMerge { .. })
        })
    {
        return;
    }
    run.outbox.retain(|m| !untold(m));
    let worktree = run.tasks[i].worktree.clone();
    let op = next_op(run);
    emit_op(run, op, Some(&id), OpKind::AbortMerge { worktree }, fx);
    history(
        run,
        i,
        now,
        "untold conflict undone until every dependency finishes",
    );
}

/// M8a.6 ruling N5: a held task that a message waits for (an answer, or an amendment)
/// gets the run head merged into its worktree first (decision 36's hand-back) once
/// every dependency has finished, and resumes only when that comes back. One hand-back
/// at a time, and none while a conflicted one is being aborted (ruling T11-N1(b)).
pub(super) fn resume_held(run: &mut Run, fx: &mut Vec<Effect>) {
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
            || !run
                .outbox
                .iter()
                .any(|m| m.task_id == task.spec.id && m.delivered_at.is_none())
            || op_in_flight(run, task.id(), |k| {
                matches!(k, OpKind::HandBack { .. } | OpKind::AbortMerge { .. })
            })
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

/// The result of an N5 `HandBack` (rulings T11-I1..I3 and T11-N1..N3). A conflict
/// while a dependency is still unfinished is undone (`AbortMerge`) and dropped: the
/// hand-back after the last dependency brings it again, so the worker hears of one
/// conflict per hand-back. Any other conflict queues the conflict message, even when
/// the task is no longer held (its dependency was cancelled meanwhile). A dependency
/// gained while it ran keeps the hold, and another hand-back follows once it finishes.
/// Otherwise the hold is cleared: an answered task resumes, its queued messages going
/// out as one turn; an unanswered one is back on its question, its messages waiting
/// for the answer. A hand-back from the merge queue (decision 36) is M8a.14's.
pub(super) fn handed_back(
    run: &mut Run,
    i: usize,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let task = &run.tasks[i];
    if task.state.is_finished() {
        return;
    }
    let held = task.state == TaskState::Blocked && task.awaiting_deps;
    match result {
        OpResult::HandedBack { files, .. } => {
            let id = run.tasks[i].id().to_string();
            let waiting = unfinished_deps(run, &run.tasks[i]);
            if held && !waiting.is_empty() {
                let note = if files.is_empty() {
                    "run head merged; still waiting for a dependency"
                } else {
                    let worktree = run.tasks[i].worktree.clone();
                    let op = next_op(run);
                    emit_op(run, op, Some(&id), OpKind::AbortMerge { worktree }, fx);
                    "run head conflicted; undoing the merge until every dependency finishes"
                };
                if run.tasks[i].held_answered {
                    let text = format!("answered; resumes once {} merged", waiting.join(", "));
                    run.tasks[i].block = Some(BlockInfo {
                        reason: BlockReason::Question,
                        text,
                    });
                }
                history(run, i, now, note);
                return;
            }
            if !files.is_empty() {
                outbox::queue(run, &id, conflict_message(&files), now);
            }
            if !held {
                return;
            }
            let task = &mut run.tasks[i];
            task.awaiting_deps = false;
            if std::mem::take(&mut task.held_answered) {
                task.state = TaskState::Working;
                task.block = None;
                history(run, i, now, "dependencies merged; resuming");
            } else {
                history(run, i, now, "dependencies merged; waiting for an answer");
            }
        }
        OpResult::Failed { message } if held => {
            let text = format!("could not merge the run head into its worktree: {message}");
            block(run, i, BlockReason::Environment, text, now);
        }
        _ => {}
    }
}

/// The result of ruling T11-N1(b)'s `AbortMerge`. Undone, the hold goes on and the
/// next hand-back follows once every dependency finishes. A failed abort leaves the
/// worktree mid-merge, so the task is blocked on its environment, unless it is already
/// `dep_cancelled` (re-review 2 M1).
pub(super) fn merge_aborted(run: &mut Run, i: usize, result: OpResult, now: u64) {
    if run.tasks[i].state.is_finished() {
        return;
    }
    match result {
        OpResult::MergeAborted => history(run, i, now, "conflicted hand-back undone"),
        OpResult::Failed { message } => {
            let text = format!("could not undo a conflicted hand-back in its worktree: {message}");
            // Re-review 2 M1: a `dep_cancelled` block stays (it is permanent, and an
            // environment block would let the next pass hold the task on the cancelled
            // dependency); its text records the leftover merge.
            let task = &mut run.tasks[i];
            match task.block.as_mut() {
                Some(b) if b.reason == BlockReason::DepCancelled => {
                    b.text = format!("{}; {text}", b.text);
                    history(run, i, now, text);
                }
                _ => block(run, i, BlockReason::Environment, text, now),
            }
        }
        _ => {}
    }
}
