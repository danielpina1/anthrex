//! The plan-edit log (milestone 8c decision 4): one record per accepted `run edit`
//! batch, for the run view's inspector. Pure — no `std::fs`, `std::process`,
//! `std::thread`, `tokio` or `std::time::SystemTime` (design decision 2). Its own
//! module, not `edits.rs`, which applies the edits.

use proto::PlanEdit;
use serde::{Deserialize, Serialize};

use super::model::Run;

/// One accepted batch: when, and what it did. Its own struct, not `TaskEvent`, so
/// milestone 9 can add the edit's `source` with `#[serde(default)]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanEditRecord {
    pub at: u64,
    pub text: String,
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
        PlanEdit::Message { to, .. } => format!("message to {to}"),
        PlanEdit::Refresh { task_id } => format!("refresh {task_id}"),
    }
}

/// Records an accepted batch at `now`, keeping the last [`PLAN_EDITS_KEPT`], and counts
/// it as an edit after approval when the plan has been approved.
pub fn record(run: &mut Run, edits: &[PlanEdit], now: u64) {
    run.plan_edits.push(PlanEditRecord {
        at: now,
        text: describe(edits),
    });
    if run.plan_edits.len() > PLAN_EDITS_KEPT {
        let excess = run.plan_edits.len() - PLAN_EDITS_KEPT;
        run.plan_edits.drain(..excess);
    }
    if run.approved_at.is_some() {
        run.plan_edits_since_approval = run.plan_edits_since_approval.saturating_add(1);
    }
}

#[cfg(test)]
#[path = "edit_log_tests.rs"]
mod tests;
