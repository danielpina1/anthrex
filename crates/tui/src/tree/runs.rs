//! Runs in the project tree (milestone 8c, decisions 6–10): which runs are shown, which
//! windows they own, a run's rolled-up status, and the grouping of plain windows and
//! runs into projects.

use super::{
    NodeKey, ProjectChild, ProjectGroup, RuntimeCounts, TreeState, display_names, matches_filter,
    urgency,
};
use proto::{AgentRole, RunInfo, RunState, Runtime, Status, TaskState, WindowInfo};
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

/// Decision 9: the status a run contributes to its project's roll-up.
pub fn run_status(run: &RunInfo) -> Status {
    match run.state {
        RunState::AwaitingApproval | RunState::Paused | RunState::Halted => Status::Attention,
        RunState::Running
            if run
                .tasks
                .iter()
                .any(|task| task.state == TaskState::Blocked) =>
        {
            Status::Attention
        }
        RunState::Running | RunState::Planning => Status::Working,
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

/// The text a run is known by in the tree: its goal, or its id when the goal is blank.
pub fn run_title(run: &RunInfo) -> &str {
    if run.goal.trim().is_empty() {
        &run.run_id
    } else {
        &run.goal
    }
}

/// The project-tree filter (decision 7): a run matches on its goal or its id.
pub(super) fn run_matches_filter(run: &RunInfo, filter: &str) -> bool {
    matches_filter(&run.goal, filter) || matches_filter(&run.run_id, filter)
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
) -> Vec<ProjectGroup<'a>> {
    let shown: Vec<&'a RunInfo> = shown_runs(runs).collect();
    let shown_ids: HashSet<&str> = shown.iter().map(|run| run.run_id.as_str()).collect();

    let mut plain: HashMap<&'a Path, Vec<&'a WindowInfo>> = HashMap::new();
    for window in windows
        .iter()
        .filter(|window| !owned_by(window, &shown_ids))
    {
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
                orchestrator: orchestrator_of(run, windows),
            });
    }

    let roots: HashSet<&'a Path> = plain.keys().chain(by_run.keys()).copied().collect();
    let names = display_names(roots.iter().copied());
    let mut projects: Vec<_> = roots
        .into_iter()
        .map(|root| {
            let mut windows = plain.remove(root).unwrap_or_default();
            windows.sort_by_key(|window| window.id);
            let runs = by_run.remove(root).unwrap_or_default();
            let status = windows
                .iter()
                .map(|window| window.status)
                .chain(runs.iter().map(|shown| run_status(shown.run)))
                .min_by_key(|status| urgency(*status))
                .unwrap_or(Status::Idle);
            let mut counts = RuntimeCounts::default();
            for window in &windows {
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
            | NodeKey::Task { run, .. }
            | NodeKey::AgentRound { run, .. } => is_shown(run),
            NodeKey::Project(_) | NodeKey::Window(_) | NodeKey::Subagent { .. } => true,
        });
    }
}
