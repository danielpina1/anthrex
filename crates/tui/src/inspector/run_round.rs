//! The run view's inspection of an agent round (milestone 8c decisions 29–31a,
//! Interfaces "Inspector contents, exact": Agent round). Pure, as `run.rs` is.

use super::run_format::{
    billable, clean, counts_text, effort_text, finding_text, format_duration, format_tokens,
    kind_glyph, most_severe, rank, rows, session_text, strength_text,
};
use super::{Field, Inspection, field};
use crate::app::App;
use crate::tree::{DisplayRound, RowKind, display_rounds, round_label};
use proto::{AgentRole, Route, RunInfo, Severity, TaskInfo};

/// An agent round (Interfaces "Agent round").
pub(crate) fn round_inspection(
    run: &RunInfo,
    task: &TaskInfo,
    round: &DisplayRound<'_>,
    app: &App,
) -> Inspection {
    let info = round.info;
    let live = round.ended_at.is_none();
    let limited = live && app.rate_limited(info);
    let status = if limited {
        "rate-limited"
    } else if live {
        round
            .window
            .map_or("starting", |window| window.status.label())
    } else {
        "finished"
    };
    let duration = match round.ended_at {
        None => app.run_age(round.started_at),
        Some(ended) => ended.saturating_sub(round.started_at),
    };
    let right = format!("{status} · {} · {}", format_duration(duration), task.id);
    let label = round_label(info.role, info.session, round.number);
    let route = &info.route;
    let name = format!(
        "{label}  {} · {} · {}",
        route.runtime.label(),
        strength_text(route.strength),
        effort_text(route.effort)
    );
    let fields = match info.role {
        AgentRole::Reviewer => reviewer_fields(task, round, app),
        AgentRole::Worker | AgentRole::Orchestrator | AgentRole::Scout | AgentRole::Planner => {
            worker_fields(task, round, limited, app)
        }
    };
    let glyph = kind_glyph(
        RowKind::AgentRound {
            run,
            task,
            round: round.clone(),
        },
        app,
    );
    rows(glyph, name, right, fields)
}

fn worker_fields(
    task: &TaskInfo,
    round: &DisplayRound<'_>,
    limited: bool,
    app: &App,
) -> Vec<Field> {
    let info = round.info;
    let doing = if round.ended_at.is_some() {
        "finished".to_owned()
    } else {
        let tool = round.window.and_then(|window| window.tool.as_deref());
        let mut text = match tool {
            _ if limited => "rate-limited · waiting out the runtime's retry".to_owned(),
            Some(tool) => format!("last tool: {}", clean(tool)),
            None if info.turn_open => "thinking".to_owned(),
            None => "waiting for its next turn".to_owned(),
        };
        if info.open_subagents > 0 {
            text.push_str(&format!(" · {} sub-agents open", info.open_subagents));
        }
        text
    };
    let mut fields = vec![field("doing", doing)];
    if round.last {
        let mut activity = format!(
            "turns {} · tool calls {} · tokens {}",
            info.turns,
            info.tool_calls,
            format_tokens(billable(&info.usage))
        );
        if info.denials > 0 {
            activity.push_str(&format!(" · {} denied", info.denials));
        }
        fields.push(field("activity", activity));
    }
    if (round.number > 1 || info.session > 1)
        && let Some(fixing) = fixing_text(task, round.started_at)
    {
        fields.push(field("fixing", fixing));
    }
    let place = format!("worktree {}", clean(&task.branch));
    fields.push(field("session", session_text(info.window_id, &place, app)));
    fields
}

