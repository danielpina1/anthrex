//! Milestone 9.1 decisions 51 and ruling C-21, engine side: the `sync` fix task a
//! conflicted propagate adds. Its worktree starts at the upper stage's head the
//! conflict was found on, and before its first session M8a's hand-back merges the lower
//! head into it; its claims must keep that merge; when it merges, the stage holds the
//! lower head. A child of `propagate.rs`, split out to keep that file under 600 lines.
//! Pure (design decision 2).

use proto::{BlockReason, RunState, TaskState};

use super::super::dispatch::{block, history};
use super::super::{Effect, OpKind, OpResult, emit_op, next_op};
use super::stage_mut;
use crate::run::contract::sha7;
use crate::run::model::{FixOf, Run, Task};

/// Controller ruling C-21 (5): a sync claim whose head dropped the lower stage's merge
/// (milestone 9.2: a base sync task's, the base commit's; invented).
pub(in crate::run::engine) fn lost_merge(run: &Run, task: &Task) -> String {
    let onto = task.sync.as_ref().map_or("", |s| s.onto.as_str());
    let what = match &task.fixes {
        Some(FixOf::Propagate { from, .. }) => format!("stage {from}'s merge"),
        Some(FixOf::Base { .. }) => format!("the merge of {}@{}", run.base_branch, sha7(onto)),
        _ => format!("stage {}'s merge", task.stage().saturating_sub(1)),
    };
    format!(
        "task_done rejected: sync task must keep {what}: its head does not contain {}",
        sha7(onto)
    )
}

/// Controller ruling C-21 (3): a hand-back into a sync task after its first (the merge
/// queue's, a held task's or a refresh) brought the upper stage's head `run_head` in;
/// its claims no longer count that head's changes as the task's.
pub(in crate::run::engine) fn record_hand_back(
    run: &mut Run,
    i: usize,
    run_head: &str,
    result: &OpResult,
) {
    if !matches!(result, OpResult::HandedBack { .. }) {
        return;
    }
    if let Some(sync) = run.tasks[i].sync.as_mut().filter(|s| s.handed_back)
        && sync.handed.last().map(String::as_str) != Some(run_head)
        && sync.to_head != run_head
    {
        sync.handed.push(run_head.to_string());
    }
}

/// A `sync` fix task whose merge is not yet in its worktree (decision 51).
pub(in crate::run::engine) fn sync_due(task: &Task) -> bool {
    task.sync.as_ref().is_some_and(|s| !s.handed_back)
}

/// Where a task's worktree starts: a sync task's at the upper stage's head its conflict
/// was found on, so the hand-back's merge is exactly the conflicted tree; any other at
/// its stage's head (decision 47).
pub(in crate::run::engine) fn start_of(run: &Run, task: &Task) -> String {
    match &task.sync {
        Some(sync) if !sync.to_head.is_empty() => sync.to_head.clone(),
        _ => run.head_for(task).to_string(),
    }
}

/// Decision 51: before a sync task's first session, M8a's `HandBack` merges the lower
/// stage's head into its worktree (`merge-tree`, the markers and an engine-written
/// `MERGE_HEAD`, never `git merge`). Called where the session would start; `true` when
/// the session waits for it.
pub(in crate::run::engine) fn hand_back_first(
    run: &mut Run,
    i: usize,
    start: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) -> bool {
    let task = &run.tasks[i];
    let Some(sync) = task.sync.as_ref().filter(|s| !s.handed_back) else {
        return false;
    };
    let id = task.id().to_string();
    let waiting =
        super::super::schedule::op_in_flight(run, &id, |k| matches!(k, OpKind::HandBack { .. }));
    if waiting {
        return true;
    }
    let onto = sync.onto.clone();
    let kind = OpKind::HandBack {
        worktree: task.worktree.clone(),
        run_head: onto.clone(),
        task_head: Some(start.to_string()),
        list_merged: false,
    };
    let op = next_op(run);
    emit_op(run, op, Some(&id), kind, fx);
    let text = format!("merging {} into its worktree first", sha7(&onto));
    history(run, i, now, text);
    true
}

/// The sync task's `HandBack` result: its session starts at the commit the merge was
/// made onto; a failure blocks it (`environment`), as a failed prepare does.
pub(in crate::run::engine) fn handed_back(
    run: &mut Run,
    i: usize,
    start: Option<String>,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    if run.tasks[i].state != TaskState::Preparing {
        return;
    }
    match result {
        OpResult::HandedBack { files, onto, .. } => {
            let start = onto
                .or(start)
                .unwrap_or_else(|| start_of(run, &run.tasks[i]));
            if let Some(sync) = run.tasks[i].sync.as_mut() {
                sync.handed_back = true;
            }
            let text = if files.is_empty() {
                "the merge was clean".to_string()
            } else {
                format!("handed back with conflicts in {}", files.join(", "))
            };
            history(run, i, now, text);
            if run.state == RunState::Running {
                super::super::dispatch::launch_worker(run, i, start, now, fx);
            } else {
                run.tasks[i].ready_from = Some(start);
            }
        }
        OpResult::Failed { message } => {
            let text = format!("could not merge the lower stage into its worktree: {message}");
            block(run, i, BlockReason::Environment, text, now);
        }
        _ => {}
    }
}

/// A lost sync hand-back: the next running pass (`launch_ready`) starts it again.
pub(in crate::run::engine) fn hand_back_lost(run: &mut Run, i: usize, start: Option<String>) {
    if run.tasks[i].state == TaskState::Preparing {
        run.tasks[i].ready_from = start;
    }
}

/// Decision 51: a sync task merged into stage `n`, which now holds its `onto` and what
/// that head held; a newer lower head is due again.
pub(crate) fn sync_merged(run: &mut Run, n: u16, id: &str) {
    let Some(sync) = run.task(id).and_then(|t| t.sync.clone()) else {
        return;
    };
    // Milestone 9.2 decision 34: a base sync task brings the base, not a lower stage.
    if let Some(Some(FixOf::Base { base_sha, .. })) = run.task(id).map(|t| t.fixes.clone()) {
        return super::super::delivery::sync::task_merged(run, n, &base_sha);
    }
    if let Some(record) = stage_mut(run, n) {
        record.tasks_in.extend(sync.tasks);
        record.synced_from = Some(sync.onto);
    }
    run.propagate_due.insert(n);
}
