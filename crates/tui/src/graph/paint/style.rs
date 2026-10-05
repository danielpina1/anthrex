//! What a node looks like beyond its text: its status glyph and colour, its border,
//! and whether it is dimmed (milestone 4.6 decision 12; milestone 8c decisions 16,
//! 19 and 20, Interfaces "Glyphs").
//!
//! Pure: everything is read from the row, the `App` and the run it belongs to, and the
//! daemon's time comes from `App::run_now` through `App::rate_limited`, never from the
//! client's clock.

use crate::app::App;
use crate::theme::{self, Glyph, Role, TaskLook};
use crate::tree::{DisplayRound, NodeKey, Row, RowKind};
use proto::{
    AgentRole, DesignAgentInfo, DesignAgentStatus, FullState, LaneState, PairPhase, PlannerInfo,
    PlannerState, RaceLane, RoundOutcome, RunState, ScoutInfo, ScoutState, Status, TaskInfo,
    TaskState, WindowInfo,
};
use ratatui::style::{Modifier, Style};
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

/// The border and dimming of one node. Every box's border is muted: a box never has
/// the keys, so it never wears the accent (milestone 9.0.7 decision 1). The focused
/// window's box, a lit task's and a critical-path task's are bold instead; a finished
/// agent node, and every node the highlight leaves out, is dim.
pub(super) fn node_style(row: &Row<'_>, app: &App, highlight: &Highlight) -> NodeStyle {
    let focused = match &row.kind {
        RowKind::Window { info, .. } => app.focused == Some(info.id),
        _ => highlight.lights(&row.key),
    };
    let mut border = theme::role(theme::Role::Muted, app.palette());
    if focused || matches!(&row.kind, RowKind::Task { task, .. } if task.on_critical_path) {
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
        // Milestone 9.1: a stage every task of which merged, green on its head, or (an
        // open PR's stage, milestone 9.7 ruling T9-1) green on an earlier head.
        RowKind::Stage { stage, .. } => {
            stage.merged == stage.tasks && stage.full.state == FullState::Green
        }
        // Milestone 9.6 ruling T18-1: a design agent that is done or failed.
        RowKind::DesignAgent { agent, .. } => matches!(
            agent.state,
            DesignAgentStatus::Done | DesignAgentStatus::Failed
        ),
        RowKind::Project { .. }
        | RowKind::Window { .. }
        | RowKind::Subagent { .. }
        | RowKind::Run { .. }
        | RowKind::Task { .. }
        | RowKind::Round { .. }
        | RowKind::IdleOrchestrator { .. } => false,
    }
}

/// A node's status glyph and its role (milestone 9.0.7 decision 3), in ASCII when
/// the palette says so.
pub(crate) fn node_glyph(row: &Row<'_>, app: &App) -> (&'static str, Role) {
    let frame = app.spinner_frame;
    let ascii = app.palette().ascii;
    match &row.kind {
        RowKind::Project { status, .. } => theme::status_look(*status, frame, ascii),
        RowKind::Window { info, .. } => theme::status_look(info.status, frame, ascii),
        RowKind::Subagent { info } => theme::subagent_look(info, frame, ascii),
        RowKind::Run { run, .. } => theme::run_look(crate::tree::shown_state(run), ascii),
        RowKind::Planner { planner, .. } => planner_glyph(planner, app),
        RowKind::Scout { scout, window, .. } => scout_glyph(scout, *window, app),
        RowKind::DesignAgent { agent, window, .. } => design_agent_glyph(agent, *window, app),
        RowKind::Task { run, task } => theme::task_look(
            TaskLook {
                state: task.state,
                gate_open: run.state == RunState::AwaitingApproval,
                held: crate::tree::task_held(run, task),
                paused: crate::tree::is_paused(task),
                animating: animating(task, app),
                needs_you: crate::app::alerts::task_needs_you(run, task),
            },
            frame,
            ascii,
        ),
        RowKind::AgentRound { task, round, .. } => round_glyph(task, round, app),
        RowKind::Stage { stage, .. } => theme::stage_look(stage, frame, ascii),
        // Milestone 9.3 decision 32: a round by its outcome, the open one as its run.
        RowKind::Round { run, round } => match round.outcome {
            None => theme::run_look(crate::tree::shown_state(run), ascii),
            Some(RoundOutcome::Completed) => check(app),
            Some(RoundOutcome::Rejected | RoundOutcome::Cancelled) => ended(app),
        },
        RowKind::IdleOrchestrator { .. } => (theme::glyph(Glyph::NotStarted, ascii), Role::Muted),
    }
}

/// Milestone 9.6 ruling T18-1: a design agent by its state: queued `◌`, running as a
/// live agent, submitted and done `✓` in `Done` (ruling T18-8, 9.0.7 decision 3: a
/// submitted draft is held until the brainstorm settles, the agent reported; only done
/// dims, in `finished`), failed `✗`.
fn design_agent_glyph(
    agent: &DesignAgentInfo,
    window: Option<&WindowInfo>,
    app: &App,
) -> (&'static str, Role) {
    let ascii = app.palette().ascii;
    match agent.state {
        DesignAgentStatus::Queued => (theme::glyph(Glyph::NotStarted, ascii), Role::Muted),
        DesignAgentStatus::Running => live(window, false, app),
        DesignAgentStatus::Submitted | DesignAgentStatus::Done => check(app),
        DesignAgentStatus::Failed => cross(app),
    }
}

/// Rejected, blocking or failed (decision 3's review and agent-node row).
fn cross(app: &App) -> (&'static str, Role) {
    (
        theme::glyph(Glyph::Failed, app.palette().ascii),
        Role::Failed,
    )
}