/// The latest gate failure before `start`: a blocking review's most severe critical
/// or important finding, a failed check, or a failed proof. A review is timed by its
/// reviewer round's start, strictly before `start` (ruling I1): the daemon sends the
/// worker back when the reviewer submits, and ends the reviewer's round only when its
/// process exits, after the send-back. A check or proof is timed by its record's `at`,
/// at or before `start`: the daemon sends back at or after that `at`.
fn fixing_text(task: &TaskInfo, start: u64) -> Option<String> {
    let mut failures: Vec<(u64, String)> = Vec::new();
    for review in task
        .reviews
        .iter()
        .filter(|r| r.blocking && r.verdict.is_some())
    {
        let started = task
            .rounds
            .iter()
            .find(|r| r.role == AgentRole::Reviewer && r.round == review.round)
            .map(|r| r.started_at)
            .filter(|at| *at < start);
        let worst = review
            .findings
            .iter()
            .filter(|f| f.severity != Severity::Minor)
            .min_by_key(|f| rank(f.severity));
        if let (Some(at), Some(worst)) = (started, worst) {
            failures.push((at, finding_text(worst)));
        }
    }
    if let Some(check) = task.last_check.as_ref().filter(|check| !check.ok) {
        let first = |text: &str| {
            text.lines()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .map(str::to_owned)
        };
        let last = |text: &str| {
            text.lines()
                .map(str::trim)
                .rfind(|l| !l.is_empty())
                .map(str::to_owned)
        };
        let line = check
            .decider_summary
            .as_deref()
            .and_then(first)
            .or_else(|| last(&check.summary));
        let text = line.map_or_else(
            || "check failed".to_owned(),
            |line| format!("check failed: {}", clean(&line)),
        );
        failures.push((check.at, text));
    }
    if let Some(proof) = task.last_proof.as_ref().filter(|proof| !proof.ok) {
        failures.push((proof.at, "test proof failed".to_owned()));
    }
    failures
        .into_iter()
        .filter(|(at, _)| *at <= start)
        .max_by_key(|(at, _)| *at)
        .map(|(_, text)| text)
}

fn reviewer_fields(task: &TaskInfo, round: &DisplayRound<'_>, app: &App) -> Vec<Field> {
    let info = round.info;
    let workers = display_rounds(task, &app.windows);
    let judged = workers
        .iter()
        .filter(|worker| worker.info.role == AgentRole::Worker)
        .filter(|worker| worker.started_at < round.started_at)
        .max_by_key(|worker| (worker.started_at, worker.number));
    let mut fields = Vec::new();
    if let Some(judged) = judged {
        let label = round_label(judged.info.role, judged.info.session, judged.number);
        let route: &Route = &judged.info.route;
        fields.push(field(
            "judging",
            format!(
                "{label} · {} · {}",
                route.runtime.label(),
                strength_text(route.strength)
            ),
        ));
    }
    let author = judged.map_or(task.route.strength, |judged| judged.info.route.strength);
    fields.push(field(
        "strength",
        format!(
            "{} vs author {}",
            strength_text(info.route.strength),
            strength_text(author)
        ),
    ));
    let of_round = || {
        task.reviews
            .iter()
            .filter(|review| review.round == info.round)
    };
    let review = of_round()
        .find(|review| review.verdict.is_some())
        .or_else(|| of_round().next());
    let verdict = match review.and_then(|review| review.verdict.map(|v| (v, review.blocking))) {
        Some((proto::Verdict::Approve, _)) => "approve",
        Some((proto::Verdict::Changes, true)) => "changes (blocking)",
        Some((proto::Verdict::Changes, false)) => "changes (minor only, counts as approval)",
        None if round.ended_at.is_none() => "reviewing",
        None => "none — the round ended without one",
    };
    fields.push(field("verdict", verdict));
    let findings = review
        .map(|review| review.findings.as_slice())
        .unwrap_or_default();
    let findings = match most_severe(findings) {
        None => "none".to_owned(),
        Some(worst) => format!("{} · {}", counts_text(findings), finding_text(worst)),
    };
    fields.push(field("findings", findings));
    fields.push(field(
        "session",
        session_text(info.window_id, "read-only review worktree", app),
    ));
    fields
}
