use super::*;
use unicode_width::UnicodeWidthStr;

/// A window with sub-agents, so a test can place it anywhere among its siblings.
fn window_with_subagents(id: u32, name: &str, subagents: Vec<SubagentInfo>) -> WindowInfo {
    let mut window = window(id, "/p", name, Runtime::Claude, Status::Idle, 0);
    window.subagents = subagents;
    window
}

/// Two root sub-agents, the first of which has one child.
fn forked_subagents() -> Vec<SubagentInfo> {
    vec![
        subagent("s1", None, "Explore", "map", SubagentState::Running, 90),
        subagent(
            "s2",
            Some("s1"),
            "general-purpose",
            "grep",
            SubagentState::Running,
            60,
        ),
        subagent("s3", None, "tests", "suite", SubagentState::Running, 30),
    ]
}

#[test]
fn project_rows_have_no_guides() {
    let windows = example();
    let rows = build(&windows, &TreeState::default());
    let projects: Vec<_> = rows
        .iter()
        .filter(|row| matches!(row.kind, RowKind::Project { .. }))
        .collect();

    assert_eq!(projects.len(), 2);
    for row in projects {
        assert_eq!(row.guides, "", "project {:?} is a root", row.key);
    }
    for row in rows
        .iter()
        .filter(|row| !matches!(row.kind, RowKind::Project { .. }))
    {
        assert!(
            !row.guides.is_empty(),
            "{:?} hangs below a project",
            row.key
        );
    }
}

#[test]
fn windows_use_tee_and_elbow() {
    let windows = vec![
        window(1, "/p", "one", Runtime::Shell, Status::Idle, 0),
        window(2, "/p", "two", Runtime::Shell, Status::Idle, 0),
        window(3, "/p", "three", Runtime::Shell, Status::Idle, 0),
    ];

    assert_eq!(
        guide_strings(&build(&windows, &TreeState::default())),
        vec!["", "├─", "├─", "└─"]
    );
}

#[test]
fn subagents_nest_under_a_non_final_window() {
    let windows = vec![
        window_with_subagents(1, "first", forked_subagents()),
        window(2, "/p", "last", Runtime::Shell, Status::Idle, 0),
    ];
    let rows = build(&windows, &TreeState::default());

    assert_eq!(
        guide_strings(&rows),
        vec!["", "├─", "│ ├─", "│ │ └─", "│ └─", "└─"]
    );
    for row in &rows[2..5] {
        assert!(
            row.guides.starts_with("│ "),
            "{:?} hangs below a window with a later sibling",
            row.key
        );
    }
}

#[test]
fn subagents_nest_under_the_final_window() {
    let windows = vec![
        window(1, "/p", "first", Runtime::Shell, Status::Idle, 0),
        window_with_subagents(2, "last", forked_subagents()),
    ];
    let rows = build(&windows, &TreeState::default());

    assert_eq!(
        guide_strings(&rows),
        vec!["", "├─", "└─", "  ├─", "  │ └─", "  └─"]
    );
    for row in &rows[3..] {
        assert!(
            row.guides.starts_with("  "),
            "{:?} hangs below the last window",
            row.key
        );
    }
}

#[test]
fn deep_nesting_is_unbounded() {
    let mut chain = vec![subagent(
        "c1",
        None,
        "level",
        "one",
        SubagentState::Running,
        100,
    )];
    for level in 2..=5u64 {
        chain.push(subagent(
            &format!("c{level}"),
            Some(&format!("c{}", level - 1)),
            "level",
            &level.to_string(),
            SubagentState::Running,
            100 - level,
        ));
    }
    chain.push(subagent(
        "tail",
        None,
        "level",
        "tail",
        SubagentState::Running,
        10,
    ));
    let windows = vec![
        window_with_subagents(1, "deep", chain),
        window(2, "/p", "sibling", Runtime::Shell, Status::Idle, 0),
    ];
    let rows = build(&windows, &TreeState::default());

    assert_eq!(
        guide_strings(&rows),
        vec![
            "",             // project
            "├─",           // deep
            "│ ├─",         // c1
            "│ │ └─",       // c2
            "│ │   └─",     // c3
            "│ │     └─",   // c4
            "│ │       └─", // c5, six levels below the project
            "│ └─",         // tail
            "└─",           // sibling
        ]
    );
    assert_eq!(UnicodeWidthStr::width(rows[6].guides.as_str()), 12);
}

