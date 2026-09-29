//! The run view's inspections of a run, a sub-planner and a scout (milestone 8c
//! decisions 29–31a, Interfaces "Inspector contents, exact"); the formatting they share
//! with tasks and rounds is in `run_format.rs`.
//!
//! Pure: every time is the snapshot's, aged through `App::run_age` and compared through
//! `App::rate_limited`, and a wall-clock stamp is formatted with the offset the `App`
//! was given at startup (`local_hhmm`). No clock is read here.

use super::run_format::{
    billable, clean, format_duration, format_tokens, kind_glyph, local_hhmm, progress_text,
    reason_text, rows, session_text,
};
use super::run_orch::orchestrator_text;
use super::{Field, Inspection, field};
use crate::app::{App, state_text};
use crate::tree::RowKind;
use daemon::run::triage::{kinds_scale, source_label};
use proto::{
    AgentRoundInfo, PlannerInfo, PlannerState, RunInfo, RunPath, RunState, Runtime, ScoutInfo,
    ScoutState, TaskInfo, TaskState, TokenUsage, WindowInfo,
};

/// The run (Interfaces "Run (orchestrator)"). The orchestrator's window does not change
/// what is shown; milestone 9 may add its session.
pub(crate) fn run_inspection(
    run: &RunInfo,
    _orchestrator: Option<&WindowInfo>,
    app: &App,
) -> Inspection {
    let state = match (run.state, run.paused_from) {
        (RunState::Paused, Some(from)) => format!("paused (from {})", state_text(from)),
        (state, _) => state_text(state).to_owned(),
    };
    let right = format!("{state} · {}", format_duration(app.run_age(run.created_at)));
    let mut fields = vec![field(
        "progress",
        progress_text(run.tasks.iter(), super::RUN_PROGRESS_WIDTH),
    )];
    if !run.critical_path.is_empty() {
        fields.push(field("path", path_text(run)));
    }
    fields.push(field("agents", agents_text(run, app)));
    fields.push(field("spend", spend_text(run)));
    fields.push(field("gate", gate_text(run, app)));
    if let Some(orchestrator) = &run.orchestrator {
        fields.push(field("orchestrator", orchestrator_text(orchestrator)));
        if let Some(summary) = &orchestrator.summary {
            fields.push(field("summary", clean(summary)));
        }
    }
    if let Some(attention) = attention_text(run) {
        fields.push(field("attention", attention));
    }
    let glyph = kind_glyph(
        RowKind::Run {
            run,
            orchestrator: None,
            position: None,
        },
        app,
    );
    rows(
        glyph,
        format!("{}  {}", run.run_id, clean(&run.goal)),
        right,
        fields,
    )
}

fn path_text(run: &RunInfo) -> String {
    let left = run
        .critical_path
        .iter()
        .filter(|id| {
            !run.tasks
                .iter()
                .any(|task| task.id == **id && task.state.is_finished())
        })
        .count();
    let noun = if left == 1 { "task" } else { "tasks" };
    let mut text = format!(
        "critical path {} · {left} {noun} left",
        clean(&run.critical_path.join(" → "))
    );
    if let Some(p) = run.bound_ratio_permille {
        text.push_str(&format!(" · {}.{}× the bound", p / 1000, p % 1000 / 100));
    }
    text
}

fn agents_text(run: &RunInfo, app: &App) -> String {
    let mut text = format!(
        "workers {}/{} · readers {}/{}",
        run.writers_busy, run.max_writers, run.readers_busy, run.max_readers
    );
    let routes = || {
        run.tasks
            .iter()
            .flat_map(|task| std::iter::once(&task.route).chain(task.review_route.as_ref()))
    };
    for runtime in [Runtime::Claude, Runtime::Codex, Runtime::Shell] {
        if !routes().any(|route| route.runtime == runtime) {
            continue;
        }
        let limited: Vec<&AgentRoundInfo> = run
            .tasks
            .iter()
            .flat_map(|task| &task.rounds)
            .filter(|round| round.route.runtime == runtime && round.ended_at.is_none())
            .filter(|round| app.rate_limited(round))
            .collect();
        let label = runtime.label();
        if limited.is_empty() {
            text.push_str(&format!(" · {label} ok"));
            continue;
        }
        text.push_str(&format!(" · {label} rate-limited"));
        if let Some(since) = limited.iter().filter_map(|r| r.rate_limited_since).min() {
            text.push_str(&format!(" {}", format_duration(app.run_age(since))));
        }
    }
    text
}

