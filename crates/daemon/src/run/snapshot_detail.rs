//! Milestone 9.0.5: what the snapshot and the task detail request derive from a task's
//! rounds: the live round's activity line (decision 3) and a task's detail, its plan
//! text and its worker's own summary (decisions 4 and 7). Pure, like `snapshot.rs`:
//! everything here reads memory the caller already holds.

use proto::safe_text::multi_line;
use proto::{AgentRole, SummarySource, TaskDetailInfo, WORKER_SUMMARY_MAX};

use crate::run::model::{Run, Task};

/// Decision 7: `task_id`'s brief, acceptance and worker summary, whatever the run's
/// state (decision 16a keeps them out of the snapshot, not out of this reply); `None`
/// when the run has no such task.
pub fn task_detail(run: &Run, task_id: &str) -> Option<TaskDetailInfo> {
    let task = run.task(task_id)?;
    let (worker_summary, summary_source) = match worker_summary(task) {
        Some((text, source)) => (Some(text), Some(source)),
        None => (None, None),
    };
    Some(TaskDetailInfo {
        run_id: run.id.clone(),
        task_id: task_id.to_string(),
        brief: task.spec.brief.clone(),
        acceptance: task.spec.acceptance.clone(),
        worker_summary,
        summary_source,
    })
}

/// Decision 4: the accepted `task_done` summary when it is not blank and its session is
/// the task's latest worker session (ruling D-1; a claim with no session counts as
/// current), else the last message of the latest worker round whose turn is closed,
/// sanitised (line breaks kept) and capped at [`WORKER_SUMMARY_MAX`] characters with `…`.
fn worker_summary(task: &Task) -> Option<(String, SummarySource)> {
    let clean = |text: &str| {
        let text = multi_line(text);
        (!text.trim().is_empty()).then(|| cut(&text, WORKER_SUMMARY_MAX))
    };
    let workers = || {
        task.rounds
            .iter()
            .rev()
            .filter(|r| r.role == AgentRole::Worker)
    };
    let latest = workers().next().map(|r| r.session);
    let current = |session: Option<u32>| session.is_none() || latest.is_none() || session == latest;
    if let Some(summary) = task
        .done
        .as_ref()
        .filter(|d| current(d.session))
        .and_then(|d| clean(&d.summary))
    {
        return Some((summary, SummarySource::TaskDone));
    }
    workers()
        .find(|r| !r.turn_open || r.ended)
        .and_then(|r| r.last_text.as_deref())
        .and_then(clean)
        .map(|text| (text, SummarySource::LastMessage))
}

/// The activity of the task's live round (the latest one not ended), `None` otherwise,
/// so a terminal run's tasks carry nothing new (decision 2).
pub(crate) fn live_activity(task: &Task) -> Option<String> {
    task.rounds
        .iter()
        .rev()
        .find(|r| !r.ended)
        .and_then(|r| r.activity.clone())
}

/// `text` cut to at most `max` characters, on a character boundary, with `…` after the
/// kept part when anything was cut.
pub(crate) fn cut(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        None => text.to_string(),
        Some((end, _)) => format!("{}…", &text[..end]),
    }
}

#[cfg(test)]
#[path = "snapshot_detail_tests.rs"]
mod tests;