#[test]
fn collapse_changes_the_guides_of_the_row_above() {
    let windows = vec![
        window(1, "/p", "first", Runtime::Shell, Status::Idle, 0),
        window_with_subagents(
            2,
            "last",
            vec![subagent(
                "s1",
                None,
                "Explore",
                "map",
                SubagentState::Running,
                10,
            )],
        ),
    ];
    let mut state = TreeState::default();
    assert_eq!(
        guide_strings(&build(&windows, &state)),
        vec!["", "├─", "└─", "  └─"]
    );

    assert!(state.toggle(&NodeKey::Window(2)));
    assert_eq!(
        guide_strings(&build(&windows, &state)),
        vec!["", "├─", "└─"],
        "collapsing the last window drops its children and leaves its siblings alone"
    );

    let projects = vec![
        window(1, "/a", "a", Runtime::Shell, Status::Idle, 0),
        window(2, "/b", "b", Runtime::Shell, Status::Idle, 0),
        window(3, "/c", "c", Runtime::Shell, Status::Idle, 0),
    ];
    let mut state = TreeState::default();
    assert_eq!(
        guide_strings(&build(&projects, &state)),
        vec!["", "└─", "", "└─", "", "└─"]
    );

    assert!(state.toggle(&NodeKey::Project("/b".into())));
    assert_eq!(
        guide_strings(&build(&projects, &state)),
        vec!["", "└─", "", "", "└─"],
        "collapsing a middle project leaves the projects after it unchanged"
    );
}

#[test]
fn filtering_the_last_sibling_promotes_the_one_above() {
    let windows = vec![
        window(1, "/p", "api", Runtime::Shell, Status::Idle, 0),
        window(2, "/p", "api-worker", Runtime::Shell, Status::Idle, 0),
        window(3, "/p", "docs", Runtime::Shell, Status::Idle, 0),
    ];
    assert_eq!(
        guide_strings(&build(&windows, &TreeState::default())),
        vec!["", "├─", "├─", "└─"]
    );

    let state = TreeState {
        filter: "api".into(),
        ..TreeState::default()
    };
    let rows = build(&windows, &state);

    assert_eq!(
        row_keys(&rows),
        vec![
            NodeKey::Project("/p".into()),
            NodeKey::Window(1),
            NodeKey::Window(2),
        ]
    );
    assert_eq!(
        guide_strings(&rows),
        vec!["", "├─", "└─"],
        "the filter hid the last window, so the one above it is now last"
    );
}

/// The same promotion one level down, where the window itself survives only
/// because a sub-agent matched — so its other sub-agents really are dropped.
#[test]
fn filtering_the_last_subagent_promotes_the_one_above() {
    let windows = vec![window_with_subagents(
        1,
        "worker",
        vec![
            subagent("s1", None, "Explore", "map", SubagentState::Running, 90),
            subagent("s2", None, "Explore", "tokens", SubagentState::Running, 60),
            subagent("s3", None, "tests", "suite", SubagentState::Running, 30),
        ],
    )];
    assert_eq!(
        guide_strings(&build(&windows, &TreeState::default())),
        vec!["", "└─", "  ├─", "  ├─", "  └─"]
    );

    let state = TreeState {
        filter: "explore".into(),
        ..TreeState::default()
    };
    let rows = build(&windows, &state);

    assert_eq!(
        row_keys(&rows),
        vec![
            NodeKey::Project("/p".into()),
            NodeKey::Window(1),
            NodeKey::Subagent {
                window_id: 1,
                id: "s1".into(),
            },
            NodeKey::Subagent {
                window_id: 1,
                id: "s2".into(),
            },
        ],
        "neither the project nor the window matched, so only the matching sub-agents show"
    );
    assert_eq!(
        guide_strings(&rows),
        vec!["", "└─", "  ├─", "  └─"],
        "the filter hid the last sub-agent, so the one above it is now last"
    );
}

