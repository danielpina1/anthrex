//! The plan-edit log (milestone 8c decision 4): one record per `run edit` batch, for
//! the run view's inspector; since milestone 9 (decision 40), every source's accepted
//! batches and the orchestrator's and sub-planners' rejected ones. Pure — no `std::fs`, `std::process`,
//! `std::thread`, `tokio` or `std::time::SystemTime` (design decision 2). Its own
//! module, not `edits.rs`, which applies the edits.

use proto::PlanEdit;
use serde::{Deserialize, Serialize};

use super::model::Run;
use super::orch::EditSource;

/// Decision 40: how a batch ended. An accepted batch names a `message`'s resolved
/// recipients (TT §12.5; none for any other edit); a rejected one its first error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOutcome {
    Accepted { recipients: Vec<String> },
    Rejected { error: String },
}

impl EditOutcome {
    /// An accepted batch with no `message` edit.
    pub fn accepted() -> EditOutcome {
        EditOutcome::Accepted {
            recipients: Vec::new(),
        }
    }
}

/// One accepted batch: when, and what it did. Its own struct, not `TaskEvent`, so
/// milestone 9 can add the edit's `source` with `#[serde(default)]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEditRecord {
    pub at: u64,
    pub text: String,
    /// Milestone 9 decision 40: `user`, `orchestrator` or `planner:<e>`; whether the
    /// batch was accepted, a rejected one's first error, and a `message`'s recipients.
    /// Absent from an older run: an accepted batch of the user's.
    #[serde(default = "user_source")]
    pub source: String,
    #[serde(default = "accepted_by_default")]
    pub accepted: bool,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub recipients: Vec<String>,
}

fn user_source() -> String {
    "user".to_string()
}

fn accepted_by_default() -> bool {
    true
}

/// Records a run keeps; older ones are dropped.
pub const PLAN_EDITS_KEPT: usize = 50;

/// The longest record text, in characters, before `…` (review M3).
pub const DESCRIBE_MAX_CHARS: usize = 300;

/// The batch in a few words, one per edit, joined with `, `: `add t9, dep t4 on t2`.
/// The match is exhaustive, so an edit a later milestone adds must be described.
/// Control characters become spaces, and the text is cut to [`DESCRIBE_MAX_CHARS`]
/// characters plus `…` (review M3: every snapshot push carries these records).
pub fn describe(edits: &[PlanEdit]) -> String {
    let mut text = String::new();
    for (n, part) in edits.iter().map(describe_one).enumerate() {
        if n > 0 {
            text.push_str(", ");
        }
        text.push_str(&part);
        if text.chars().count() > DESCRIBE_MAX_CHARS {
            break;
        }
    }
    let mut out: String = text
        .chars()
        .take(DESCRIBE_MAX_CHARS)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if text.chars().count() > DESCRIBE_MAX_CHARS {
        out.push('…');
    }
    out
}

fn kind_label(kind: proto::MessageKind) -> &'static str {
    match kind {
        proto::MessageKind::Info => "info",
        proto::MessageKind::Change => "change",
        proto::MessageKind::StopAndWait => "stop_and_wait",
    }
}

fn describe_one(edit: &PlanEdit) -> String {
    match edit {
        PlanEdit::AddTask { task } => format!("add {}", task.id),
        PlanEdit::SplitTask { task_id, .. } => format!("split {task_id}"),
        PlanEdit::CancelTask { task_id } => format!("cancel {task_id}"),
        PlanEdit::AmendTask { task_id, .. } => format!("amend {task_id}"),
        PlanEdit::AddDep { task_id, dep } => format!("dep {task_id} on {dep}"),
        PlanEdit::Answer { task_id, .. } => format!("answer {task_id}"),
        PlanEdit::Pause => "pause".to_string(),
        PlanEdit::Resume => "resume".to_string(),
        PlanEdit::Finish => "finish".to_string(),
        // Decision 40's wording: `message t1,t2 (change)`.
        PlanEdit::Message { to, kind, .. } => format!("message {to} ({})", kind_label(*kind)),
        PlanEdit::Refresh { task_id } => format!("refresh {task_id}"),
    }
}

/// The longest stored error of a rejected batch, in characters, `…` included.
pub const ERROR_MAX_CHARS: usize = 300;

/// M9.9 review fixes, M4: a rejected batch's error, one line of at most
/// [`ERROR_MAX_CHARS`] characters.
fn clip_error(error: &str) -> String {
    let one_line = |c: char| if c.is_control() { ' ' } else { c };
    if error.chars().count() <= ERROR_MAX_CHARS {
        return error.chars().map(one_line).collect();
    }
    let mut out: String = error
        .chars()
        .take(ERROR_MAX_CHARS - 1)
        .map(one_line)
        .collect();
    out.push('…');
    out
}

/// Records a batch from `source` at `now` with its `outcome` (decision 40), keeping the
/// last [`PLAN_EDITS_KEPT`]; an accepted batch counts as an edit after approval when
/// the plan has been approved.
pub fn record(
    run: &mut Run,
    edits: &[PlanEdit],
    now: u64,
    source: &EditSource,
    outcome: EditOutcome,
) {
    let (accepted, error, recipients) = match outcome {
        EditOutcome::Accepted { recipients } => (true, None, recipients),
        EditOutcome::Rejected { error } => (false, Some(clip_error(&error)), Vec::new()),
    };
    run.plan_edits.push(PlanEditRecord {
        at: now,
        text: describe(edits),
        source: source.label(),
        accepted,
        error,
        recipients,
    });
    // M9.9 review fixes, M4: a full log drops its oldest rejected batch first, so a
    // stream of refusals never pushes the accepted edits out.
    while run.plan_edits.len() > PLAN_EDITS_KEPT {
        let oldest = run.plan_edits.iter().position(|r| !r.accepted).unwrap_or(0);
        run.plan_edits.remove(oldest);
    }
    if accepted && run.approved_at.is_some() {
        run.plan_edits_since_approval = run.plan_edits_since_approval.saturating_add(1);
    }
}

#[cfg(test)]
#[path = "edit_log_tests.rs"]
mod tests;
