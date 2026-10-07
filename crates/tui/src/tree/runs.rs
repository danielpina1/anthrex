//! Runs in the project tree (milestone 8c, decisions 6–10): which runs are shown, which
//! windows they own, a run's rolled-up status, and the grouping of plain windows and
//! runs into projects. Milestone 9.3 decision 32: a project's idle orchestrator is a row
//! of its own, in its window's place.

use super::{
    NodeKey, ProjectChild, ProjectGroup, Row, RowKind, RuntimeCounts, TreeState, display_names,
    matches_filter, urgency,
};
use crate::safe_text::one_line;
use proto::{
    AgentRole, BlockReason, DeliveryMode, DocGateKind, HoldInfo, HoldState, IdleOrchestrator,
    RunInfo, RunState, Runtime, Status, TaskInfo, TaskState, WindowInfo,
};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// A shown run and its orchestrator's window, when one is listed (decision 10).
pub(super) struct ShownRun<'a> {
    pub run: &'a RunInfo,
    pub orchestrator: Option<&'a WindowInfo>,
}

/// Decision 6: the runs the tree shows are the non-terminal ones, oldest first by
/// `(created_at, run_id)` — the snapshot arrives newest first (decision 7). A run id
/// listed twice is shown once, the first copy in that order, so no two rows share a
/// key (the daemon builds the list from a map, so this is robustness only).
pub fn shown_runs(runs: &[RunInfo]) -> impl Iterator<Item = &RunInfo> {
    let mut shown: Vec<&RunInfo> = runs.iter().filter(|run| !run.state.is_terminal()).collect();
    shown.sort_by(|left, right| {
        left.created_at
            .cmp(&right.created_at)
            .then_with(|| left.run_id.cmp(&right.run_id))
    });
    let mut seen = HashSet::new();
    shown.retain(|run| seen.insert(run.run_id.as_str()));
    shown.into_iter()
}

/// Milestone 9 decision 42c: a `paused(message)` task — blocked, but by the
/// orchestrator's `stop_and_wait`, not by anything the user must answer.
pub fn is_paused(task: &TaskInfo) -> bool {
    task.state == TaskState::Blocked
        && task
            .block
            .as_ref()
            .is_some_and(|block| block.reason == BlockReason::MessagePause)
}

/// Milestone 9 decision 28: the run's holds that await the user's approval.
pub fn awaiting_holds(run: &RunInfo) -> impl Iterator<Item = &HoldInfo> {
    run.holds
        .iter()
        .filter(|hold| hold.state == HoldState::Awaiting)
}

/// Milestone 9 decision 28: `task` waits in a hold not yet approved, so it is not
/// runnable. A hold the snapshot does not list holds nothing.
pub fn task_held(run: &RunInfo, task: &TaskInfo) -> bool {
    let Some(id) = task.hold.as_deref() else {
        return false;
    };
    run.holds.iter().any(|hold| {
        hold.id == id && matches!(hold.state, HoldState::Drafting | HoldState::Awaiting)
    })
}

/// The state a run is drawn as. Final fix wave FW-47 (WB-D-I1, milestone 9.0.7
/// decisions 3 and 4): a design gate the orchestrator is revising waits on the
/// orchestrator, not on you, so it is drawn as the phase it revises in (as the run
/// node's text already reads it), never as `AwaitingApproval`'s ⚑.
pub fn shown_state(run: &RunInfo) -> RunState {
    match (run.state, &run.doc_gate) {
        (RunState::AwaitingApproval, Some(gate)) if gate.revising.is_some() => match gate.kind {
            DocGateKind::Brainstorm => RunState::Brainstorming,
            DocGateKind::Spec => RunState::Specifying,
            DocGateKind::Plan => RunState::Planning,
        },
        (state, _) => state,
    }
}

/// Decision 9: the status a run contributes to its project's roll-up. Milestone 9: a
/// hold awaiting approval asks for the user; a `paused(message)` task does not until
/// the daemon lists it in the run's attention (after 600 s, decision 42c), since any
/// line the daemon lists there asks for the user.
pub fn run_status(run: &RunInfo) -> Status {
    let blocked = |task: &TaskInfo| task.state == TaskState::Blocked && !is_paused(task);
    let asks = run.tasks.iter().any(blocked)
        || awaiting_holds(run).next().is_some()
        || !run.attention.is_empty();
    match shown_state(run) {
        RunState::AwaitingApproval | RunState::Paused | RunState::Halted => Status::Attention,
        RunState::Running | RunState::Planning | RunState::Brainstorming | RunState::Specifying
            if asks =>
        {
            Status::Attention
        }
        RunState::Running | RunState::Planning | RunState::Brainstorming | RunState::Specifying => {
            Status::Working
        }
        RunState::Complete | RunState::Accepted => Status::Done,
        // Terminal: never shown (decision 6), mapped only to keep the match exhaustive.
        RunState::Discarded | RunState::Failed => Status::Exited,
    }
}

