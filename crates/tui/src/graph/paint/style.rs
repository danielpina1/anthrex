//! What a node looks like beyond its text: its status glyph and colour, its border,
//! and whether it is dimmed (milestone 4.6 decision 12; milestone 8c decisions 16,
//! 19 and 20, Interfaces "Glyphs").
//!
//! Pure: everything is read from the row, the `App` and the run it belongs to, and the
//! daemon's time comes from `App::run_now` through `App::rate_limited`, never from the
//! client's clock.

use crate::app::App;
use crate::theme;
use crate::tree::{DisplayRound, NodeKey, Row, RowKind};
use proto::{
    AgentRole, FullState, PlannerInfo, PlannerState, RunState, ScoutInfo, ScoutState, StageInfo,
    Status, TaskInfo, TaskState, WindowInfo,
};
use ratatui::style::{Color, Modifier, Style};
use std::collections::HashSet;

/// A node's border style and whether every one of its cells is dimmed.
pub(super) struct NodeStyle {
    pub border: Style,
    pub dim: bool,
}

/// Decision 20's dependency highlight, worked out once per frame: while the shown
/// selection is a task, its declared and implicit dependencies and its dependents are
/// lit, and every other node except the selection is dimmed.
#[derive(Default)]
pub(super) struct Highlight {
    selected: Option<NodeKey>,
    run: String,
    lit: HashSet<String>,
}

