//! The run view's inspection of a task (milestone 8c decisions 29–31a, Interfaces
//! "Inspector contents, exact": Task). Pure, as `run.rs` is.

use super::run_format::{
    clean, counts_text, effort_text, format_tokens, kind_glyph, local_hhmm, location, most_severe,
    progress_bar, reason_text, rows, size_letter, strength_text, test_mode_text,
};
use super::{Inspection, field};
use crate::app::App;
use crate::theme;
use crate::tree::RowKind;
use proto::{DeciderSource, ReviewInfo, RunInfo, RunState, TaskInfo, TaskState, TestMode};

/// The history entries shown: the snapshot sends at most this many, newest first, and
/// the view never shows more whatever it is sent.
const HISTORY_SHOWN: usize = 10;
/// A task (Interfaces "Task").
pub(crate) fn task_inspection(run: &RunInfo, task: &TaskInfo, app: &App) -> Inspection {
    let right = format!(
        "{} · {} · {}",
        size_letter(task.size),
        test_mode_text(task.test_mode),
        stage_text(run, task)
    );
    let mut fields = vec![
        field("stages", stages_text(run, task)),
        field("route", route_text(task)),
    ];
    if let Some(deps) = deps_text(run, task, app) {
        fields.push(field("deps", deps));
    }
    fields.push(field("budget", budget_text(task)));
    fields.push(field("tries", tries_text(run, task)));
    if let Some(diff) = diff_text(task) {
        fields.push(field("diff", diff));
    }
    if let Some(review) = review_text(task) {
        fields.push(field("review", review));
    }
    if !task.history.is_empty() {
        let history: Vec<String> = task
            .history
            .iter()
            .take(HISTORY_SHOWN)
            .map(|event| {
                let at = local_hhmm(event.at, app.utc_offset_secs);
                format!("{at} {}", clean(&event.text))
            })
            .collect();
        fields.push(field("history", history.join(" · ")));
    }
    let glyph = kind_glyph(RowKind::Task { run, task }, app);
    rows(
        glyph,
        format!("{}  {}", task.id, clean(&task.title)),
        right,
        fields,
    )
}

fn stage_text(run: &RunInfo, task: &TaskInfo) -> String {
    if run.state == RunState::AwaitingApproval {
        return "planned".to_owned();
    }
    match task.state {
        TaskState::Pending => "waiting".to_owned(),
        TaskState::Queued => "queued".to_owned(),
        TaskState::Preparing => "preparing".to_owned(),
        TaskState::Working => "working".to_owned(),
        TaskState::Proof => "test proof".to_owned(),
        TaskState::Check => "check".to_owned(),
        TaskState::Review => format!("review round {}", task.reviews.len()),
        TaskState::MergeQueue => "merge queue".to_owned(),
        TaskState::Merged => "merged".to_owned(),
        TaskState::Cancelled => "cancelled".to_owned(),
        TaskState::Reported => "reported".to_owned(),
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
}

fn outcome(ok: Option<bool>) -> &'static str {
    match ok {
        Some(true) => "✓",
        Some(false) => "✗",
        None => "·",
    }
}

/// The last review with a verdict.
fn last_verdict(task: &TaskInfo) -> Option<&ReviewInfo> {
    task.reviews
        .iter()
        .rev()
        .find(|review| review.verdict.is_some())
}

fn stages_text(run: &RunInfo, task: &TaskInfo) -> String {
    let state = task.state;
    let done = if state == TaskState::Working {
        "●"
    } else {
        outcome(task.done_signal.map(|_| true))
    };
    let proof = if task.test_mode != TestMode::Tdd {
        "–"
    } else if state == TaskState::Proof {
        "●"
    } else {
        outcome(task.last_proof.as_ref().map(|proof| proof.ok))
    };
    let check = if run.unverified {
        "–"
    } else if state == TaskState::Check {
        "●"
    } else {
        outcome(task.last_check.as_ref().map(|check| check.ok))
    };
    let review = if task.review_route.is_none() {
        "–"
    } else if state == TaskState::Review {
        "●"
    } else {
        outcome(last_verdict(task).map(|review| !review.blocking))
    };
    let merge = match state {
        TaskState::Merged => "✓",
        TaskState::MergeQueue => "●",
        _ => "·",
    };
    format!("done {done} → proof {proof} → check {check} → review {review} → merge {merge}")
}

fn route_text(task: &TaskInfo) -> String {
    let route = &task.route;
    let mut text = format!(
        "{} · {} · {} effort",
        route.runtime.label(),
        strength_text(route.strength),
        effort_text(route.effort)
    );
    if let Some(reviewer) = &task.review_route {
        text.push_str(&format!(
            "  →  reviewer {} · {}",
            reviewer.runtime.label(),
            strength_text(reviewer.strength)
        ));
    }
    text
}