#[test]
fn guides_are_two_columns_per_level() {
    let windows = vec![window_with_subagents(
        1,
        "deep",
        vec![
            subagent("l2", None, "level", "two", SubagentState::Running, 30),
            subagent(
                "l3",
                Some("l2"),
                "level",
                "three",
                SubagentState::Running,
                20,
            ),
            subagent(
                "l4",
                Some("l3"),
                "level",
                "four",
                SubagentState::Running,
                10,
            ),
        ],
    )];
    let rows = build(&windows, &TreeState::default());

    assert_eq!(
        guide_strings(&rows),
        vec!["", "└─", "  └─", "    └─", "      └─"]
    );
    assert_eq!(
        rows.iter()
            .map(|row| UnicodeWidthStr::width(row.guides.as_str()))
            .collect::<Vec<_>>(),
        vec![0, 2, 4, 6, 8],
        "levels 1 to 4 are two columns each"
    );
}

#[test]
fn collapsed_project_shows_only_its_row() {
    let windows = example();
    let mut state = TreeState::default();
    let blog = NodeKey::Project("/r/blog".into());
    assert!(state.toggle(&blog));

    let rows = build(&windows, &state);
    let blog_row = rows
        .iter()
        .find(|row| row.key == blog)
        .expect("collapsed blog project row");
    let RowKind::Project { collapsed, .. } = &blog_row.kind else {
        panic!("blog key should identify a project row");
    };

    assert!(*collapsed);
    assert!(!row_keys(&rows).contains(&NodeKey::Window(8)));
    assert_eq!(agent_order(&rows), (1..=7).collect::<Vec<_>>());
}

#[test]
fn collapsed_window_hides_its_subagents() {
    let windows = example();
    let mut state = TreeState::default();
    assert!(state.toggle(&NodeKey::Window(1)));

    let rows = build(&windows, &state);
    let keys = row_keys(&rows);
    for id in ["a1", "a2", "a3"] {
        assert!(!keys.contains(&NodeKey::Subagent {
            window_id: 1,
            id: id.into(),
        }));
    }
    let window = rows
        .iter()
        .find(|row| row.key == NodeKey::Window(1))
        .expect("collapsed window row");
    let RowKind::Window {
        has_subagents,
        collapsed,
        ..
    } = &window.kind
    else {
        panic!("window key should identify a window row");
    };

    assert!(*has_subagents);
    assert!(*collapsed);
    assert_eq!(agent_order(&rows), (1..=8).collect::<Vec<_>>());
    assert_eq!(
        rows.iter()
            .filter_map(|row| match &row.kind {
                RowKind::Window { position, .. } => Some(*position),
                RowKind::Project { .. } | RowKind::Subagent { .. } => None,
            })
            .collect::<Vec<_>>(),
        (1..=8).collect::<Vec<_>>()
    );
}

#[test]
fn toggle_only_accepts_projects_and_windows() {
    let mut state = TreeState::default();
    let project = NodeKey::Project("/r/shop".into());
    let window = NodeKey::Window(1);
    let subagent = NodeKey::Subagent {
        window_id: 1,
        id: "a1".into(),
    };

    assert!(state.toggle(&project));
    assert!(state.is_collapsed(&project));
    assert!(state.toggle(&project));
    assert!(!state.is_collapsed(&project));
    assert!(state.toggle(&window));
    assert!(state.is_collapsed(&window));
    let before = state.collapsed.clone();
    assert!(!state.toggle(&subagent));
    assert_eq!(state.collapsed, before);
}

#[test]
fn filter_matches_subagents_and_keeps_their_ancestors() {
    let windows = example();
    let state = TreeState {
        filter: "STYLE".into(),
        ..TreeState::default()
    };
    let rows = build(&windows, &state);

    assert_eq!(
        row_keys(&rows),
        vec![
            NodeKey::Project("/r/shop".into()),
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
        ]
    );
    let RowKind::Window { position, .. } = &rows[1].kind else {
        panic!("second row should be frontend");
    };
    assert_eq!(*position, 1);
    assert_eq!(agent_order(&rows), vec![4]);
}