/// `(merged, total)` over a run's tasks, cancelled tasks left out of both.
pub fn run_progress(run: &RunInfo) -> (usize, usize) {
    let counted = run
        .tasks
        .iter()
        .filter(|task| task.state != TaskState::Cancelled);
    counted.fold((0, 0), |(merged, total), task| {
        (
            merged + usize::from(task.state == TaskState::Merged),
            total + 1,
        )
    })
}

/// The run title change: the text a run is named by, its short title when it has one
/// (protocol 18's `RunInfo.title`), else its goal.
pub fn run_label(run: &RunInfo) -> &str {
    if run.title.trim().is_empty() {
        &run.goal
    } else {
        &run.title
    }
}

/// The text a run is known by in the tree: its [`run_label`], or its id when that is
/// blank.
pub fn run_title(run: &RunInfo) -> &str {
    let label = run_label(run);
    if label.trim().is_empty() {
        &run.run_id
    } else {
        label
    }
}

/// The project-tree filter (decision 7): a run matches on its goal, its title or its id.
pub(super) fn run_matches_filter(run: &RunInfo, filter: &str) -> bool {
    matches_filter(&run.goal, filter)
        || matches_filter(&run.title, filter)
        || matches_filter(&run.run_id, filter)
}

/// Milestone 9.3 decision 32: the idle row's text, `orchestrator · idle · after <h4>`,
/// then ` · <k> runs` from two runs on (KG §11). `<h4>` is the cleaned run id's last
/// four characters, so a hidden carrier in the id can neither show nor shift them.
pub fn idle_text(idle: &IdleOrchestrator) -> String {
    let after = one_line(&idle.after_run);
    let h4 = crate::actions_request::short_id(&after);
    let runs = match idle.runs {
        k if k >= 2 => format!(" · {k} runs"),
        _ => String::new(),
    };
    format!("orchestrator · idle · after {h4}{runs}")
}

/// The idle chain's last run's outcome as a word: `delivered` for a complete `pr` run
/// (D17, looked up in the snapshot), else the state's own word.
pub fn idle_outcome(idle: &IdleOrchestrator, runs: &[RunInfo]) -> &'static str {
    let pr = (runs.iter().find(|run| run.run_id == idle.after_run))
        .and_then(|run| run.delivery.as_ref())
        .is_some_and(|delivery| delivery.mode == DeliveryMode::Pr);
    // D17: a chain is idle while its run is `Complete` only for a delivered `pr` run, so
    // a run the snapshot no longer lists reads `delivered` too.
    let missing = !runs.iter().any(|run| run.run_id == idle.after_run);
    match idle.outcome {
        RunState::Complete if pr || missing => "delivered",
        state => crate::app::state_text(state),
    }
}

/// Window `id`'s row: its own, or the idle row whose window it is (decision 32).
pub fn window_row(rows: &[Row<'_>], id: u32) -> Option<usize> {
    rows.iter().position(|row| match &row.kind {
        RowKind::Window { info, .. } => info.id == id,
        RowKind::IdleOrchestrator { window, .. } => window.id == id,
        _ => false,
    })
}

/// The project-tree filter on an idle row: its text or its chain id.
pub(super) fn idle_matches_filter(idle: &IdleOrchestrator, filter: &str) -> bool {
    matches_filter(&idle_text(idle), filter) || matches_filter(&idle.chain, filter)
}

/// Decision 32: the idle, unended chains with a listed window (never window 0), each
/// window once, by project. An ended chain (`fresh`) has no row.
fn idle_rows<'a>(
    idle: &'a [IdleOrchestrator],
    windows: &'a [WindowInfo],
) -> Vec<(&'a IdleOrchestrator, &'a WindowInfo)> {
    let (mut ids, mut chains) = (HashSet::new(), HashSet::new());
    (idle.iter())
        .filter(|idle| !idle.fresh)
        .filter_map(|idle| {
            let id = idle.window_id.filter(|id| *id != 0)?;
            let window = windows.iter().find(|window| window.id == id)?;
            let new = !ids.contains(&id) && !chains.contains(idle.chain.as_str());
            ids.insert(id);
            chains.insert(idle.chain.as_str());
            new.then_some((idle, window))
        })
        .collect()
}

/// Decision 8: a window is owned, and so not listed as a plain window, when its `run`
/// names a shown run. Every other window — no snapshot yet, a run that left, a run-less
/// headless window — is plain.
fn owned_by(window: &WindowInfo, shown: &HashSet<&str>) -> bool {
    window
        .run
        .as_ref()
        .is_some_and(|run| shown.contains(run.run_id.as_str()))
}

