//! Milestone 9.0.5 decisions 22–24: a task's inspection as GOAL, STATUS and RESULT.
//! GOAL reads the on-demand detail (`app::task_detail`); STATUS the snapshot (the
//! stage in plain words, the latest worker round, the live `now:` line, the last
//! check and the review, then every milestone 8c field); RESULT the worker's summary,
//! the verdict, the diff and the merge commit. Nothing the flat list showed is
//! dropped. Pure: it reads `App` and returns sections; the panel sanitises and wraps.

use super::run_format::{format_duration, reason_text};
use super::{Field, Section, SectionField};
use crate::app::App;
use crate::app::task_detail::DetailState;
use crate::safe_text::{multi_line, one_line};
use crate::tree::{NodeKey, is_paused, round_label, task_held};
use proto::{
    AgentRole, AgentRoundInfo, DeciderSource, RunInfo, RunState, SummarySource, TaskInfo,
    TaskState, Verdict,
};

fn plain(label: &'static str, value: impl Into<String>) -> SectionField {
    SectionField {
        label,
        note: None,
        value: value.into(),
        collapse: false,
    }
}

/// Interfaces "Stage words".
pub(super) fn stage_words(run: &RunInfo, task: &TaskInfo) -> String {
    if run.state == RunState::AwaitingApproval {
        return "planned, not started".to_owned();
    }
    let stage = if is_paused(task) {
        "paused by a message".to_owned()
    } else {
        match task.state {
            TaskState::Pending => "waiting for its dependencies".to_owned(),
            TaskState::Queued => "queued for a worker".to_owned(),
            TaskState::Preparing => "preparing its worktree".to_owned(),
            TaskState::Working => "worker is working".to_owned(),
            TaskState::Proof => "running the test proof".to_owned(),
            TaskState::Check => "running the check".to_owned(),
            TaskState::Review => "in review".to_owned(),
            TaskState::MergeQueue => "waiting to merge".to_owned(),
            TaskState::Merged => "merged".to_owned(),
            TaskState::Cancelled => "cancelled".to_owned(),
            TaskState::Reported => "report delivered".to_owned(),
            TaskState::Blocked => {
                let mut text = match &task.block {
                    Some(block) => format!("blocked: {}", reason_text(block.reason)),
                    None => "blocked".to_owned(),
                };
                if task.block_source == Some(DeciderSource::Fallback) {
                    text.push_str(" (fallback)");
                }
                text
            }
        }
    };
    if task_held(run, task) {
        format!("{stage} · held")
    } else {
        stage
    }
}

/// The latest round of `role`, by start.
fn latest(task: &TaskInfo, role: AgentRole) -> Option<&AgentRoundInfo> {
    task.rounds
        .iter()
        .filter(|round| round.role == role)
        .max_by_key(|round| (round.started_at, round.session, round.round))
}

/// `<round label> · <runtime> <model> · <elapsed> · <n> tool calls`, the elapsed time
/// frozen at the round's end.
pub(super) fn worker_line(task: &TaskInfo, app: &App) -> Option<String> {
    let round = latest(task, AgentRole::Worker)?;
    let label = round_label(round.role, round.session, round.round);
    let elapsed = match round.ended_at {
        Some(ended) => ended.saturating_sub(round.started_at),
        None => app.run_age(round.started_at),
    };
    let model = one_line(&round.route.model);
    let who = if model.trim().is_empty() {
        round.route.runtime.label().to_owned()
    } else {
        format!("{} {model}", round.route.runtime.label())
    };
    Some(format!(
        "{label} · {who} · {} · {} tool calls",
        format_duration(elapsed),
        round.tool_calls
    ))
}

/// The task's live round: the latest-started one without an end. `TaskInfo.activity`
/// is this round's latest action (decision 26).
pub(super) fn live_round(task: &TaskInfo) -> Option<&AgentRoundInfo> {
    task.rounds
        .iter()
        .filter(|round| round.ended_at.is_none())
        .max_by_key(|round| round.started_at)
}

/// `TaskInfo.activity`, prefixed `reviewer: ` when the live round is a reviewer's.
pub(super) fn now_line(task: &TaskInfo) -> Option<String> {
    let activity = task.activity.as_deref()?;
    let prefix = match live_round(task).map(|round| round.role) {
        Some(AgentRole::Reviewer) => "reviewer: ",
        _ => "",
    };
    Some(format!("{prefix}{}", one_line(activity)))
}

/// The first non-blank line of `text`.
fn first_line(text: &str) -> Option<&str> {
    text.lines().map(str::trim).find(|line| !line.is_empty())
}

/// Decision 24.
pub(super) fn check_line(run: &RunInfo, task: &TaskInfo) -> String {
    if run.unverified {
        return "–".to_owned();
    }
    let Some(check) = &task.last_check else {
        return "not yet".to_owned();
    };
    let mut text = if check.ok {
        "✓ passed".to_owned()
    } else if check.timed_out {
        "✗ timed out".to_owned()
    } else {
        "✗ failed".to_owned()
    };
    let summary = check
        .decider_summary
        .as_deref()
        .and_then(first_line)
        .or_else(|| first_line(&check.summary));
    if let Some(line) = summary {
        text.push_str(&format!(" · {}", one_line(line)));
    }
    match task.state {
        TaskState::Working if !check.ok => text.push_str(&format!(
            " → bounced (check {}/{})",
            task.bounces.check, run.max_bounces
        )),
        TaskState::Blocked => text.push_str(" → blocked"),
        TaskState::MergeQueue | TaskState::Merged if check.ok => text.push_str(" → merge queue"),
        _ => {}
    }
    text
}