fn deps_text(run: &RunInfo, task: &TaskInfo, app: &App) -> Option<String> {
    let mut deps: Vec<&String> = Vec::new();
    for dep in task.deps.iter().chain(&task.implicit_deps) {
        if *dep != task.id && !deps.contains(&dep) {
            deps.push(dep);
        }
    }
    let gate_open = run.state == RunState::AwaitingApproval;
    let waits: Vec<String> = deps
        .iter()
        .map(
            |dep| match run.tasks.iter().find(|other| other.id == **dep) {
                Some(other) if other.state == TaskState::Merged => format!("{dep} ✓"),
                Some(other) => {
                    let glyph = theme::task_glyph(other.state, gate_open, false, app.spinner_frame);
                    format!("{dep} {glyph}")
                }
                None => (*dep).clone(),
            },
        )
        .collect();
    let unblocks: Vec<&str> = run
        .tasks
        .iter()
        .filter(|other| other.id != task.id)
        .filter(|other| {
            other
                .deps
                .iter()
                .chain(&other.implicit_deps)
                .any(|d| *d == task.id)
        })
        .map(|other| other.id.as_str())
        .collect();
    let mut parts = Vec::new();
    if !waits.is_empty() {
        parts.push(format!("waits on {}", waits.join(" ")));
    }
    if !unblocks.is_empty() {
        parts.push(format!("unblocks {}", unblocks.join(", ")));
    }
    if task.on_critical_path {
        parts.push("on critical path".to_owned());
    }
    (!parts.is_empty()).then(|| clean(&parts.join(" · ")))
}

fn budget_text(task: &TaskInfo) -> String {
    let (budget, spent) = (&task.budget, &task.spent_session);
    let calls = (u64::from(spent.tool_calls), u64::from(budget.tool_calls));
    let secs = (spent.secs, u64::from(budget.minutes) * 60);
    // The larger fraction; an empty budget that has been spent against is full.
    let fraction = |(done, total): (u64, u64)| {
        if total == 0 {
            (u64::from(done > 0), 1)
        } else {
            (done.min(total), total)
        }
    };
    let (a, b) = (fraction(calls), fraction(secs));
    let (done, total) = if u128::from(a.0) * u128::from(b.1) >= u128::from(b.0) * u128::from(a.1) {
        a
    } else {
        b
    };
    let mut tokens = format_tokens(spent.tokens);
    if let Some(limit) = budget.tokens {
        tokens.push_str(&format!("/{}", format_tokens(limit)));
    }
    format!(
        "{} {}/{} tool calls · {}/{} min · {tokens} tokens",
        progress_bar(done, total, super::PROGRESS_WIDTH),
        spent.tool_calls,
        budget.tool_calls,
        spent.secs / 60,
        budget.minutes
    )
}

fn tries_text(run: &RunInfo, task: &TaskInfo) -> String {
    let (bounces, max) = (&task.bounces, run.max_bounces);
    let mut text = format!(
        "review {}/{max} bounces · check {}/{max}",
        bounces.review, bounces.check
    );
    if bounces.proof > 0 {
        text.push_str(&format!(" · proof {}/{max}", bounces.proof));
    }
    if bounces.merge > 0 {
        text.push_str(&format!(" · merge {}/{max}", bounces.merge));
    }
    if task.stalls > 0 {
        text.push_str(&format!(" · stalls {}", task.stalls));
    }
    if task.rung > 0 {
        text.push_str(&format!(" · escalation step {}", task.rung));
    }
    if let Some(check) = &task.size_check
        && !check.agreed
        && let Some(decided) = check.decided
    {
        text.push_str(&format!(
            " · size raised {}→{}",
            size_letter(check.engine),
            size_letter(decided)
        ));
    }
    text
}

fn diff_text(task: &TaskInfo) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(diff) = &task.diff {
        parts.push(format!(
            "{} files · +{} −{}",
            diff.files, diff.added, diff.removed
        ));
    }
    if let (Some(test), Some(red)) = (&task.test, &task.red) {
        let red7: String = red.chars().take(7).collect();
        let mut part = format!("test `{}` red {}", clean(test), clean(&red7));
        if let Some(proof) = &task.last_proof {
            part.push_str(if proof.ok { " ✓" } else { " ✗" });
        }
        parts.push(part);
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn review_text(task: &TaskInfo) -> Option<String> {
    let review = last_verdict(task)?;
    let Some(worst) = most_severe(&review.findings) else {
        return Some(format!("r{} ✓ no findings", review.round));
    };
    let mark = if review.blocking { "✗" } else { "✓" };
    let text = format!("\"{}\"", clean(&worst.text));
    let place = location(worst).map_or(text.clone(), |place| format!("{place} {text}"));
    Some(format!(
        "r{} {mark} {}: {place}",
        review.round,
        counts_text(&review.findings)
    ))
}