#[test]
fn filter_on_a_project_shows_all_its_windows() {
    let windows = example();
    let state = TreeState {
        filter: "blog".into(),
        ..TreeState::default()
    };

    assert_eq!(
        row_keys(&build(&windows, &state)),
        vec![NodeKey::Project("/r/blog".into()), NodeKey::Window(8),]
    );
}

#[test]
fn filter_ignores_collapse() {
    let windows = example();
    let mut state = TreeState {
        filter: "billing".into(),
        ..TreeState::default()
    };
    assert!(state.toggle(&NodeKey::Project("/r/shop".into())));

    let rows = build(&windows, &state);
    assert!(row_keys(&rows).contains(&NodeKey::Window(2)));
}

#[test]
fn move_selection_clamps_without_wrapping() {
    let windows = example();
    let rows = build(&windows, &TreeState::default());
    let mut state = TreeState::default();

    state.move_selection(&rows, 1);
    assert_eq!(state.selected, Some(rows[0].key.clone()));

    let last = rows.last().expect("nonempty rows").key.clone();
    state.select(&rows, last.clone());
    state.move_selection(&rows, 1);
    assert_eq!(state.selected, Some(last));

    let first = rows[0].key.clone();
    state.select(&rows, first.clone());
    state.move_selection(&rows, -1);
    assert_eq!(state.selected, Some(first));
}

#[test]
fn selection_is_repaired_by_index() {
    let windows = example();
    let rows = build(&windows, &TreeState::default());
    let mut state = TreeState::default();
    state.select(&rows, NodeKey::Window(3));
    assert_eq!(state.selected_index(&rows), Some(6));

    let without_search: Vec<_> = windows
        .iter()
        .filter(|window| window.id != 3)
        .cloned()
        .collect();
    let rows = build(&without_search, &state);
    state.repair_selection(&rows);
    assert_eq!(state.selected, Some(NodeKey::Window(4)));
    assert_eq!(state.selected_index(&rows), Some(6));

    let rows = build(&[], &state);
    state.repair_selection(&rows);
    assert_eq!(state.selected, None);
    assert_eq!(state.selected_index(&rows), None);
}

#[test]
fn selection_follows_its_key_when_projects_reorder() {
    let mut windows = example();
    let rows = build(&windows, &TreeState::default());
    let mut state = TreeState::default();
    state.select(&rows, NodeKey::Window(8));

    windows
        .iter_mut()
        .find(|window| window.id == 8)
        .expect("notes window")
        .status = Status::Attention;
    windows
        .iter_mut()
        .find(|window| window.id == 2)
        .expect("billing window")
        .status = Status::Idle;
    let rows = build(&windows, &state);
    state.repair_selection(&rows);

    assert_eq!(state.selected, Some(NodeKey::Window(8)));
    assert_eq!(state.selected_index(&rows), Some(1));
}

#[test]
fn viewport_reveal_scrolls_minimally() {
    let mut viewport = Viewport { top: 0, height: 5 };

    viewport.reveal(3);
    assert_eq!(viewport.top, 0);
    viewport.reveal(7);
    assert_eq!(viewport.top, 3);
    viewport.reveal(1);
    assert_eq!(viewport.top, 1);
}

#[test]
fn viewport_scroll_clamps() {
    let mut viewport = Viewport { top: 0, height: 5 };

    viewport.scroll(30, 16);
    assert_eq!(viewport.top, 11);
    viewport.scroll(-30, 16);
    assert_eq!(viewport.top, 0);
}

#[test]
fn prune_drops_vanished_keys() {
    let windows = example();
    let mut state = TreeState::default();
    let blog = NodeKey::Project("/r/blog".into());
    let notes = NodeKey::Window(8);
    assert!(state.toggle(&blog));
    assert!(state.toggle(&notes));

    let remaining: Vec<_> = windows
        .iter()
        .filter(|window| window.id != 8)
        .cloned()
        .collect();
    state.prune(&remaining);

    assert!(!state.collapsed.contains(&blog));
    assert!(!state.collapsed.contains(&notes));
}
