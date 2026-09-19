use proto::{Runtime, Status, SubagentInfo, WindowInfo};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use unicode_width::UnicodeWidthChar;

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
    let mut result = String::new();
    let mut width = 0;
    for character in text.chars() {
        let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
        if width + character_width > max_width {
            break;
        }
        result.push(character);
        width += character_width;
    }
    result
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
mod tests {
    use super::*;
    use proto::{Runtime, Status, SubagentInfo, SubagentState, WindowInfo};
    use std::path::{Path, PathBuf};

    fn window(
        id: u32,
        project: &str,
        name: &str,
        runtime: Runtime,
        status: Status,
        since_secs: u64,
    ) -> WindowInfo {
        WindowInfo {
            id,
            name: name.into(),
            runtime,
            cwd: project.into(),
            project: project.into(),
            branch: None,
            status,
            tool: None,
            since_secs,
            last_output_secs: 0,
            session_id: None,
            model: None,
            subagents: vec![],
            exit: None,
        }
    }

    fn subagent(
        id: &str,
        parent_id: Option<&str>,
        kind: &str,
        label: &str,
        state: SubagentState,
        started_secs: u64,
    ) -> SubagentInfo {
        SubagentInfo {
            id: id.into(),
            parent_id: parent_id.map(str::to_owned),
            kind: kind.into(),
            label: Some(label.into()),
            model: None,
            state,
            tool: None,
            started_secs,
            ended_secs: None,
            needs_permission: false,
        }
    }

    fn example() -> Vec<WindowInfo> {
        let mut api = window(
            1,
            "/r/shop",
            "api-worker",
            Runtime::Claude,
            Status::Working,
            120,
        );
        api.model = Some("claude-opus-5".into());
        let mut a1 = subagent(
            "a1",
            None,
            "Explore",
            "map routes",
            SubagentState::Running,
            90,
        );
        a1.tool = Some("Read".into());
        let a2 = subagent(
            "a2",
            Some("a1"),
            "general-purpose",
            "grep handlers",
            SubagentState::Running,
            20,
        );
        let mut a3 = subagent(
            "a3",
            None,
            "tests",
            "run unit suite",
            SubagentState::Done,
            60,
        );
        a3.ended_secs = Some(15);
        api.subagents = vec![a1, a2, a3];

        let billing = window(
            2,
            "/r/shop",
            "billing",
            Runtime::Codex,
            Status::Attention,
            41,
        );
        let search = window(3, "/r/shop", "search", Runtime::Codex, Status::Idle, 300);

        let mut frontend = window(
            4,
            "/r/shop",
            "frontend",
            Runtime::Claude,
            Status::Working,
            60,
        );
        frontend.model = Some("claude-sonnet-4-5".into());
        let mut b1 = subagent(
            "b1",
            None,
            "general-purpose",
            "style pass",
            SubagentState::Running,
            50,
        );
        b1.tool = Some("Edit".into());
        let b2 = subagent(
            "b2",
            Some("b1"),
            "Explore",
            "find tokens",
            SubagentState::Running,
            30,
        );
        let mut b3 = subagent(
            "b3",
            Some("b2"),
            "Explore",
            "list files",
            SubagentState::Done,
            25,
        );
        b3.ended_secs = Some(5);
        frontend.subagents = vec![b1, b2, b3];

        let docs = window(5, "/r/shop", "docs", Runtime::Claude, Status::Done, 180);
        let infra = window(6, "/r/shop", "infra", Runtime::Codex, Status::Idle, 720);
        let perf = window(7, "/r/shop", "perf", Runtime::Claude, Status::Starting, 2);
        let notes = window(8, "/r/blog", "notes", Runtime::Claude, Status::Idle, 30);

        vec![api, billing, search, frontend, docs, infra, perf, notes]
    }

