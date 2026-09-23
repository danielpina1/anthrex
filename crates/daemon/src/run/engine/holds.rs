//! M8a.6 ruling N5 as task state (M8a.11 fix round 1, ruling T11-I1..I3): the hold of a
//! started task that gains an unfinished dependency, the one hand-back that ends it, and
//! the hand-back's result. Pure (design decision 2).

use proto::{BlockInfo, BlockReason, TaskState};

use super::dispatch::{block, history};
use super::schedule::{deps_done, op_in_flight, unfinished_deps};
use super::{Effect, OpKind, OpResult, emit_op, next_op, outbox};
use crate::run::contract::conflict_message;
use crate::run::model::Run;

/// M8a.6 ruling N5 as task state (ruling T11-I1..I3): a started task that has an
/// unfinished dependency carries the hold (`awaiting_deps`) from the moment it gains
/// one. While held it is never `working`: an answer that un-blocked it is taken back,
/// and its messages wait in the outbox. Only `handed_back` clears the hold; a
/// `dep_cancelled` block replaces it (review minor 8).
pub(super) fn enforce_holds(run: &mut Run, now: u64) {
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
        }
        if run.tasks[i].awaiting_deps && run.tasks[i].state == TaskState::Working {
            let text = if waiting.is_empty() {
                "answered; resumes once the run head is merged into its worktree".to_string()
            } else {
                format!("answered; resumes once {} merged", waiting.join(", "))
            };
            let task = &mut run.tasks[i];
            task.state = TaskState::Blocked;
            task.block = Some(BlockInfo {
                reason: BlockReason::Question,
                text,
            });
        }
    }
}

/// M8a.6 ruling N5: a held task that has been answered (a message waits for it) gets
/// the run head merged into its worktree first (decision 36's hand-back) once every
/// dependency has finished, and resumes only when that comes back. One hand-back at a
/// time.
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

/// The result of an N5 `HandBack` (ruling T11-I1..I3). A conflict queues the conflict
/// message. A dependency gained while it ran keeps the hold, and another hand-back
/// follows once it finishes. Otherwise the hold is cleared and the task resumes, its
/// queued messages going out as one turn. A hand-back from the merge queue (decision
/// 36) is M8a.14's.
pub(super) fn handed_back(run: &mut Run, i: usize, result: OpResult, now: u64) {
    let task = &run.tasks[i];
    if task.state != TaskState::Blocked || !task.awaiting_deps {
        return;
    }
    match result {
        OpResult::HandedBack { files } => {
            let id = run.tasks[i].id().to_string();
            if !files.is_empty() {
                outbox::queue(run, &id, conflict_message(&files), now);
            }
            let waiting = unfinished_deps(run, &run.tasks[i]);
            if !waiting.is_empty() {
                let text = format!("answered; resumes once {} merged", waiting.join(", "));
                run.tasks[i].block = Some(BlockInfo {
                    reason: BlockReason::Question,
                    text,
                });
                history(
                    run,
                    i,
                    now,
                    "run head merged; still waiting for a dependency",
                );
                return;
            }
            let task = &mut run.tasks[i];
            task.awaiting_deps = false;
            task.state = TaskState::Working;
            task.block = None;
            history(run, i, now, "dependencies merged; resuming");
        }
        OpResult::Failed { message } => {
            let text = format!("could not merge the run head into its worktree: {message}");
            block(run, i, BlockReason::Environment, text, now);
        }
        _ => {}
    }
}