fn spend_text(run: &RunInfo) -> String {
    let usage = run
        .usage
        .as_ref()
        .map(|usage| usage.total)
        .unwrap_or_else(|| {
            let mut sum = TokenUsage::default();
            for round in run.tasks.iter().flat_map(|task| &task.rounds) {
                sum += round.usage;
            }
            sum
        });
    let mut text = format!("tokens {}", format_tokens(billable(&usage)));
    let denominator =
        u128::from(usage.input) + u128::from(usage.cache_read) + u128::from(usage.cache_write);
    // Omitted when the denominator is 0 (decision 31).
    if let Some(share) = (u128::from(usage.cache_read) * 100).checked_div(denominator) {
        text.push_str(&format!(" (cache {share}%)"));
    }
    let calls: u64 = run
        .tasks
        .iter()
        .map(|task| u64::from(task.spent_total.tool_calls))
        .sum();
    text.push_str(&format!(" · tool calls {calls}"));
    if let Some(left) = run.estimate_left_secs {
        text.push_str(&format!(" · est. left ~{}", format_duration(left)));
    }
    text
}

fn gate_text(run: &RunInfo, app: &App) -> String {
    if run.state == RunState::Planning {
        return "planning · s submits the plan yourself".to_owned();
    }
    if run.state == RunState::AwaitingApproval {
        return "awaiting approval · a approve · x reject · e edit · d remove".to_owned();
    }
    if run.path == Some(RunPath::Fast) {
        let mut text = "fast path · no plan gate".to_owned();
        if let Some(triage) = &run.triage {
            text.push_str(&format!(
                " · triage: {} ({})",
                kinds_scale(&triage.kinds, triage.scale),
                source_label(triage.source)
            ));
        }
        return text;
    }
    let Some(by) = run.approved_by.as_deref() else {
        return "plan not approved".to_owned();
    };
    let mut text = if by == "--yes" {
        "plan approved by --yes".to_owned()
    } else {
        "plan approved".to_owned()
    };
    if let Some(at) = run.approved_at {
        text.push_str(&format!(" {}", local_hhmm(at, app.utc_offset_secs)));
    }
    match run.plan_edits_since_approval {
        0 => {}
        1 => text.push_str(" · 1 plan edit since"),
        n => text.push_str(&format!(" · {n} plan edits since")),
    }
    if let Some(last) = run.plan_edits.first() {
        text.push_str(&format!(" · last: {}", clean(&last.text)));
    }
    text
}

/// The `attention` row: the halt, else the first blocked task, else the first other
/// line; then how many more there are, each counted once. Milestone 9: a hold awaiting
/// approval comes first among the other lines (decision 28); a `paused(message)` task
/// is not a blocked one, and shows only when the daemon lists it (decision 42c).
fn attention_text(run: &RunInfo) -> Option<String> {
    let blocked: Vec<&TaskInfo> = run
        .tasks
        .iter()
        .filter(|task| task.state == TaskState::Blocked && task.block.is_some())
        .filter(|task| !crate::tree::is_paused(task))
        .collect();
    let is_task_line = |line: &str| {
        run.tasks
            .iter()
            .any(|task| task.block.is_some() && line.starts_with(&format!("{} blocked (", task.id)))
    };
    let holds = crate::tree::awaiting_holds(run).map(|hold| match hold.tasks.len() {
        1 => format!("hold {}: 1 task waits for approval", hold.id),
        n => format!("hold {}: {n} tasks wait for approval", hold.id),
    });
    let others: Vec<String> = holds
        .chain(
            (run.attention.iter())
                .filter(|line| !is_task_line(line))
                .cloned(),
        )
        .collect();
    let (first, more) = if run.state == RunState::Halted {
        let reason = run.halted_reason.as_deref().map(clean);
        let text = reason.map_or_else(|| "halted".to_owned(), |r| format!("halted: {r}"));
        (text, blocked.len() + others.len())
    } else if let Some(task) = blocked.first() {
        let block = task.block.as_ref()?;
        let text = format!(
            "{} blocked: {} — \"{}\"",
            clean(&task.id),
            reason_text(block.reason),
            clean(&block.text)
        );
        (text, blocked.len() - 1 + others.len())
    } else {
        // Milestone 9 decision 29: the promotion line M8c put a local time into is
        // gone, so every line is shown as the daemon sent it.
        (clean(others.first()?), others.len() - 1)
    };
    Some(if more > 0 {
        format!("{first} · +{more} more")
    } else {
        first
    })
}

