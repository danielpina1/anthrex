use proto::{Runtime, Status, SubagentInfo, WindowInfo};
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
        window: &'a WindowInfo,
        info: &'a SubagentInfo,
        guides: String,
    },
    // Milestone 8 adds Run { .. }.
}

#[derive(Debug, Clone, PartialEq)]
pub struct Row<'a> {
    pub key: NodeKey,
    /// Columns of indentation before the row's own content. Project 0, window 2,
    /// sub-agent 2 (its guide string starts with the window stem).
    pub indent: u16,
    pub kind: RowKind<'a>,
}

#[derive(Debug, Clone, Default)]
pub struct TreeState {
    _reserved: (),
}

pub struct SubagentNode<'a> {
    pub info: &'a SubagentInfo,
    pub children: Vec<SubagentNode<'a>>,
}

struct ProjectGroup<'a> {
    root: &'a Path,
    name: String,
    status: Status,
    counts: RuntimeCounts,
    members: Vec<ProjectChild<'a>>,
}

pub fn build<'a>(windows: &'a [WindowInfo], _state: &TreeState) -> Vec<Row<'a>> {
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

    let mut rows = Vec::new();
    let mut position = 0;
    for project in projects {
        rows.push(Row {
            key: NodeKey::Project(project.root.to_path_buf()),
            indent: 0,
            kind: RowKind::Project {
                root: project.root,
                name: project.name,
                status: project.status,
                counts: project.counts,
                collapsed: false,
            },
        });
        for member in project.members {
            match member {
                ProjectChild::Window(window) => {
                    position += 1;
                    rows.push(Row {
                        key: NodeKey::Window(window.id),
                        indent: 2,
                        kind: RowKind::Window {
                            info: window,
                            position,
                            has_subagents: !window.subagents.is_empty(),
                            collapsed: false,
                        },
                    });
                    let forest = subagent_forest(&window.subagents);
                    emit_subagent_rows(&mut rows, window, &forest, &mut Vec::new());
                }
            }
        }
    }
    rows
}

fn emit_subagent_rows<'a>(
    rows: &mut Vec<Row<'a>>,
    window: &'a WindowInfo,
    nodes: &[SubagentNode<'a>],
    ancestor_has_later_sibling: &mut Vec<bool>,
) {
    for (index, node) in nodes.iter().enumerate() {
        let has_later_sibling = index + 1 < nodes.len();
        let mut guides = String::from("│ ");
        for ancestor_has_later in ancestor_has_later_sibling.iter().copied() {
            guides.push_str(if ancestor_has_later { "│ " } else { "  " });
        }
        guides.push_str(if has_later_sibling { "├ " } else { "└ " });
        rows.push(Row {
            key: NodeKey::Subagent {
                window_id: window.id,
                id: node.info.id.clone(),
            },
            indent: 2,
            kind: RowKind::Subagent {
                window,
                info: node.info,
                guides,
            },
        });
        ancestor_has_later_sibling.push(has_later_sibling);
        emit_subagent_rows(rows, window, &node.children, ancestor_has_later_sibling);
        ancestor_has_later_sibling.pop();
    }
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

pub fn display_names<'a>(roots: impl IntoIterator<Item = &'a Path>) -> HashMap<PathBuf, String> {
    let roots: Vec<PathBuf> = roots
        .into_iter()
        .map(Path::to_path_buf)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    let mut by_base: HashMap<String, Vec<&PathBuf>> = HashMap::new();
    for root in &roots {
        by_base.entry(base_name(root)).or_default().push(root);
    }

    let mut names = HashMap::new();
    for (base, group) in by_base {
        if group.len() == 1 {
            names.insert(group[0].clone(), base);
            continue;
        }

        let candidates: Vec<_> = group
            .iter()
            .map(|root| {
                let parent = root
                    .parent()
                    .and_then(Path::file_name)
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| root.display().to_string());
                format!("{base} ({parent})")
            })
            .collect();
        let mut candidate_counts: HashMap<String, usize> = HashMap::new();
        for candidate in &candidates {
            *candidate_counts.entry(candidate.clone()).or_default() += 1;
        }
        for (root, candidate) in group.into_iter().zip(candidates) {
            let name = if candidate_counts[&candidate] > 1 {
                root.display().to_string()
            } else {
                candidate
            };
            names.insert(root.clone(), name);
        }
    }
    let mut name_counts: HashMap<String, usize> = HashMap::new();
    for name in names.values() {
        *name_counts.entry(name.clone()).or_default() += 1;
    }
    for (root, name) in &mut names {
        if name_counts[name] > 1 {
            *name = root.display().to_string();
        }
    }
    names
}

fn base_name(root: &Path) -> String {
    root.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| root.display().to_string())
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

pub fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

pub fn subagent_forest(subagents: &[SubagentInfo]) -> Vec<SubagentNode<'_>> {
    let index_by_id: HashMap<&str, usize> = subagents
        .iter()
        .enumerate()
        .map(|(index, info)| (info.id.as_str(), index))
        .collect();
    let parent_indices: Vec<_> = subagents
        .iter()
        .map(|info| {
            info.parent_id
                .as_deref()
                .and_then(|parent_id| index_by_id.get(parent_id).copied())
        })
        .collect();

    let mut cycle_members = HashSet::new();
    for start in 0..subagents.len() {
        let mut path = Vec::new();
        let mut path_positions = HashMap::new();
        let mut current = Some(start);
        while let Some(index) = current {
            if let Some(cycle_start) = path_positions.get(&index).copied() {
                cycle_members.extend(path[cycle_start..].iter().copied());
                break;
            }
            path_positions.insert(index, path.len());
            path.push(index);
            current = parent_indices[index];
        }
    }

    let mut roots = Vec::new();
    let mut children = vec![Vec::new(); subagents.len()];
    for (index, parent) in parent_indices.into_iter().enumerate() {
        if cycle_members.contains(&index) {
            roots.push(index);
        } else if let Some(parent) = parent {
            children[parent].push(index);
        } else {
            roots.push(index);
        }
    }
    sort_subagents(&mut roots, subagents);
    for siblings in &mut children {
        sort_subagents(siblings, subagents);
    }
    roots
        .into_iter()
        .map(|index| make_subagent_node(index, subagents, &children))
        .collect()
}

fn sort_subagents(indices: &mut [usize], subagents: &[SubagentInfo]) {
    indices.sort_by(|left, right| {
        subagents[*right]
            .started_secs
            .cmp(&subagents[*left].started_secs)
            .then_with(|| subagents[*left].id.cmp(&subagents[*right].id))
    });
}

fn make_subagent_node<'a>(
    index: usize,
    subagents: &'a [SubagentInfo],
    children: &[Vec<usize>],
) -> SubagentNode<'a> {
    SubagentNode {
        info: &subagents[index],
        children: children[index]
            .iter()
            .map(|child| make_subagent_node(*child, subagents, children))
            .collect(),
    }
}

#[cfg(test)]
mod tests;
