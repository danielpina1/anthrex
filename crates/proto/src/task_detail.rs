//! Milestone 9.0.5 decisions 2 to 4: one task's plan text and its worker's own account of
//! the work, sent only on request (`RunRequest::TaskDetail`), never in a snapshot, so
//! decision 16a of milestone 9 (no plan text on every push) still holds.

use serde::{Deserialize, Serialize};

/// The most characters a worker summary (`TaskDetailInfo.worker_summary`) or a round's
/// last message carries; a longer one is cut on a character boundary with `…`.
pub const WORKER_SUMMARY_MAX: usize = 1500;

/// The most characters `TaskInfo.activity` carries: one line, cut with `…`.
pub const ACTIVITY_MAX: usize = 160;

/// Where `TaskDetailInfo.worker_summary` came from (decision 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummarySource {
    /// The summary the worker passed to `task_done`.
    TaskDone,
    /// The last assistant text of the latest closed worker turn.
    LastMessage,
}

/// The answer to `RunRequest::TaskDetail`, carried (boxed) in `RunReply::TaskDetail`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskDetailInfo {
    pub run_id: String,
    pub task_id: String,
    pub brief: String,
    pub acceptance: Vec<String>,
    /// At most `WORKER_SUMMARY_MAX` characters, line breaks kept
    /// (`safe_text::multi_line`); `None` until the task has one.
    pub worker_summary: Option<String>,
    pub summary_source: Option<SummarySource>,
}