impl Highlight {
    /// The highlight for `rows`: none unless tree navigation shows the selection
    /// (as `is_selected` requires) and it names a task row that is on the canvas.
    pub(super) fn of(rows: &[Row<'_>], app: &App) -> Self {
        if app.tree_input.is_none() {
            return Self::default();
        }
        let Some(selected @ NodeKey::Task { .. }) = app.tree.selected.as_ref() else {
            return Self::default();
        };
        let Some((run, task)) = rows.iter().find_map(|row| match &row.kind {
            RowKind::Task { run, task } if &row.key == selected => Some((*run, *task)),
            _ => None,
        }) else {
            return Self::default();
        };
        let names_it = |other: &TaskInfo| {
            other
                .deps
                .iter()
                .chain(&other.implicit_deps)
                .any(|dep| *dep == task.id)
        };
        let mut lit: HashSet<String> = task
            .deps
            .iter()
            .chain(&task.implicit_deps)
            .cloned()
            .collect();
        lit.extend(
            run.tasks
                .iter()
                .filter(|other| names_it(other))
                .map(|other| other.id.clone()),
        );
        lit.remove(&task.id);
        Highlight {
            selected: Some(selected.clone()),
            run: run.run_id.clone(),
            lit,
        }
    }

    fn lights(&self, key: &NodeKey) -> bool {
        matches!(key, NodeKey::Task { run, id } if *run == self.run && self.lit.contains(id))
    }

    fn dims(&self, key: &NodeKey) -> bool {
        self.selected
            .as_ref()
            .is_some_and(|selected| selected != key && !self.lights(key))
    }
}

/// The border and dimming of one node. The focused window's box and a lit task's use
/// the focused border colour; a critical-path task's border is bold; a finished
/// agent node, and every node the highlight leaves out, is dim.
pub(super) fn node_style(row: &Row<'_>, app: &App, highlight: &Highlight) -> NodeStyle {
    let focused = match &row.kind {
        RowKind::Window { info, .. } => app.focused == Some(info.id),
        _ => highlight.lights(&row.key),
    };
    let mut border = if focused {
        theme::border_focused(app.settings.accent)
    } else {
        theme::border()
    };
    if matches!(&row.kind, RowKind::Task { task, .. } if task.on_critical_path) {
        border = border.add_modifier(Modifier::BOLD);
    }
    NodeStyle {
        border,
        dim: finished(&row.kind) || highlight.dims(&row.key),
    }
}

/// A round, scout or planner that has ended stays on the canvas, dimmed (decision 16).
fn finished(kind: &RowKind<'_>) -> bool {
    match kind {
        RowKind::AgentRound { round, .. } => round.ended_at.is_some(),
        RowKind::Scout { scout, .. } => {
            scout.ended_at.is_some()
                || matches!(scout.state, ScoutState::Reported | ScoutState::Failed)
        }
        RowKind::Planner { planner, .. } => {
            planner.ended_at.is_some()
                || matches!(planner.state, PlannerState::Finished | PlannerState::Failed)
        }
        // Milestone 9.1: a stage every task of which merged, green on its head.
        RowKind::Stage { stage, .. } => {
            stage.merged == stage.tasks && stage.full.state == FullState::Green
        }
        RowKind::Project { .. }
        | RowKind::Window { .. }
        | RowKind::Subagent { .. }
        | RowKind::Run { .. }
        | RowKind::Task { .. } => false,
    }
}

/// A node's status glyph and its colour.
pub(crate) fn node_glyph(row: &Row<'_>, app: &App) -> (&'static str, Color) {
    let frame = app.spinner_frame;
    match &row.kind {
        RowKind::Project { status, .. } => status_pair(*status, frame),
        RowKind::Window { info, .. } => status_pair(info.status, frame),
        RowKind::Subagent { info } => (
            theme::subagent_glyph(info, frame),
            theme::subagent_color(info),
        ),
        RowKind::Run { run, .. } => (theme::RUN_GLYPH, theme::run_color(run.state)),
        RowKind::Planner { planner, .. } => planner_glyph(planner, app),
        RowKind::Scout { scout, window, .. } => scout_glyph(scout, *window, frame),
        RowKind::Task { run, task } => theme::task_look(
            task.state,
            run.state == RunState::AwaitingApproval,
            crate::tree::task_held(run, task),
            crate::tree::is_paused(task),
            animating(task, app),
            frame,
        ),
        RowKind::AgentRound { task, round, .. } => round_glyph(task, round, app),
        RowKind::Stage { stage, .. } => stage_glyph(stage, frame),
    }
}

/// Milestone 9.1 decision 55: a stage's tier 3, `◌` while its branch is not created.
fn stage_glyph(stage: &StageInfo, frame: usize) -> (&'static str, Color) {
    match stage.full.state {
        _ if stage.head.is_none() => status_pair(Status::Starting, frame),
        FullState::Green => check(),
        FullState::Red => CROSS,
        FullState::Running | FullState::Bisecting => status_pair(Status::Working, frame),
        FullState::None => status_pair(Status::Idle, frame),
    }
}

fn status_pair(status: Status, frame: usize) -> (&'static str, Color) {
    (
        theme::status_glyph(status, frame),
        theme::status_color(status),
    )
}

/// Rejected, blocking or failed: red (decision 19).
const CROSS: (&str, Color) = ("✗", Color::Red);

/// Approved, reported or finished: the done colour, green.
fn check() -> (&'static str, Color) {
    ("✓", theme::status_color(Status::Done))
}

/// A live agent node: the spinner while its window is `Working`, `◆` while the window
/// asks for attention or the round is rate-limited, else `●` (decision 19).
fn live(window: Option<&WindowInfo>, rate_limited: bool, frame: usize) -> (&'static str, Color) {
    match window.map(|window| window.status) {
        Some(Status::Working) => status_pair(Status::Working, frame),
        Some(Status::Attention) => status_pair(Status::Attention, frame),
        _ if rate_limited => status_pair(Status::Attention, frame),
        _ => ("●", theme::status_color(Status::Working)),
    }
}

fn listed(app: &App, window_id: Option<u32>) -> Option<&WindowInfo> {
    let id = window_id?;
    app.windows.iter().find(|window| window.id == id)
}

/// `working` animates only while one of its live worker rounds' windows is `Working`.
fn animating(task: &TaskInfo, app: &App) -> bool {
    task.state == TaskState::Working
        && task.rounds.iter().any(|round| {
            round.role == AgentRole::Worker
                && round.ended_at.is_none()
                && listed(app, round.window_id).is_some_and(|w| w.status == Status::Working)
        })
}

fn round_glyph(task: &TaskInfo, round: &DisplayRound<'_>, app: &App) -> (&'static str, Color) {
    let info = round.info;
    let is_live = round.ended_at.is_none();
    match info.role {
        AgentRole::Reviewer => {
            let review = task
                .reviews
                .iter()
                .find(|review| review.round == info.round && review.verdict.is_some());
            match review {
                Some(review) if review.blocking => CROSS,
                Some(_) => check(),
                None if is_live => live(round.window, app.rate_limited(info), app.spinner_frame),
                None => ("–", theme::DIM),
            }
        }
        AgentRole::Worker | AgentRole::Orchestrator | AgentRole::Scout | AgentRole::Planner => {
            if is_live {
                live(round.window, app.rate_limited(info), app.spinner_frame)
            } else if task.state == TaskState::Blocked && last_worker_round(task, round) {
                CROSS
            } else {
                check()
            }
        }
        // A decider has no rounds (decision 43); a stray one is drawn as ended.
        AgentRole::Decider => ("–", theme::DIM),
    }
}

/// Whether `round` is the task's last worker round: the final piece of its latest
/// worker session. `DisplayRound::last` alone marks every session's final piece.
fn last_worker_round(task: &TaskInfo, round: &DisplayRound<'_>) -> bool {
    let latest = task
        .rounds
        .iter()
        .filter(|info| info.role == AgentRole::Worker)
        .max_by_key(|info| (info.started_at, info.session));
    round.last
        && round.info.role == AgentRole::Worker
        && latest.is_some_and(|latest| {
            (latest.session, latest.started_at) == (round.info.session, round.info.started_at)
        })
}

fn scout_glyph(
    scout: &ScoutInfo,
    window: Option<&WindowInfo>,
    frame: usize,
) -> (&'static str, Color) {
    match scout.state {
        ScoutState::Starting | ScoutState::Working => live(window, false, frame),
        ScoutState::Reported => check(),
        ScoutState::Failed => CROSS,
    }
}

fn planner_glyph(planner: &PlannerInfo, app: &App) -> (&'static str, Color) {
    match planner.state {
        PlannerState::Planning => live(listed(app, planner.window_id), false, app.spinner_frame),
        PlannerState::Finished => check(),
        PlannerState::Failed => CROSS,
    }
}