/// STATUS's `review`.
fn review_line(task: &TaskInfo, flat: &[Field]) -> String {
    if task.review_route.is_none() {
        return "–".to_owned();
    }
    if task.state == TaskState::Review {
        return format!("in review · round {}", task.reviews.len());
    }
    old(flat, "review").unwrap_or_else(|| "not yet".to_owned())
}

fn old(flat: &[Field], label: &str) -> Option<String> {
    flat.iter()
        .find(|field| field.label == label)
        .map(|field| field.value.clone())
}

fn goal(run: &RunInfo, task: &TaskInfo, app: &App) -> Section {
    let key = NodeKey::Task {
        run: run.run_id.clone(),
        id: task.id.clone(),
    };
    let mut fields = Vec::new();
    let (brief, acceptance) = match app.task_detail_for(&run.run_id, &task.id) {
        Some(DetailState::Ready(detail)) => (detail.brief.clone(), detail.acceptance.clone()),
        Some(DetailState::Failed(text)) => (text.clone(), Vec::new()),
        Some(DetailState::InFlight(_)) | None => ("loading…".to_owned(), Vec::new()),
    };
    fields.push(SectionField {
        label: "brief",
        note: None,
        value: brief,
        collapse: !app.brief_expanded_for(&key),
    });
    if !task.owns.is_empty() {
        let owns: Vec<String> = task.owns.iter().map(|o| one_line(o)).collect();
        fields.push(plain("owns", owns.join(", ")));
    }
    if !acceptance.is_empty() {
        // One criterion is one row: a line break inside it must not forge another.
        let lines: Vec<String> = acceptance
            .iter()
            .map(|c| format!("☐ {}", one_line(c)))
            .collect();
        fields.push(plain("done when", lines.join("\n")));
    }
    Section {
        title: "GOAL",
        fields,
    }
}

fn status(run: &RunInfo, task: &TaskInfo, flat: &[Field], app: &App) -> Section {
    let mut fields = vec![plain("stage", stage_words(run, task))];
    if let Some(worker) = worker_line(task, app) {
        fields.push(plain("worker", worker));
    }
    if let Some(now) = now_line(task) {
        fields.push(plain("now", now));
    }
    fields.push(plain("check", check_line(run, task)));
    fields.push(plain("review", review_line(task, flat)));
    for field in flat {
        if !matches!(field.label, "diff" | "review") {
            fields.push(plain(field.label, field.value.clone()));
        }
    }
    Section {
        title: "STATUS",
        fields,
    }
}

fn result(run: &RunInfo, task: &TaskInfo, flat: &[Field], app: &App) -> Section {
    let mut fields = Vec::new();
    if let Some(DetailState::Ready(detail)) = app.task_detail_for(&run.run_id, &task.id)
        && let Some(summary) = &detail.worker_summary
    {
        let note = match detail.summary_source {
            Some(SummarySource::TaskDone) => Some("task_done"),
            Some(SummarySource::LastMessage) => Some("last message"),
            None => None,
        };
        fields.push(SectionField {
            label: "summary",
            note,
            value: summary.clone(),
            collapse: false,
        });
    }
    if let Some(review) = task.reviews.iter().rev().find(|r| r.verdict.is_some()) {
        let word = match review.verdict {
            Some(Verdict::Approve) => "approve",
            _ => "changes",
        };
        let text = match first_line(&review.summary) {
            Some(line) => format!("{word} · {}", one_line(line)),
            None => word.to_owned(),
        };
        fields.push(plain("verdict", text));
    }
    if let Some(diff) = old(flat, "diff") {
        fields.push(plain("diff", diff));
    }
    if let Some(commit) = &task.merge_commit {
        let mut text: String = one_line(commit).chars().take(7).collect();
        if let Some(reason) = &task.merged_without_approval {
            text.push_str(&format!(" · without approval: {}", one_line(reason)));
        }
        fields.push(plain("merged", text));
    }
    if fields.is_empty() {
        fields.push(plain("", "nothing yet"));
    }
    Section {
        title: "RESULT",
        fields,
    }
}

/// Decision 22's three sections. `flat` is milestone 8c's field list for the task,
/// which STATUS carries on (and RESULT takes its `diff` from).
pub(super) fn sections(run: &RunInfo, task: &TaskInfo, flat: &[Field], app: &App) -> Vec<Section> {
    let result = result(run, task, flat, app);
    let mut status = status(run, task, flat, app);
    if let Some(line) = outcome_line(task, &result) {
        status.fields.insert(0, plain("result", line));
    }
    vec![goal(run, task, app), status, result]
}

/// Ruling D-2: a merged or reported task's outcome, for the head of STATUS, so it
/// shows without scrolling. The first line of the worker's summary, else of RESULT's
/// first field (the detail may not have landed); `None` while the task is unfinished
/// or RESULT reads `nothing yet`.
fn outcome_line(task: &TaskInfo, result: &Section) -> Option<String> {
    if !matches!(task.state, TaskState::Merged | TaskState::Reported) {
        return None;
    }
    let first = result
        .fields
        .first()
        .filter(|field| !field.label.is_empty())?;
    first_line(&multi_line(&first.value)).map(one_line)
}
