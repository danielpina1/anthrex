//! Task-state predicates of the plan edits (decision 13), split from `edits.rs` for
//! AGENTS.md rule 8. Pure — no `std::fs`, `std::process`, `std::thread`, `tokio` or
//! `std::time::SystemTime` (design decision 2).

use proto::{AgentRole, BlockReason, TaskState};

use super::model::Task;

/// The states in which a task has not started: route, size, test mode and dependencies
/// may change, and it may be split.
pub(crate) fn not_started(state: TaskState) -> bool {
    matches!(
        state,
        TaskState::Pending | TaskState::Queued | TaskState::Blocked
    )
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

/// A task whose worker can take an amendment as a message.
pub(crate) fn has_live_worker(task: &Task) -> bool {
    task.state == TaskState::Working
        || task
            .rounds
            .iter()
            .any(|r| r.role == AgentRole::Worker && !r.ended)
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
