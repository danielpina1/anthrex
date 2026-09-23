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
        worktree: None,
        branch: None,
        status,
        tool: None,
        since_secs,
        last_output_secs: 0,
        session_id: None,
        model: None,
        subagents: vec![],
        exit: None,
        kind: proto::WindowKind::Pty,
        run: None,
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

pub(super) fn example() -> Vec<WindowInfo> {
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

fn guide_strings<'a>(rows: &'a [Row<'_>]) -> Vec<&'a str> {
    rows.iter().map(|row| row.guides.as_str()).collect()
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

    assert_eq!(
        guide_strings(&rows),
        vec![
            "",         // shop
            "├─",       // 1 api-worker
            "│ ├─",     // a1
            "│ │ └─",   // a2, the last child of a1
            "│ └─",     // a3, the last root sub-agent
            "├─",       // 2 billing
            "├─",       // 3 search
            "├─",       // 4 frontend
            "│ └─",     // b1
            "│   └─",   // b2, below a last sibling
            "│     └─", // b3
            "├─",       // 5 docs
            "├─",       // 6 infra
            "└─",       // 7 perf, the last window of shop
            "",         // blog
            "└─",       // 8 notes
        ]
    );
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
    let forest = subagent_forest(&subagents, 300);
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
    let forest = subagent_forest(&subagents, 300);

    assert_eq!(
        forest
            .iter()
            .map(|node| node.info.id.as_str())
            .collect::<Vec<_>>(),
        vec!["old", "middle", "young"]
    );
}

/// Task M6.9: a finished sub-agent's row disappears once it has been finished longer
/// than `keep_finished_secs`; a still-`Running` one never does, whatever its
/// `started_secs`.
#[test]
fn finished_subagents_older_than_the_setting_are_hidden() {
    let mut worker = window(1, "/p", "worker", Runtime::Shell, Status::Idle, 0);
    worker.subagents = vec![
        finished_subagent("old", None, SubagentState::Done, 150),
        finished_subagent("recent", None, SubagentState::Done, 60),
        subagent(
            "running",
            None,
            "agent",
            "running",
            SubagentState::Running,
            10,
        ),
    ];
    let windows = vec![worker.clone()];

    let strict = TreeState {
        keep_finished_secs: 120,
        ..TreeState::default()
    };
    let keys = row_keys(&build(&windows, &strict));
    assert!(!keys.contains(&subagent_key(1, "old")), "{keys:?}");
    assert!(keys.contains(&subagent_key(1, "recent")), "{keys:?}");
    assert!(keys.contains(&subagent_key(1, "running")), "{keys:?}");

    // The default (300) is above both finished ages, so both show.
    let keys = row_keys(&build(&windows, &TreeState::default()));
    assert!(keys.contains(&subagent_key(1, "old")), "{keys:?}");
    assert!(keys.contains(&subagent_key(1, "recent")), "{keys:?}");
}

/// Hazard: hiding a finished sub-agent must not drop its own children with it. Three
/// levels — a shown window, a hidden finished sub-agent, and a still-`Running`
/// sub-agent beneath it — because a hidden node with no children of its own can't tell
/// "descendants reattached" from "descendants dropped".
#[test]
fn hidden_finished_subagents_reattach_their_running_descendants() {
    let mut worker = window(1, "/p", "worker", Runtime::Shell, Status::Idle, 0);
    worker.subagents = vec![
        finished_subagent("parent", None, SubagentState::Done, 200),
        subagent(
            "child",
            Some("parent"),
            "agent",
            "child",
            SubagentState::Running,
            10,
        ),
    ];
    let windows = vec![worker];

    let state = TreeState {
        keep_finished_secs: 120,
        ..TreeState::default()
    };
    let keys = row_keys(&build(&windows, &state));
    assert!(!keys.contains(&subagent_key(1, "parent")), "{keys:?}");
    assert!(
        keys.contains(&subagent_key(1, "child")),
        "the running child must survive its hidden parent: {keys:?}"
    );
}

fn finished_subagent(
    id: &str,
    parent_id: Option<&str>,
    state: SubagentState,
    ended_secs: u64,
) -> SubagentInfo {
    SubagentInfo {
        ended_secs: Some(ended_secs),
        ..subagent(id, parent_id, "agent", id, state, 0)
    }
}

fn subagent_key(window_id: u32, id: &str) -> NodeKey {
    NodeKey::Subagent {
        window_id,
        id: id.to_string(),
    }
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
fn display_names_resolve_collisions_across_basename_groups() {
    let names = display_names([
        Path::new("/w/work/api"),
        Path::new("/w/oss/api"),
        Path::new("/w/api (work)"),
    ]);

    assert_eq!(names[Path::new("/w/work/api")], "/w/work/api");
    assert_eq!(names[Path::new("/w/oss/api")], "api (oss)");
    assert_eq!(names[Path::new("/w/api (work)")], "/w/api (work)");
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
fn short_model_keeps_grapheme_sequences_within_eight_columns() {
    assert_eq!(short_model(Runtime::Codex, "1234567✈️"), "1234567");
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
mod state;
