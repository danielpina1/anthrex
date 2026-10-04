//! The run view's inspection of a task (milestone 8c decisions 29–31a, Interfaces
//! "Inspector contents, exact": Task). Pure, as `run.rs` is.

use super::run_format::{
    clean, effort_text, format_tokens, kind_glyph, local_hhmm, progress_bar_in, reason_text, rows,
    size_letter, strength_text, test_mode_text,
};
use super::run_orch::{messages_text, notes_text};
use super::run_patterns;
use super::run_task_outcome::{diff_text, pipeline_text, review_row};
use super::{Inspection, field};
use crate::actions_request::ActionTarget;
use crate::app::App;
use crate::theme;
use crate::tree::{NodeKey, RowKind};
use proto::{DeciderSource, RunInfo, RunState, TaskInfo, TaskState};

/// The history entries shown: the snapshot sends at most this many, newest first, and
/// the view never shows more whatever it is sent.
const HISTORY_SHOWN: usize = 10;
/// A task (Interfaces "Task"): the state word right of the title (milestone 9.0.7
/// decision 12), size and test mode in the footer.
pub(crate) fn task_inspection(run: &RunInfo, task: &TaskInfo, app: &App) -> Inspection {
    let mut fields = vec![
        field("pipeline", pipeline_text(run, task, false)),
        field("route", route_text(task)),
    ];
    // Milestone 9.5 decision 29: the race's lanes and the pair's two sessions.
    let ascii = app.palette().ascii;
    fields.extend(run_patterns::race_row(task, ascii).map(|race| field("race", race)));
    fields.extend(run_patterns::pair_row(task, ascii).map(|pair| field("pair", pair)));
    fields.extend(super::run_stage::task_fields(run, task));
    // Milestone 9.3 decision 32: DETAIL's `round <r>`, for a run of several rounds.
    if run.rounds.len() > 1 {
        fields.push(field("round", task.round.to_string()));
    }
    if let Some(deps) = deps_text(run, task, app) {
        fields.push(field("deps", deps));
    }
    fields.push(field("budget", budget_text(task, app.palette().ascii)));
    fields.push(field("tries", tries_text(run, task)));
    if let Some(diff) = diff_text(task) {
        fields.push(field("diff", diff));
    }
    if task.review_route.is_some() {
        // One row in M8c's flat list: the review row's first line.
        let review = review_row(task);
        fields.push(field("review", review.lines().next().unwrap_or_default()));
    }
    if let Some(messages) = messages_text(task) {
        fields.push(field("messages", messages));
    }
    if let Some(notes) = notes_text(task, app) {
        fields.push(field("notes", notes));
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
    // Milestone 9.0.7 decision 12: drawn as OUTCOME, EVIDENCE, INTENT and DETAIL.
    // `fields` keeps milestone 8c's flat list, which DETAIL carries on.
    let sections = super::run_task_sections::sections(run, task, &fields, app);
    let key = NodeKey::Task {
        run: run.run_id.clone(),
        id: task.id.clone(),
    };
    let actions = crate::app::actions::menu_items(run, &ActionTarget::Task(task.id.clone()))
        .is_some_and(|items| !items.is_empty());
    Inspection {
        layout: super::FieldLayout::Sections,
        sections,
        scroll: app.inspector_scroll_for(&key),
        footer: Some(footer_text(run, task)),
        actions,
        ..rows(
            glyph,
            format!("{}  {}", task.id, clean(&task.title)),
            state_word(run, task),
            fields,
        )
    }
}

/// Decision 12's footer: `stage <n> of <m> · <tag> <model> · <S|M|L> · <mode>`, the
/// stage only in a multi-stage run, the model `default` when the route names none.
fn footer_text(run: &RunInfo, task: &TaskInfo) -> String {
    let mut parts = Vec::new();
    if run.stages.len() > 1 {
        parts.push(format!("stage {} of {}", task.stage, run.stages.len()));
    }
    // One source with the plan review's route column (milestone 9.0.7 task 11).
    parts.push(super::run_format::route_tag(&task.route));
    parts.push(size_letter(task.size).to_owned());
    parts.push(test_mode_text(task.test_mode).to_owned());
    parts.join(" · ")
}

/// The state word: the stage, then ` · held` while the task waits in a hold
/// (milestone 9 decision 28); `planned` at the gate.
pub(crate) fn state_word(run: &RunInfo, task: &TaskInfo) -> String {
    if run.state == RunState::AwaitingApproval {
        return "planned".to_owned();
    }
    let stage = state_stage(task);
    if crate::tree::task_held(run, task) {
        format!("{stage} · held")
    } else {
        stage
    }
}

fn state_stage(task: &TaskInfo) -> String {
    if crate::tree::is_paused(task) {
        return "paused (message)".to_owned();
    }
    if let Some(word) = run_patterns::state_word(task) {
        return word.to_owned();
    }
    match task.state {
        TaskState::Pending => "waiting".to_owned(),
        TaskState::Queued => "queued".to_owned(),
        TaskState::Preparing => "preparing".to_owned(),
        TaskState::Working => "working".to_owned(),
        TaskState::Proof => "test proof".to_owned(),
        TaskState::Check => "check".to_owned(),
        TaskState::Review => {
            format!("in review · r{}", run_patterns::counted_reviews(task).len())
        }
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
    let ascii = app.palette().ascii;
    // Final fix wave M4: an implicit dep reads `(implied)`, as in the plan review.
    let implied = |dep: &String| {
        if task.deps.contains(dep) {
            ""
        } else {
            " (implied)"
        }
    };
    let waits: Vec<String> = deps
        .iter()
        .map(|dep| {
            let wait = match run.tasks.iter().find(|other| other.id == **dep) {
                Some(other) if other.state == TaskState::Merged => {
                    format!("{dep} {}", theme::glyph(theme::Glyph::Passed, ascii))
                }
                Some(other) => {
                    let look = theme::TaskLook {
                        state: other.state,
                        gate_open,
                        held: crate::tree::task_held(run, other),
                        paused: crate::tree::is_paused(other),
                        animating: false,
                        needs_you: crate::app::alerts::task_needs_you(run, other),
                    };
                    let (glyph, _) = theme::task_look(look, app.spinner_frame, ascii);
                    format!("{dep} {glyph}")
                }
                None => (*dep).clone(),
            };
            format!("{wait}{}", implied(dep))
        })
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
        parts.push(format!("after {}", waits.join(", ")));
    }
    if !unblocks.is_empty() {
        parts.push(format!("unblocks {}", unblocks.join(", ")));
    }
    if task.on_critical_path {
        parts.push("on critical path".to_owned());
    }
    (!parts.is_empty()).then(|| clean(&parts.join(" · ")))
}

fn budget_text(task: &TaskInfo, ascii: bool) -> String {
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
        progress_bar_in(done, total, super::PROGRESS_WIDTH, ascii),
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

/// The selected task's inspection in the run view, if a task is selected.
fn selected_task(app: &App) -> Option<Inspection> {
    let rows = app.nav_rows();
    let key = app.tree.selected.as_ref()?;
    let row = rows.iter().find(|row| &row.key == key)?;
    matches!(row.kind, RowKind::Task { .. }).then(|| super::inspect(row, app))
}

/// Milestone 9.0.5 decision 25: the rows the selected task's sections take at `width`
/// columns of panel interior, the title and the footer not counted; 0 when no task is
/// selected. The reducer clamps the panel's scroll with it, as the renderer draws.
pub fn task_panel_rows(app: &App, width: u16) -> usize {
    selected_task(app).map_or(0, |inspection| {
        super::panel::sections::body_lines(&inspection.sections, usize::from(width), app.palette())
            .len()
    })
}

/// The rows of a `width`×`height` interior the selected task's body is drawn in: the
/// title row or rows and the pinned footer taken out (decision 12). A page is this.
pub fn task_panel_room(app: &App, width: u16, height: u16) -> u16 {
    let room = selected_task(app).map_or(usize::from(height.saturating_sub(1)), |inspection| {
        super::panel::sections::room(
            &inspection,
            usize::from(width),
            usize::from(height),
            app.palette(),
        )
    });
    u16::try_from(room).unwrap_or(u16::MAX)
}