/// Decision 10: the lowest-id listed window that is `run`'s orchestrator.
pub(super) fn orchestrator_of<'a>(
    run: &RunInfo,
    windows: &'a [WindowInfo],
) -> Option<&'a WindowInfo> {
    windows
        .iter()
        .filter(|window| {
            window.run.as_ref().is_some_and(|reference| {
                reference.run_id == run.run_id && reference.role == AgentRole::Orchestrator
            })
        })
        .min_by_key(|window| window.id)
}

/// Every project that has a plain window or a shown run, with its roll-up (decision 9),
/// in display order: most urgent first, then by name, then by root.
pub(super) fn group_projects<'a>(
    windows: &'a [WindowInfo],
    runs: &'a [RunInfo],
    idle: &'a [IdleOrchestrator],
) -> Vec<ProjectGroup<'a>> {
    let shown: Vec<&'a RunInfo> = shown_runs(runs).collect();
    let shown_ids: HashSet<&str> = shown.iter().map(|run| run.run_id.as_str()).collect();
    // Decision 32: an idle orchestrator's window is its row, listed nowhere else.
    let mut by_idle: HashMap<&'a Path, Vec<(&'a IdleOrchestrator, &'a WindowInfo)>> =
        HashMap::new();
    for (idle, window) in idle_rows(idle, windows) {
        by_idle
            .entry(idle.project.as_path())
            .or_default()
            .push((idle, window));
    }
    let idle_ids: HashSet<u32> = (by_idle.values().flatten()).map(|(_, w)| w.id).collect();
    let windows_left: Vec<&'a WindowInfo> = (windows.iter())
        .filter(|window| !idle_ids.contains(&window.id))
        .collect();

    let mut plain: HashMap<&'a Path, Vec<&'a WindowInfo>> = HashMap::new();
    for window in (windows_left.iter().copied()).filter(|window| !owned_by(window, &shown_ids)) {
        plain
            .entry(window.project.as_path())
            .or_default()
            .push(window);
    }
    let mut by_run: HashMap<&'a Path, Vec<ShownRun<'a>>> = HashMap::new();
    for run in shown {
        by_run
            .entry(run.project.as_path())
            .or_default()
            .push(ShownRun {
                run,
                orchestrator: orchestrator_of(run, windows)
                    .filter(|window| !idle_ids.contains(&window.id)),
            });
    }

    let roots: HashSet<&'a Path> = (plain.keys().chain(by_run.keys()).chain(by_idle.keys()))
        .copied()
        .collect();
    let names = display_names(roots.iter().copied());
    let mut projects: Vec<_> = roots
        .into_iter()
        .map(|root| {
            let mut windows = plain.remove(root).unwrap_or_default();
            windows.sort_by_key(|window| window.id);
            let runs = by_run.remove(root).unwrap_or_default();
            let idle = by_idle.remove(root).unwrap_or_default();
            let status = (windows.iter().chain(idle.iter().map(|(_, window)| window)))
                .map(|window| window.status)
                .chain(runs.iter().map(|shown| run_status(shown.run)))
                .min_by_key(|status| urgency(*status))
                .unwrap_or(Status::Idle);
            let mut counts = RuntimeCounts::default();
            for window in windows.iter().chain(idle.iter().map(|(_, window)| window)) {
                match window.runtime {
                    Runtime::Claude => counts.claude += 1,
                    Runtime::Codex => counts.codex += 1,
                    Runtime::Shell => counts.shell += 1,
                }
            }
            ProjectGroup {
                root,
                name: names
                    .get(root)
                    .expect("every grouped root has a display name")
                    .clone(),
                status,
                counts,
                runs,
                idle,
                members: windows.into_iter().map(ProjectChild::Window).collect(),
            }
        })
        .collect();
    projects.sort_by(|left, right| {
        urgency(left.status)
            .cmp(&urgency(right.status))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.root.cmp(right.root))
    });
    projects
}

impl TreeState {
    /// Keeps the run-view keys of shown runs and drops the rest, and records the
    /// projects shown runs name, so `prune` keeps a `Project` key whose root has a run
    /// but no window (decision 7). Call `prune` after it for the project keys to follow.
    pub fn prune_runs(&mut self, runs: &[RunInfo]) {
        let shown: Vec<&RunInfo> = shown_runs(runs).collect();
        let is_shown = |id: &str| shown.iter().any(|run| run.run_id == id);
        self.run_roots = shown.iter().map(|run| run.project.clone()).collect();
        self.collapsed.retain(|key| match key {
            NodeKey::Run(run)
            | NodeKey::Planner { run, .. }
            | NodeKey::Scout { run, .. }
            | NodeKey::DesignAgent { run, .. }
            | NodeKey::Task { run, .. }
            | NodeKey::Stage { run, .. }
            | NodeKey::Round { run, .. }
            | NodeKey::AgentRound { run, .. } => is_shown(run),
            NodeKey::Project(_)
            | NodeKey::Window(_)
            | NodeKey::Subagent { .. }
            | NodeKey::Chain(_) => true,
        });
    }
}