/// Approved, reported or finished.
fn check(app: &App) -> (&'static str, Role) {
    (theme::glyph(Glyph::Passed, app.palette().ascii), Role::Done)
}

/// An ended node with nothing to show: `–` in `Muted`.
fn ended(app: &App) -> (&'static str, Role) {
    (theme::glyph(Glyph::Ended, app.palette().ascii), Role::Muted)
}

/// A live agent node: the spinner while its window is `Working`, `⚑` while the window
/// asks for attention, `⊘` in `Paused` while the round is rate-limited or waits out a
/// failed turn (milestone 9.5 decision 43; waiting on someone else, never "needs you":
/// no alert is raised for it), else `●` (decision 19).
fn live(window: Option<&WindowInfo>, rate_limited: bool, app: &App) -> (&'static str, Role) {
    let (frame, ascii) = (app.spinner_frame, app.palette().ascii);
    match window.map(|window| window.status) {
        Some(Status::Working) => theme::status_look(Status::Working, frame, ascii),
        Some(Status::Attention) => theme::status_look(Status::Attention, frame, ascii),
        _ if rate_limited => (theme::glyph(Glyph::Blocked, ascii), Role::Paused),
        _ => (theme::glyph(Glyph::Live, ascii), Role::Working),
    }
}

fn listed(app: &App, window_id: Option<u32>) -> Option<&WindowInfo> {
    let id = window_id?;
    app.windows.iter().find(|window| window.id == id)
}

/// `working` animates only while one of its live writing rounds' windows (a worker's,
/// a racer's or a test writer's) is `Working`.
fn animating(task: &TaskInfo, app: &App) -> bool {
    task.state == TaskState::Working
        && task.rounds.iter().any(|round| {
            matches!(
                round.role,
                AgentRole::Worker | AgentRole::Racer | AgentRole::TestWriter
            ) && round.ended_at.is_none()
                && listed(app, round.window_id).is_some_and(|w| w.status == Status::Working)
        })
}

fn round_glyph(task: &TaskInfo, round: &DisplayRound<'_>, app: &App) -> (&'static str, Role) {
    let info = round.info;
    let is_live = round.ended_at.is_none();
    match info.role {
        AgentRole::Reviewer => {
            // Ruling T20-2 (I1): both lanes number their reviews from 1.
            let review = task.reviews.iter().find(|review| {
                review.round == info.round && review.lane == info.lane && review.verdict.is_some()
            });
            match review {
                Some(review) if review.blocking => cross(app),
                Some(_) => check(app),
                None if is_live => live(round.window, app.round_waits(info), app),
                None => ended(app),
            }
        }
        // Milestone 9.5 decision 29: an ended racer by its lane's outcome; an ended
        // test writer fails only as the last writer of a task blocked writing its test.
        // Live, both follow a worker's rules.
        AgentRole::Racer if !is_live => match lane_state(task, info.lane) {
            Some(LaneState::Lost | LaneState::Out) => ended(app),
            _ => check(app),
        },
        AgentRole::TestWriter if !is_live => {
            let writing = task.pair.as_ref().map(|pair| pair.phase) == Some(PairPhase::Writing);
            let blocked = writing && task.state == TaskState::Blocked;
            match blocked && last_round_of(task, round, AgentRole::TestWriter) {
                true => cross(app),
                false => check(app),
            }
        }
        AgentRole::Worker
        | AgentRole::Orchestrator
        | AgentRole::Scout
        | AgentRole::Planner
        | AgentRole::Racer
        | AgentRole::TestWriter => {
            if is_live {
                live(round.window, app.round_waits(info), app)
            } else if task.state == TaskState::Blocked
                && last_round_of(task, round, AgentRole::Worker)
            {
                cross(app)
            } else {
                check(app)
            }
        }
        // A decider has no rounds (decision 43); a stray one is drawn as ended.
        // Nor does a design agent (milestone 9.6).
        AgentRole::Decider | AgentRole::Brainstormer | AgentRole::DocReviewer => ended(app),
    }
}

/// Whether `round` is the task's last round of `role`: the final piece of its latest
/// session of that role. `DisplayRound::last` alone marks every session's final piece.
fn last_round_of(task: &TaskInfo, round: &DisplayRound<'_>, role: AgentRole) -> bool {
    let latest = task
        .rounds
        .iter()
        .filter(|info| info.role == role)
        .max_by_key(|info| (info.started_at, info.session));
    round.last
        && round.info.role == role
        && latest.is_some_and(|latest| {
            (latest.session, latest.started_at) == (round.info.session, round.info.started_at)
        })
}

/// The state of a racing task's `lane`, while the snapshot names it.
fn lane_state(task: &TaskInfo, lane: Option<RaceLane>) -> Option<LaneState> {
    let lanes = &task.race.as_ref()?.lanes;
    let info = lanes.iter().find(|info| Some(info.lane) == lane)?;
    Some(info.state)
}

fn scout_glyph(scout: &ScoutInfo, window: Option<&WindowInfo>, app: &App) -> (&'static str, Role) {
    match scout.state {
        ScoutState::Starting | ScoutState::Working => live(window, false, app),
        ScoutState::Reported => check(app),
        ScoutState::Failed => cross(app),
    }
}

fn planner_glyph(planner: &PlannerInfo, app: &App) -> (&'static str, Role) {
    match planner.state {
        PlannerState::Planning => live(listed(app, planner.window_id), false, app),
        PlannerState::Finished => check(app),
        PlannerState::Failed => cross(app),
    }
}