/// A sub-planner (Interfaces "Sub-planner").
pub(crate) fn planner_inspection(run: &RunInfo, planner: &PlannerInfo, app: &App) -> Inspection {
    let tasks = || {
        run.tasks
            .iter()
            .filter(|task| task.epic.as_deref() == Some(planner.epic.as_str()))
    };
    let took = planner.ended_at.map_or_else(
        || app.run_age(planner.started_at),
        |ended| ended.saturating_sub(planner.started_at),
    );
    let right = match planner.state {
        PlannerState::Planning => format!("planning · {}", format_duration(took)),
        PlannerState::Finished => {
            let n = tasks().count();
            let noun = if n == 1 { "task" } else { "tasks" };
            format!("finished · planned {n} {noun} in {}", format_duration(took))
        }
        PlannerState::Failed => format!("failed · {}", format_duration(took)),
    };
    let mut fields = vec![field(
        "progress",
        progress_text(tasks(), super::PROGRESS_WIDTH),
    )];
    if !planner.area.is_empty() {
        fields.push(field("area", clean(&planner.area.join(" · "))));
    }
    let mut edits = format!(
        "{} accepted · {} rejected",
        planner.edits_accepted, planner.edits_rejected
    );
    if let Some(rejection) = &planner.last_rejection {
        edits.push_str(&format!(" ({})", clean(rejection)));
    }
    let times = match planner.replans.len() {
        0 => None,
        1 => Some("once".to_owned()),
        2 => Some("twice".to_owned()),
        n => Some(format!("{n} times")),
    };
    if let Some(times) = times {
        let list = clean(&planner.replans.join(", "));
        edits.push_str(&format!(" · re-planned {times} ({list})"));
    }
    fields.push(field("edits", edits));
    // Milestone 9 decision 33: the planner's own note, and its session.
    if let Some(note) = &planner.note {
        fields.push(field("note", clean(note)));
    }
    fields.push(field(
        "session",
        session_text(planner.window_id, "main checkout, read-only", app),
    ));
    let title = planner.title.trim();
    let name = if title.is_empty() {
        format!("planner {}", clean(&planner.epic))
    } else {
        format!("planner {}  {}", clean(&planner.epic), clean(title))
    };
    let glyph = kind_glyph(RowKind::Planner { run, planner }, app);
    rows(glyph, name, right, fields)
}

/// A scout (Interfaces "Scout"): its question is the one wrapping field.
pub(crate) fn scout_inspection(
    run: &RunInfo,
    scout: &ScoutInfo,
    window: Option<&WindowInfo>,
    app: &App,
) -> Inspection {
    let took = scout.ended_at.map_or_else(
        || app.run_age(scout.started_at),
        |ended| ended.saturating_sub(scout.started_at),
    );
    let state = match scout.state {
        ScoutState::Starting => "starting",
        ScoutState::Working => "working",
        ScoutState::Reported => "reported",
        ScoutState::Failed => "failed",
    };
    let live = matches!(scout.state, ScoutState::Starting | ScoutState::Working);
    let mut state_value = match (&scout.state, &scout.failure) {
        (ScoutState::Failed, Some(failure)) => format!("failed: {}", clean(failure)),
        _ => state.to_owned(),
    };
    if let Some(tool) = window
        .and_then(|window| window.tool.as_deref())
        .filter(|_| live)
    {
        state_value.push_str(&format!(" · {}", clean(tool)));
    }
    let mut fields = vec![
        Field {
            label: "question",
            value: clean(&scout.question),
            wrap: true,
        },
        field("state", state_value),
        field("took", format_duration(took)),
        field(
            "report",
            scout
                .report_bytes
                .map_or_else(|| "not yet".to_owned(), |bytes| format!("{bytes} bytes")),
        ),
    ];
    if !scout.files.is_empty() {
        fields.push(field("files", clean(&scout.files.join(", "))));
    }
    fields.push(field(
        "session",
        session_text(scout.window_id, "main checkout, read-only", app),
    ));
    let right = format!("{state} · {}", format_duration(took));
    let glyph = kind_glyph(RowKind::Scout { run, scout, window }, app);
    rows(glyph, format!("scout {}", clean(&scout.id)), right, fields)
}
