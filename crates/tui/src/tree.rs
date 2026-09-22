mod forest;
mod names;
mod rows;

use proto::{Runtime, Status, SubagentInfo, WindowInfo};
pub use forest::{SubagentNode, subagent_forest};
pub use names::display_names;
use rows::{SubagentWalk, emit_subagents, guide_prefix, visible_windows};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NodeKey {
    Project(PathBuf),
    Window(u32),
    Subagent { window_id: u32, id: String },
    // Milestone 8 adds Run(..): see decision 26.
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RuntimeCounts {
    pub claude: usize,
    pub codex: usize,
    pub shell: usize,
}

pub enum ProjectChild<'a> {
    Window(&'a WindowInfo),
    // Milestone 8 adds Run { .. }: run rows sit above plain windows and own their windows (spec 5.1 item 4).
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
    // Milestone 8 adds Run { .. }.
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
        }
    }
}

impl TreeState {
    pub fn is_collapsed(&self, key: &NodeKey) -> bool {
        self.collapsed.contains(key)
    }

    pub fn toggle(&mut self, key: &NodeKey) -> bool {
        match key {
            NodeKey::Project(_) | NodeKey::Window(_) => {
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

    pub fn prune(&mut self, windows: &[WindowInfo]) {
        self.collapsed.retain(|key| match key {
            NodeKey::Project(root) => windows.iter().any(|window| window.project == *root),
            NodeKey::Window(id) => windows.iter().any(|window| window.id == *id),
            NodeKey::Subagent { .. } => false,
        });
    }
}

struct ProjectGroup<'a> {
    root: &'a Path,
    name: String,
    status: Status,
    counts: RuntimeCounts,
    members: Vec<ProjectChild<'a>>,
}

pub fn build<'a>(windows: &'a [WindowInfo], state: &TreeState) -> Vec<Row<'a>> {
    let mut by_root: HashMap<&'a Path, Vec<&'a WindowInfo>> = HashMap::new();
    for window in windows {
        by_root
            .entry(window.project.as_path())
            .or_default()
            .push(window);
    }

    let names = display_names(by_root.keys().copied());
    let mut projects: Vec<_> = by_root
        .into_iter()
        .map(|(root, mut windows)| {
            windows.sort_by_key(|window| window.id);
            let status = windows
                .iter()
                .map(|window| window.status)
                .min_by_key(|status| urgency(*status))
                .expect("a project is created from at least one window");
            let mut counts = RuntimeCounts::default();
            for window in &windows {
                match window.runtime {
                    Runtime::Claude => counts.claude += 1,
                    Runtime::Codex => counts.codex += 1,
                    Runtime::Shell => counts.shell += 1,
                }
            }
            let members = windows.into_iter().map(ProjectChild::Window).collect();
            ProjectGroup {
                root,
                name: names
                    .get(root)
                    .expect("every grouped root has a display name")
                    .clone(),
                status,
                counts,
                members,
            }
        })
        .collect();
    projects.sort_by(|left, right| {
        urgency(left.status)
            .cmp(&urgency(right.status))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.root.cmp(right.root))
    });

    let filter = state.filter.to_lowercase();
    let filtering = !filter.is_empty();
    let mut rows = Vec::new();
    let mut position = 0;
    for project in projects {
        let project_matches = filtering && matches_filter(&project.name, &filter);
        let project_has_match = project_matches
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
        let visible = visible_windows(
            project.members,
            filtering,
            project_matches,
            &filter,
            state.keep_finished_secs,
        );
        let count = visible.len();
        for (index, member) in visible.into_iter().enumerate() {
            let has_later_sibling = index + 1 < count;
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
            RowKind::Project { .. } | RowKind::Subagent { .. } => None,
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

pub fn runtime_tag(runtime: Runtime) -> &'static str {
    match runtime {
        Runtime::Claude => "cl",
        Runtime::Codex => "cx",
        Runtime::Shell => "sh",
    }
}

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
