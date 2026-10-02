mod forest;
mod names;
mod rows;
mod run_rows;
mod runs;

pub use forest::{SubagentNode, subagent_forest};
pub use names::display_names;
use proto::{
    AgentRole, AgentRoundInfo, PlannerInfo, RunInfo, Runtime, ScoutInfo, StageInfo, Status,
    SubagentInfo, TaskInfo, WindowInfo,
};
use rows::{SubagentWalk, emit_subagents, guide_prefix, visible_windows};
pub use run_rows::{RunFilter, display_rounds, round_label, run_rows};
use runs::{ShownRun, group_projects, run_matches_filter};
pub use runs::{
    awaiting_holds, is_paused, run_progress, run_status, run_title, shown_runs, task_held,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeKey {
    Project(PathBuf),
    Window(u32),
    Subagent {
        window_id: u32,
        id: String,
    },
    /// Milestone 8c: a run, both its project-tree node and its run view's root.
    Run(String),
    Planner {
        run: String,
        epic: String,
    },
    Scout {
        run: String,
        id: String,
    },
    Task {
        run: String,
        id: String,
    },
    /// Milestone 9.1 decision 55: a stage of a `Multi` run, between the run and its tasks.
    Stage {
        run: String,
        n: u16,
    },
    /// `round` is the display round (milestone 8c decision 14), not `AgentRoundInfo.round`.
    AgentRound {
        run: String,
        task: String,
        role: AgentRole,
        session: u32,
        round: u32,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeCounts {
    pub claude: usize,
    pub codex: usize,
    pub shell: usize,
}

/// A plain window of a project. A project's shown runs are held apart, in
/// `ProjectGroup::runs`: their rows sit above the plain windows, and the windows they
/// own are not members at all (milestone 8c decisions 7 and 8).
pub enum ProjectChild<'a> {
    Window(&'a WindowInfo),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RowKind<'a> {
    Project {
        root: &'a Path,
        name: String,
        status: Status,
        counts: RuntimeCounts,
        collapsed: bool,
    },
    Window {
        info: &'a WindowInfo,
        position: usize,
        has_subagents: bool,
        collapsed: bool,
    },
    Subagent {
        info: &'a SubagentInfo,
    },
    /// Milestone 8c: a run, a leaf of the project tree and the run view's root. The
    /// orchestrator's window, when listed, takes `position` (decision 10).
    Run {
        run: &'a RunInfo,
        orchestrator: Option<&'a WindowInfo>,
        position: Option<usize>,
    },
    // The run view's rows (built from task M8c.4 on).
    Planner {
        run: &'a RunInfo,
        planner: &'a PlannerInfo,
    },
    Scout {
        run: &'a RunInfo,
        scout: &'a ScoutInfo,
        window: Option<&'a WindowInfo>,
    },
    Task {
        run: &'a RunInfo,
        task: &'a TaskInfo,
    },
    /// Milestone 9.1 decision 55: a stage node of a `Multi` run.
    Stage {
        run: &'a RunInfo,
        stage: &'a StageInfo,
    },
    AgentRound {
        run: &'a RunInfo,
        task: &'a TaskInfo,
        round: DisplayRound<'a>,
    },
}

/// One agent-round node of the run view (milestone 8c decision 14).
#[derive(Debug, Clone, PartialEq)]
pub struct DisplayRound<'a> {
    pub info: &'a AgentRoundInfo,
    pub number: u32,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    /// The session's last display round: its counters and sub-agents hang here.
    pub last: bool,
    pub window: Option<&'a WindowInfo>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row<'a> {
    pub key: NodeKey,
    /// The box-drawing prefix to draw before the row's own marker, two columns
    /// per level. Empty on project rows, which are roots.
    pub guides: String,
    /// How far below a root this row sits: 0 for a project, 1 for a window, 2
    /// for one of that window's sub-agents, and one more per nested level.
    ///
    /// Carried explicitly rather than recovered from `guides`, whose width per
    /// level is the sidebar's business alone: the graph overview's tiers must
    /// not move when the guide alphabet changes.
    pub depth: u16,
    pub kind: RowKind<'a>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Viewport {
    pub top: usize,
    pub height: u16,
}

impl Viewport {
    pub fn reveal(&mut self, index: usize) {
        if index < self.top {
            self.top = index;
        } else if index >= self.top.saturating_add(usize::from(self.height)) {
            self.top = index
                .saturating_add(1)
                .saturating_sub(usize::from(self.height));
        }
    }

    pub fn scroll(&mut self, delta: isize, len: usize) {
        let max_top = len.saturating_sub(usize::from(self.height));
        let top = self.top.min(max_top);
        self.top = if delta < 0 {
            top.saturating_sub(delta.unsigned_abs())
        } else {
            top.saturating_add(delta as usize).min(max_top)
        };
    }
}

/// `config.toml`'s `ui.tree_keep_finished_secs` default (decision 4), and `TreeState`'s
/// own fallback when nothing else set it (task M4's `TreeState::default()` calls that
/// still need a tree, e.g. `App::focus_relative`'s wrap-around lookup, and every
/// existing test that predates `UiSettings`).
const DEFAULT_KEEP_FINISHED_SECS: u64 = 300;

#[derive(Debug, Clone)]
pub struct TreeState {
    pub collapsed: HashSet<NodeKey>,
    pub filter: String,
    pub selected: Option<NodeKey>,
    pub sidebar: Viewport,
    pub overview: Viewport,
    /// A finished (`Done` or `Failed`) sub-agent whose `ended_secs` exceeds this gets no
    /// row, and its descendants reattach to the nearest still-shown ancestor (task
    /// M6.9). Set from `UiSettings::tree_keep_finished_secs`, itself
    /// `config.toml`'s `ui.tree_keep_finished_secs`.
    pub keep_finished_secs: u64,
    selected_index: usize,
    /// The projects the shown runs name, as `prune_runs` last saw them, so `prune`
    /// keeps a `Project` key whose root has a run but no window (milestone 8c
    /// decision 7).
    run_roots: HashSet<PathBuf>,
}

impl Default for TreeState {
    fn default() -> Self {
        Self {
            collapsed: HashSet::new(),
            filter: String::new(),
            selected: None,
            sidebar: Viewport::default(),
            overview: Viewport::default(),
            keep_finished_secs: DEFAULT_KEEP_FINISHED_SECS,
            selected_index: 0,
            run_roots: HashSet::new(),
        }
    }
}

impl TreeState {
    pub fn is_collapsed(&self, key: &NodeKey) -> bool {
        self.collapsed.contains(key)
    }

    pub fn toggle(&mut self, key: &NodeKey) -> bool {
        match key {
            NodeKey::Project(_)
            | NodeKey::Window(_)
            | NodeKey::Run(_)
            | NodeKey::Planner { .. }
            | NodeKey::Scout { .. }
            | NodeKey::Task { .. }
            | NodeKey::Stage { .. }
            | NodeKey::AgentRound { .. } => {
                if !self.collapsed.remove(key) {
                    self.collapsed.insert(key.clone());
                }
                true
            }
            NodeKey::Subagent { .. } => false,
        }
    }

    pub fn select(&mut self, rows: &[Row<'_>], key: NodeKey) {
        if let Some(index) = row_index(rows, &key) {
            self.selected = Some(key);
            self.selected_index = index;
        }
    }

    pub fn move_selection(&mut self, rows: &[Row<'_>], delta: isize) {
        if rows.is_empty() {
            self.selected = None;
            self.selected_index = 0;
            return;
        }
        let Some(current) = self.selected_index(rows) else {
            self.selected = Some(rows[0].key.clone());
            self.selected_index = 0;
            return;
        };
        let last = rows.len() - 1;
        let next = if delta < 0 {
            current.saturating_sub(delta.unsigned_abs())
        } else {
            current.saturating_add(delta as usize).min(last)
        };
        self.selected = Some(rows[next].key.clone());
        self.selected_index = next;
    }

    pub fn repair_selection(&mut self, rows: &[Row<'_>]) {
        if rows.is_empty() {
            self.selected = None;
            self.selected_index = 0;
        } else if let Some(index) = self.selected_index(rows) {
            self.selected_index = index;
        } else if self.selected.is_some() {
            let index = self.selected_index.min(rows.len() - 1);
            self.selected = Some(rows[index].key.clone());
            self.selected_index = index;
        }
    }

    pub fn selected_index(&self, rows: &[Row<'_>]) -> Option<usize> {
        self.selected.as_ref().and_then(|key| row_index(rows, key))
    }

    /// Drops the fold state of windows and projects that left. A project survives while
    /// a window or a shown run names it; the run view's keys are `prune_runs`'s.
    pub fn prune(&mut self, windows: &[WindowInfo]) {
        let run_roots = &self.run_roots;
        self.collapsed.retain(|key| match key {
            NodeKey::Project(root) => {
                run_roots.contains(root) || windows.iter().any(|window| window.project == *root)
            }
            NodeKey::Window(id) => windows.iter().any(|window| window.id == *id),
            NodeKey::Subagent { .. } => false,
            NodeKey::Run(_)
            | NodeKey::Planner { .. }
            | NodeKey::Scout { .. }
            | NodeKey::Task { .. }
            | NodeKey::Stage { .. }
            | NodeKey::AgentRound { .. } => true,
        });
    }
}

struct ProjectGroup<'a> {
    root: &'a Path,
    name: String,
    status: Status,
    counts: RuntimeCounts,
    runs: Vec<ShownRun<'a>>,
    members: Vec<ProjectChild<'a>>,
}

/// The project tree of plain windows alone: `build_with_runs` with no runs.
pub fn build<'a>(windows: &'a [WindowInfo], state: &TreeState) -> Vec<Row<'a>> {
    build_with_runs(windows, &[], state)
}

/// The project tree: every project with its shown runs, then its plain windows
/// (milestone 8c decisions 6–10).
pub fn build_with_runs<'a>(
    windows: &'a [WindowInfo],
    runs: &'a [RunInfo],
    state: &TreeState,
) -> Vec<Row<'a>> {
    let projects = group_projects(windows, runs);

    let filter = state.filter.to_lowercase();
    let filtering = !filter.is_empty();
    let mut rows = Vec::new();
    let mut position = 0;
    for project in projects {
        let project_matches = filtering && matches_filter(&project.name, &filter);
        let project_has_match = project_matches
            || project
                .runs
                .iter()
                .any(|shown| run_matches_filter(shown.run, &filter))
            || project.members.iter().any(|member| match member {
                ProjectChild::Window(window) => window_matches_filter(window, &filter),
            });
        if filtering && !project_has_match {
            continue;
        }
        let project_key = NodeKey::Project(project.root.to_path_buf());
        let project_collapsed = !filtering && state.is_collapsed(&project_key);
        rows.push(Row {
            key: project_key,
            guides: String::new(),
            depth: 0,
            kind: RowKind::Project {
                root: project.root,
                name: project.name,
                status: project.status,
                counts: project.counts,
                collapsed: project_collapsed,
            },
        });
        if project_collapsed {
            continue;
        }
        let shown_runs: Vec<&ShownRun<'a>> = project
            .runs
            .iter()
            .filter(|shown| !filtering || project_matches || run_matches_filter(shown.run, &filter))
            .collect();
        let visible = visible_windows(
            project.members,
            filtering,
            project_matches,
            &filter,
            state.keep_finished_secs,
        );
        let count = shown_runs.len() + visible.len();
        for (index, shown) in shown_runs.into_iter().enumerate() {
            let position = shown.orchestrator.map(|_| {
                position += 1;
                position
            });
            rows.push(Row {
                key: NodeKey::Run(shown.run.run_id.clone()),
                guides: guide_prefix(&[], index + 1 < count),
                depth: 1,
                kind: RowKind::Run {
                    run: shown.run,
                    orchestrator: shown.orchestrator,
                    position,
                },
            });
        }
        let first_window = count - visible.len();
        for (index, member) in visible.into_iter().enumerate() {
            let has_later_sibling = first_window + index + 1 < count;
            let window = member.window;
            let window_key = NodeKey::Window(window.id);
            let window_collapsed = !filtering && state.is_collapsed(&window_key);
            position += 1;
            rows.push(Row {
                key: window_key,
                guides: guide_prefix(&[], has_later_sibling),
                depth: 1,
                kind: RowKind::Window {
                    info: window,
                    position,
                    has_subagents: !window.subagents.is_empty(),
                    collapsed: window_collapsed,
                },
            });
            if !window_collapsed {
                // The window's own bit opens the ancestor stack: its sub-agents
                // hang below it, and the stem continues only while it has a
                // later visible sibling.
                let mut ancestors = vec![has_later_sibling];
                emit_subagents(
                    &mut rows,
                    &SubagentWalk {
                        window,
                        filter: &filter,
                        show_all: member.show_all,
                    },
                    &member.forest,
                    &mut ancestors,
                    // A window's own sub-agents are two levels below a project.
                    2,
                    false,
                );
            }
        }
    }
    rows
}

fn window_matches_filter(window: &WindowInfo, filter: &str) -> bool {
    matches_filter(&window.name, filter)
        || window
            .subagents
            .iter()
            .any(|subagent| matches_subagent(subagent, filter))
}

fn subagent_branch_matches(node: &SubagentNode<'_>, filter: &str) -> bool {
    matches_subagent(node.info, filter)
        || node
            .children
            .iter()
            .any(|child| subagent_branch_matches(child, filter))
}

fn matches_subagent(subagent: &SubagentInfo, filter: &str) -> bool {
    matches_filter(&subagent.kind, filter)
        || subagent
            .label
            .as_deref()
            .is_some_and(|label| matches_filter(label, filter))
}

fn matches_filter(text: &str, filter: &str) -> bool {
    text.to_lowercase().contains(filter)
}

pub fn agent_order(rows: &[Row<'_>]) -> Vec<u32> {
    rows.iter()
        .filter_map(|row| match &row.kind {
            RowKind::Window { info, .. } => Some(info.id),
            RowKind::Run { orchestrator, .. } => orchestrator.map(|window| window.id),
            RowKind::Project { .. }
            | RowKind::Subagent { .. }
            | RowKind::Planner { .. }
            | RowKind::Scout { .. }
            | RowKind::Task { .. }
            | RowKind::Stage { .. }
            | RowKind::AgentRound { .. } => None,
        })
        .collect()
}

pub fn row_index(rows: &[Row<'_>], key: &NodeKey) -> Option<usize> {
    rows.iter().position(|row| &row.key == key)
}

pub fn urgency(status: Status) -> u8 {
    match status {
        Status::Attention => 0,
        Status::Working => 1,
        Status::Starting => 2,
        Status::Done => 3,
        Status::Idle => 4,
        Status::Exited => 5,
    }
}

/// A runtime's two-letter tag: `theme::runtime_tag`, the one table (final fix wave M5).
pub use crate::theme::runtime_tag;

pub fn short_model(runtime: Runtime, model: &str) -> String {
    let model = match runtime {
        Runtime::Claude => {
            let stripped = model.strip_prefix("claude-").unwrap_or(model);
            let numeric_suffix = stripped.char_indices().find_map(|(index, character)| {
                (character == '-'
                    && stripped[index + character.len_utf8()..]
                        .chars()
                        .next()
                        .is_some_and(|next| next.is_ascii_digit()))
                .then_some(index)
            });
            &stripped[..numeric_suffix.unwrap_or(stripped.len())]
        }
        Runtime::Codex | Runtime::Shell => model,
    };
    cut_to_width(model, 8)
}

fn cut_to_width(text: &str, max_width: usize) -> String {
    let mut end = 0;
    for (index, grapheme) in text.grapheme_indices(true) {
        let candidate_end = index + grapheme.len();
        if UnicodeWidthStr::width(&text[..candidate_end]) > max_width {
            break;
        }
        end = candidate_end;
    }
    text[..end].to_owned()
}

/// The text a sub-agent row shows in its name column: `kind: label` when a
/// label was set, `kind` alone otherwise.
///
/// Shared by the sidebar and the graph overview's layout and painter, so the
/// string a tier is sized to and the string drawn inside it can never drift
/// apart into two definitions.
pub fn subagent_label(info: &SubagentInfo) -> String {
    match info.label.as_deref() {
        Some(label) => format!("{}: {label}", info.kind),
        None => info.kind.clone(),
    }
}

pub fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) fn example_windows() -> Vec<WindowInfo> {
    tests::example()
}

#[cfg(test)]
pub(crate) use tests::{
    alert_fixtures, orch_fixtures, plan_fixtures, pr_fixtures, run_fixtures, stage_fixtures,
};
