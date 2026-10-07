//! The run view's inspection of an agent round (milestone 8c decisions 29–31a,
//! Interfaces "Inspector contents, exact": Agent round). Pure, as `run.rs` is.

use super::run_format::{
    billable, clean, counts_text, effort_text, finding_text, format_duration, format_tokens,
    kind_glyph, local_hhmm, most_severe, rank, rows, session_text, strength_text,
};
use super::run_patterns;
use super::{Field, Inspection, field};
use crate::app::App;
use crate::tree::{DisplayRound, RowKind, display_rounds, round_label_with_lane};
use proto::{AgentRole, RaceLane, Route, RunInfo, Severity, TaskInfo};

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
    let status = match info.failed_until {
        // Milestone 9.5 decision 43: a failed turn waiting for its retry (the glyph's
        // `App::round_waits` predicate).
        Some(until) if live && !limited => failed_text(info.failed_error.as_deref(), until, app),
        _ if limited => "rate-limited".to_owned(),
        _ if live => round
            .window
            .map_or("starting", |window| window.status.label())
            .to_owned(),
        _ => "finished".to_owned(),
    };
    let duration = match round.ended_at {
        None => app.run_age(round.started_at),
        Some(ended) => ended.saturating_sub(round.started_at),
    };
    let right = format!("{status} · {} · {}", format_duration(duration), task.id);
    let label = round_label_with_lane(info.role, info.lane, info.session, round.number);
    let route = &info.route;
    let name = format!(
        "{label}  {} · {} · {}",
        route.runtime.label(),
        strength_text(route.strength),
        effort_text(route.effort.clone())
    );
    let fields = match info.role {
        AgentRole::Reviewer => reviewer_fields(task, round, app),
        AgentRole::Worker | AgentRole::Orchestrator | AgentRole::Scout | AgentRole::Planner => {
            worker_fields(task, round, limited, app)
        }
        // Milestone 9.5 decision 29: the lane's standing, or the test, after `doing`.
        AgentRole::Racer | AgentRole::TestWriter => {
            let mut fields = worker_fields(task, round, limited, app);
            let pattern = match info.role {
                AgentRole::Racer => run_patterns::race_field(task, info.lane).map(|t| ("race", t)),
                _ => run_patterns::test_field(task, app.palette().ascii).map(|t| ("test", t)),
            };
            if let Some((label, text)) = pattern {
                fields.insert(1, field(label, text));
            }
            fields
        }
        // A decider has no rounds (decision 43): nothing of a worker's to show.
        // Nor does a design agent (milestone 9.6).
        AgentRole::Decider | AgentRole::Brainstormer | AgentRole::DocReviewer => Vec::new(),
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

/// Decision 43: the most of a failed turn's error the round's status shows.
const FAILED_ERROR_CHARS: usize = 80;

/// Decision 43: `failed: <error> · retries <hh:mm>`, the error one line and cut to
/// [`FAILED_ERROR_CHARS`] characters; `failed` alone without an error, and no retry
/// time once it has passed (a paused run's round keeps its failed turn).
fn failed_text(error: Option<&str>, until: u64, app: &App) -> String {
    let mut text = "failed".to_owned();
    if let Some(error) = error {
        let head: String = error.chars().take(FAILED_ERROR_CHARS).collect();
        text.push_str(&format!(": {}", crate::safe_text::one_line(&head)));
        if error.chars().nth(FAILED_ERROR_CHARS).is_some() {
            text.push('…');
        }
    }
    if until > app.run_now() {
        text.push_str(&format!(
            " · retries {}",
            local_hhmm(until, app.utc_offset_secs)
        ));
    }
    text
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
    } else if let Some(activity) = task.activity.as_deref()
        && super::run_task_sections::live_round(task).is_some_and(|live| std::ptr::eq(live, info))
    {
        // Milestone 9.0.5 decision 26: the live round's latest action, on that round
        // only (a worker left open under a live review does not take the reviewer's).
        format!("now: {}", clean(activity))
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
        && let Some(fixing) = fixing_text(task, info.lane, round.started_at)
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
/// at or before `start`: the daemon sends back at or after that `at`. A racer's are
/// its own lane's reviews, any other round's those the task counts (ruling T20-2).
fn fixing_text(task: &TaskInfo, lane: Option<RaceLane>, start: u64) -> Option<String> {
    let mut failures: Vec<(u64, String)> = Vec::new();
    let reviews = match lane {
        Some(_) => task.reviews.iter().filter(|r| r.lane == lane).collect(),
        None => run_patterns::counted_reviews(task),
    };
    for review in reviews
        .into_iter()
        .filter(|r| r.blocking && r.verdict.is_some())
    {
        let started = task
            .rounds
            .iter()
            .find(|r| {
                r.role == AgentRole::Reviewer && r.round == review.round && r.lane == review.lane
            })
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
    // Task 20b's carry: a racer's own lane's check and proof only.
    let ours = |of: Option<RaceLane>| lane.is_none() || of == lane;
    if let Some(check) = (task.last_check.as_ref()).filter(|check| !check.ok && ours(check.lane)) {
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
    if let Some(proof) = (task.last_proof.as_ref()).filter(|proof| !proof.ok && ours(proof.lane)) {
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
    // Milestone 9.5: a lane's reviewer judges that lane's racer; any other the latest
    // worker or racer, as the daemon finds the author (`engine/review.rs`).
    let judged = workers
        .iter()
        .filter(|worker| match info.lane {
            Some(lane) => worker.info.role == AgentRole::Racer && worker.info.lane == Some(lane),
            None => matches!(worker.info.role, AgentRole::Worker | AgentRole::Racer),
        })
        .filter(|worker| worker.started_at < round.started_at)
        .max_by_key(|worker| (worker.started_at, worker.number));
    let mut fields = Vec::new();
    if let Some(judged) = judged {
        let who = judged.info;
        let label = round_label_with_lane(who.role, who.lane, who.session, judged.number);
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
    // Ruling T20-2 (I1): a review is its round's and its lane's.
    let of_round = || {
        (task.reviews.iter())
            .filter(|review| review.round == info.round && review.lane == info.lane)
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
