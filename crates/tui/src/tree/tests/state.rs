use super::*;

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
