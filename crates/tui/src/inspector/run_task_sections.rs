//! Milestone 9.0.7 decision 12 (spec §6.2): a task's inspection outcome first, as
//! OUTCOME, EVIDENCE (both `run_task_outcome`), INTENT (the brief, one line until `b`,
//! and what the task owns) and DETAIL (the phase in plain words, the latest worker
//! round, the live `now:` line, then every milestone 8c field). Nothing the 9.0.5 panel
//! showed is dropped (M9.0.5 decision 22's rule). Pure: it reads `App` and returns
//! sections; the panel sanitises, wraps and colours.

use super::run_format::{format_duration, reason_text};
use super::run_task_outcome::{evidence, outcome};
use super::{Field, Marks, Section, SectionField};
use crate::app::App;
use crate::app::task_detail::DetailState;
use crate::safe_text::one_line;
use crate::tree::{NodeKey, is_paused, round_label, task_held};
use proto::{AgentRole, AgentRoundInfo, DeciderSource, RunInfo, RunState, TaskInfo, TaskState};

fn plain(label: &'static str, value: impl Into<String>) -> SectionField {
    SectionField {
        label,
        note: None,
        value: value.into(),
        collapse: false,
        marks: Marks::None,
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
pub(super) fn first_line(text: &str) -> Option<&str> {
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

/// INTENT: the brief, cut to one wrapped line until `b` expands it, and `owns`.
fn intent(run: &RunInfo, task: &TaskInfo, app: &App) -> Section {
    let key = NodeKey::Task {
        run: run.run_id.clone(),
        id: task.id.clone(),
    };
    let brief = match app.task_detail_for(&run.run_id, &task.id) {
        Some(DetailState::Ready(detail)) => detail.brief.clone(),
        Some(DetailState::Failed(text)) => text.clone(),
        Some(DetailState::InFlight(_)) | None => "loading…".to_owned(),
    };
    let mut fields = vec![SectionField {
        collapse: !app.brief_expanded_for(&key),
        ..plain("brief", brief)
    }];
    if !task.owns.is_empty() {
        // One entry is one row's worth: a line break inside it must not forge another.
        let owns: Vec<String> = task.owns.iter().map(|o| one_line(o)).collect();
        fields.push(plain("owns", owns.join(", ")));
    }
    Section {
        title: "INTENT",
        fields,
    }
}

/// DETAIL's milestone 8c rows, in decision 12's order.
const DETAIL_ROWS: [&str; 10] = [
    "deps", "budget", "tries", "stage", "origin", "tier", "route", "messages", "notes", "history",
];

/// DETAIL: `phase` (the lifecycle words), `worker`, `now`, then milestone 8c's rows
/// from `flat` in `DETAIL_ROWS`' order (the pipeline, diff and review are OUTCOME's and
/// EVIDENCE's).
fn detail(run: &RunInfo, task: &TaskInfo, flat: &[Field], app: &App) -> Section {
    let mut fields = vec![plain("phase", stage_words(run, task))];
    if let Some(worker) = worker_line(task, app) {
        fields.push(plain("worker", worker));
    }
    if let Some(now) = now_line(task) {
        fields.push(plain("now", now));
    }
    for label in DETAIL_ROWS {
        if let Some(field) = flat.iter().find(|field| field.label == label) {
            fields.push(plain(field.label, field.value.clone()));
        }
    }
    Section {
        title: "DETAIL",
        fields,
    }
}

/// Decision 12's four sections, in order. `flat` is milestone 8c's field list for the
/// task, which DETAIL carries on.
pub(super) fn sections(run: &RunInfo, task: &TaskInfo, flat: &[Field], app: &App) -> Vec<Section> {
    let evidence = evidence(run, task, app);
    vec![
        outcome(run, task, app, &evidence),
        evidence,
        intent(run, task, app),
        detail(run, task, flat, app),
    ]
}