    fn row_keys(rows: &[Row<'_>]) -> Vec<NodeKey> {
        rows.iter().map(|row| row.key.clone()).collect()
    }

    #[test]
    fn example_rows_in_order() {
        let windows = example();
        let rows = build(&windows, &TreeState::default());

        assert_eq!(
            row_keys(&rows),
            vec![
                NodeKey::Project("/r/shop".into()),
                NodeKey::Window(1),
                NodeKey::Subagent {
                    window_id: 1,
                    id: "a1".into(),
                },
                NodeKey::Subagent {
                    window_id: 1,
                    id: "a2".into(),
                },
                NodeKey::Subagent {
                    window_id: 1,
                    id: "a3".into(),
                },
                NodeKey::Window(2),
                NodeKey::Window(3),
                NodeKey::Window(4),
                NodeKey::Subagent {
                    window_id: 4,
                    id: "b1".into(),
                },
                NodeKey::Subagent {
                    window_id: 4,
                    id: "b2".into(),
                },
                NodeKey::Subagent {
                    window_id: 4,
                    id: "b3".into(),
                },
                NodeKey::Window(5),
                NodeKey::Window(6),
                NodeKey::Window(7),
                NodeKey::Project("/r/blog".into()),
                NodeKey::Window(8),
            ]
        );
    }

    #[test]
    fn project_row_has_status_counts_and_name() {
        let windows = example();
        let rows = build(&windows, &TreeState::default());

        let RowKind::Project {
            name,
            status,
            counts,
            ..
        } = &rows[0].kind
        else {
            panic!("first row should be the shop project");
        };
        assert_eq!(name, "shop");
        assert_eq!(*status, Status::Attention);
        assert_eq!(
            *counts,
            RuntimeCounts {
                claude: 4,
                codex: 3,
                shell: 0,
            }
        );

        let blog = rows
            .iter()
            .find(|row| row.key == NodeKey::Project("/r/blog".into()))
            .expect("blog row");
        let RowKind::Project {
            name,
            status,
            counts,
            ..
        } = &blog.kind
        else {
            panic!("blog key should identify a project row");
        };
        assert_eq!(name, "blog");
        assert_eq!(*status, Status::Idle);
        assert_eq!(
            *counts,
            RuntimeCounts {
                claude: 1,
                codex: 0,
                shell: 0,
            }
        );
    }

    #[test]
    fn positions_follow_visible_order() {
        let windows = example();
        let rows = build(&windows, &TreeState::default());
        let positions: Vec<_> = rows
            .iter()
            .filter_map(|row| match &row.kind {
                RowKind::Window { position, .. } => Some(*position),
                RowKind::Project { .. } | RowKind::Subagent { .. } => None,
            })
            .collect();

        assert_eq!(positions, (1..=8).collect::<Vec<_>>());
        assert_eq!(agent_order(&rows), vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn guides_draw_the_nesting() {
        let windows = example();
        let rows = build(&windows, &TreeState::default());
        let guides: Vec<_> = rows
            .iter()
            .filter_map(|row| match &row.kind {
                RowKind::Subagent { info, guides, .. } => Some((info.id.as_str(), guides.as_str())),
                RowKind::Project { .. } | RowKind::Window { .. } => None,
            })
            .collect();

        assert_eq!(
            guides,
            vec![
                ("a1", "│ ├ "),
                ("a2", "│ │ └ "),
                ("a3", "│ └ "),
                ("b1", "│ └ "),
                ("b2", "│   └ "),
                ("b3", "│     └ "),
            ]
        );
        for row in rows {
            let expected = match row.kind {
                RowKind::Project { .. } => 0,
                RowKind::Window { .. } | RowKind::Subagent { .. } => 2,
            };
            assert_eq!(row.indent, expected, "wrong indent for {:?}", row.key);
        }
    }

    #[test]
    fn projects_sort_by_urgency_then_name() {
        let windows = vec![
            window(1, "/x/alpha", "a", Runtime::Shell, Status::Idle, 0),
            window(2, "/x/zeta", "z", Runtime::Shell, Status::Working, 0),
            window(3, "/x/beta", "b", Runtime::Shell, Status::Working, 0),
            window(4, "/x/omega", "o", Runtime::Shell, Status::Exited, 0),
        ];
        let rows = build(&windows, &TreeState::default());
        let projects: Vec<_> = rows
            .iter()
            .filter_map(|row| match &row.kind {
                RowKind::Project { name, .. } => Some(name.as_str()),
                RowKind::Window { .. } | RowKind::Subagent { .. } => None,
            })
            .collect();

        assert_eq!(projects, vec!["beta", "zeta", "alpha", "omega"]);
    }

    #[test]
    fn windows_sort_by_id_inside_a_project() {
        let windows = vec![
            window(5, "/p", "five", Runtime::Shell, Status::Idle, 0),
            window(2, "/p", "two", Runtime::Shell, Status::Idle, 0),
            window(9, "/p", "nine", Runtime::Shell, Status::Idle, 0),
        ];
        let rows = build(&windows, &TreeState::default());

        assert_eq!(agent_order(&rows), vec![2, 5, 9]);
    }

    #[test]
    fn orphans_and_cycles_become_roots() {
        let subagents = vec![
            subagent(
                "orphan",
                Some("missing"),
                "agent",
                "orphan",
                SubagentState::Running,
                40,
            ),
            subagent(
                "cycle-a",
                Some("cycle-b"),
                "agent",
                "cycle a",
                SubagentState::Running,
                30,
            ),
            subagent(
                "cycle-b",
                Some("cycle-a"),
                "agent",
                "cycle b",
                SubagentState::Running,
                20,
            ),
        ];
        let forest = subagent_forest(&subagents);
        let roots: Vec<_> = forest.iter().map(|node| node.info.id.as_str()).collect();

        assert_eq!(roots, vec!["orphan", "cycle-a", "cycle-b"]);
        assert!(forest.iter().all(|node| node.children.is_empty()));
    }

    #[test]
    fn siblings_sort_oldest_first() {
        let subagents = vec![
            subagent("young", None, "agent", "young", SubagentState::Running, 10),
            subagent("old", None, "agent", "old", SubagentState::Running, 50),
            subagent(
                "middle",
                None,
                "agent",
                "middle",
                SubagentState::Running,
                30,
            ),
        ];
        let forest = subagent_forest(&subagents);

        assert_eq!(
            forest
                .iter()
                .map(|node| node.info.id.as_str())
                .collect::<Vec<_>>(),
            vec!["old", "middle", "young"]
        );
    }

    #[test]
    fn display_names_disambiguate() {
        let roots = [
            Path::new("/w/work/api"),
            Path::new("/w/oss/api"),
            Path::new("/w/web"),
        ];
        let names = display_names(roots);
        assert_eq!(names[Path::new("/w/work/api")], "api (work)");
        assert_eq!(names[Path::new("/w/oss/api")], "api (oss)");
        assert_eq!(names[Path::new("/w/web")], "web");

        let colliding = display_names([Path::new("/a/x/api"), Path::new("/b/x/api")]);
        assert_eq!(colliding[Path::new("/a/x/api")], "/a/x/api");
        assert_eq!(colliding[Path::new("/b/x/api")], "/b/x/api");

        let root = display_names([Path::new("/")]);
        assert_eq!(root[Path::new("/")], "/");
    }

    #[test]
    fn short_model_shortens_claude_ids_only() {
        assert_eq!(short_model(Runtime::Claude, "claude-opus-5"), "opus");
        assert_eq!(short_model(Runtime::Claude, "claude-sonnet-4-5"), "sonnet");
        assert_eq!(
            short_model(Runtime::Claude, "claude-haiku-4-5-20251001"),
            "haiku"
        );
        assert_eq!(short_model(Runtime::Codex, "gpt-5.1-codex-max"), "gpt-5.1-");
    }

    #[test]
    fn urgency_order_matches_the_spec() {
        assert_eq!(urgency(Status::Attention), 0);
        assert_eq!(urgency(Status::Working), 1);
        assert_eq!(urgency(Status::Starting), 2);
        assert_eq!(urgency(Status::Done), 3);
        assert_eq!(urgency(Status::Idle), 4);
        assert_eq!(urgency(Status::Exited), 5);
    }

    #[test]
    fn row_index_finds_keys_and_runtime_tags_are_fixed() {
        let windows = example();
        let rows = build(&windows, &TreeState::default());

        assert_eq!(row_index(&rows, &NodeKey::Window(4)), Some(7));
        assert_eq!(row_index(&rows, &NodeKey::Window(99)), None);
        assert_eq!(runtime_tag(Runtime::Claude), "cl");
        assert_eq!(runtime_tag(Runtime::Codex), "cx");
        assert_eq!(runtime_tag(Runtime::Shell), "sh");
    }

    #[test]
    fn elapsed_time_uses_existing_sidebar_units() {
        assert_eq!(format_elapsed(0), "0s");
        assert_eq!(format_elapsed(59), "59s");
        assert_eq!(format_elapsed(60), "1m");
        assert_eq!(format_elapsed(3599), "59m");
        assert_eq!(format_elapsed(3600), "1h");
    }

    #[test]
    fn node_keys_are_hashable_path_owners() {
        let key = NodeKey::Project(PathBuf::from("/r/shop"));
        let set = std::collections::HashSet::from([key.clone()]);

        assert!(set.contains(&key));
    }
}
