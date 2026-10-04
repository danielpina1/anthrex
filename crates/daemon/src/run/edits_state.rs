//! Task-state predicates of the plan edits (decision 13), split from `edits.rs` for
//! AGENTS.md rule 8. Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (design decision 2).

use proto::{BlockReason, TaskKind, TaskState};

use super::model::Task;

/// The states in which a task has not started: route, size, test mode and dependencies
/// may change, and it may be split. A `paused(message)` task has started (milestone 9
/// decision 42c): its worker waits in its session.
pub(crate) fn not_started(task: &Task) -> bool {
    matches!(
        task.state,
        TaskState::Pending | TaskState::Queued | TaskState::Blocked
    ) && !is_paused(task)
}

/// The stage rule's "started" (ruling C-14(d)), which milestone 9.5's race and pair
/// share (review ruling I9): a task that is not [`not_started`], or one with a start
/// commit (it was dispatched and has a checkout).
pub(crate) fn has_started(task: &Task) -> bool {
    !not_started(task) || task.start_commit.is_some()
}

/// Milestone 9 decision 42c: `blocked(message_pause)`, shown as `paused(message)`.
pub(crate) fn is_paused(task: &Task) -> bool {
    task.state == TaskState::Blocked
        && task
            .block
            .as_ref()
            .is_some_and(|b| b.reason == BlockReason::MessagePause)
}

/// A task with a session or an engine operation to stop when it is cancelled.
pub(super) fn is_live(task: &Task) -> bool {
    matches!(
        task.state,
        TaskState::Preparing
            | TaskState::Working
            | TaskState::Proof
            | TaskState::Check
            | TaskState::Review
            | TaskState::MergeQueue
    ) || task.rounds.iter().any(|r| !r.ended)
}

/// A research or review task (decisions 35 and 36): a scout or a reviewer works it,
/// never a worker, so no worker message or refresh reaches it (M9.13a review, item 1).
pub(crate) fn is_reader(task: &Task) -> bool {
    matches!(task.spec.kind, TaskKind::Research | TaskKind::Review)
}

/// A task whose worker can take an amendment as a message.
pub(crate) fn has_live_worker(task: &Task) -> bool {
    task.state == TaskState::Working
        || task
            .rounds
            .iter()
            .any(|r| super::model::writes(task, r) && !r.ended)
}

fn block_label(reason: BlockReason) -> &'static str {
    match reason {
        BlockReason::MisSized => "mis_sized",
        BlockReason::Human => "human",
        BlockReason::Conflict => "conflict",
        BlockReason::DepCancelled => "dep_cancelled",
        BlockReason::Question => "question",
        BlockReason::Environment => "environment",
        BlockReason::MessagePause => "message_pause",
    }
}

/// `working`, `merged`, or `blocked(<reason>)`.
pub(crate) fn state_label(task: &Task) -> String {
    match (task.state, &task.block) {
        (TaskState::Blocked, Some(block)) => format!("blocked({})", block_label(block.reason)),
        (state, _) => state.label().to_string(),
    }
}
