//! M8c.6 review fixes: every path that selects, repairs, reveals or draws while the run
//! view is open reads the run view's rows (decision 11), and a sidebar click that names
//! a project-tree row leaves the run view first.

use super::run_view::{keys, run_key, select_nav, three};
use super::*;
use crate::tree::run_fixtures::{RUN_ID, snapshot, three_task_fixture, two_hundred_task_run};
use proto::{AgentRole, RunState};

use super::super::runs::{app_with_runs, deliver, open_run_view};

fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

fn task_key(id: &str) -> NodeKey {
    NodeKey::Task {
        run: RUN_ID.into(),
        id: id.into(),
    }
}

fn worker_round(task: &str) -> NodeKey {
    NodeKey::AgentRound {
        run: RUN_ID.into(),
        task: task.into(),
        role: AgentRole::Worker,
        session: 1,
        round: 1,
    }
}

/// Sets both viewports for a `width` x `height` terminal, as `lib::draw` does.
fn laid_out(app: &mut App, width: u16, height: u16) -> crate::ui::Layout {
    let layout = crate::ui::layout(Rect::new(0, 0, width, height), app.sidebar_width);
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    app.set_graph_viewport(layout.main);
    layout
}

/// A press on `key`'s sidebar row, which must be on screen.
fn click_sidebar(app: &mut App, layout: &crate::ui::Layout, key: &NodeKey) -> Vec<Effect> {
    let index = tree::row_index(&app.rows(), key).expect("a sidebar row");
    let row = index - app.tree.sidebar.top;
    app.on_click(
        layout.sidebar_list.x + 2,
        layout.sidebar_list.y + row as u16,
        layout,
    )
}

/// The selection is a canvas row, and its box lies wholly inside the viewport.
fn assert_selection_shown(app: &App, main: Rect) {
    let rows = app.nav_rows();
    assert!(
        app.tree.selected_index(&rows).is_some(),
        "{:?} is not a canvas row: {:?}",
        app.tree.selected,
        keys(&rows)
    );
    let key = app.tree.selected.clone().expect("a selection");
    let view = overview::view(app, main);
    let rect = view
        .layout
        .node(&key)
        .expect("the selection is placed")
        .rect;
    assert!(
        rect.y >= view.pan.y
            && rect.bottom() <= view.pan.y + view.canvas.height
            && rect.x >= view.pan.x
            && rect.right() <= view.pan.x + view.canvas.width,
        "{key:?} at {rect:?} is outside {:?} under {:?}",
        view.canvas,
        view.pan
    );
}

fn two_hundred() -> (App, crate::ui::Layout) {
    let mut app = app_with_runs(vec![], snapshot(10_000, vec![two_hundred_task_run()]));
    assert!(toggle(&mut app).is_empty());
    let layout = laid_out(&mut app, 120, 40);
    (app, layout)
}

/// Review I1: a sidebar click on a project-tree row leaves the run view first, so the
/// selection is never a row the canvas does not draw; then M4's click rule applies.
#[test]
fn a_sidebar_click_on_a_window_leaves_the_run_view_first() {
    let mut app = three();
    let _ = app.focus(3);
    open_run_view(&mut app, RUN_ID);
    let layout = laid_out(&mut app, 120, 40);
    select_nav(&mut app, task_key("t1"));

    let _ = click_sidebar(&mut app, &layout, &NodeKey::Window(1));
    assert_eq!(app.run_view, None);
    assert!(app.overview, "back in the project overview");
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
    assert_eq!(app.tree.selected, Some(NodeKey::Window(1)));
    assert!(app.tree.selected_index(&app.rows()).is_some());
    assert_eq!(keys(&app.nav_rows()), keys(&app.rows()));
    assert_eq!(app.focused, Some(1), "M4: a click focuses a PTY window");

    // Enter on it is Enter on a PTY window: focus and leave tree mode.
    let effects = tap(&mut app, KeyCode::Enter);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::Send(ClientMsg::SubscribeConversation { .. }))),
        "{effects:?}"
    );
    assert_eq!(app.focused, Some(1));
    assert_eq!(app.tree_input, None);
    assert!(!app.conversation.is_open());

    // The run view's conversation rule covers sub-agents only: its rows hold no
    // `Window`, and a PTY window's key is focused even while the view is open.
    let _ = app.focus(3);
    open_run_view(&mut app, RUN_ID);
    let effects = app.activate_tree_node(NodeKey::Window(1));
    assert!(!app.conversation.is_open(), "{effects:?}");
    assert_eq!(app.focused, Some(1));
    assert_eq!(app.run_view, None);
}

#[test]
fn a_sidebar_click_on_the_project_leaves_the_run_view_and_folds_it() {
    let mut app = three();
    open_run_view(&mut app, RUN_ID);
    let layout = laid_out(&mut app, 120, 40);
    select_nav(&mut app, task_key("t1"));
    let project = NodeKey::Project(crate::tree::run_fixtures::PROJECT.into());

    assert!(click_sidebar(&mut app, &layout, &project).is_empty());
    assert_eq!(app.run_view, None);
    assert!(
        app.tree.is_collapsed(&project),
        "M4: a click folds a project"
    );
    assert_eq!(app.tree.selected, Some(project.clone()));
    assert_eq!(keys(&app.nav_rows()), vec![project.clone()]);

    assert!(click_sidebar(&mut app, &layout, &project).is_empty());
    assert!(!app.tree.is_collapsed(&project));
    assert_eq!(app.tree.selected, Some(project));
    assert!(keys(&app.nav_rows()).contains(&run_key()));
}

/// Deviation 2: a click on the open run's own sidebar row re-roots the view; it is not
/// Enter on the root, which would focus the orchestrator.
#[test]
fn a_sidebar_click_on_the_run_row_re_roots_the_view() {
    let mut app = three();
    let _ = app.focus(1);
    open_run_view(&mut app, RUN_ID);
    let layout = laid_out(&mut app, 120, 40);
    select_nav(&mut app, worker_round("t1"));

    assert!(click_sidebar(&mut app, &layout, &run_key()).is_empty());
    assert!(app.run_view.is_some());
    assert_eq!(app.tree.selected, Some(run_key()));
    assert_eq!(app.focused, Some(1));
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
}

/// Review I2: the frame draws the run view's rows — a task's box on the canvas and its
/// line in the footer — and not the project tree's.
#[test]
fn the_run_view_draws_its_own_rows() {
    let mut app = three();
    open_run_view(&mut app, RUN_ID);
    app.inspector_visible = false;
    let layout = laid_out(&mut app, 200, 50);
    select_nav(&mut app, task_key("t1"));

    let view = overview::view(&app, layout.main);
    let buffer = drawn(&app, 200, 50);
    let canvas = text_in(&buffer, view.canvas);
    let footer = text_in(&buffer, view.footer);
    assert!(
        canvas.contains("t1") && canvas.contains("spawn"),
        "{canvas}"
    );
    assert!(
        canvas.contains("t2") && canvas.contains("status"),
        "{canvas}"
    );
    assert!(
        footer.contains("t1") && footer.contains("spawn"),
        "footer {footer:?}"
    );
    // Window 1, `shell`, is a project-tree row only.
    assert!(!canvas.contains("shell"), "{canvas}");
}

#[test]
fn a_fold_in_the_run_view_repairs_against_its_rows() {
    let mut app = three();
    open_run_view(&mut app, RUN_ID);
    select_nav(&mut app, task_key("t1"));
    assert!(keys(&app.nav_rows()).contains(&worker_round("t1")));

    assert!(tap(&mut app, KeyCode::Char(' ')).is_empty());
    let rows = app.nav_rows();
    assert!(!keys(&rows).contains(&worker_round("t1")), "t1 is folded");
    assert_eq!(app.tree.selected, Some(task_key("t1")));
    assert!(app.tree.selected_index(&rows).is_some());
    assert!(app.run_view.is_some());

    assert!(tap(&mut app, KeyCode::Char(' ')).is_empty());
    assert!(keys(&app.nav_rows()).contains(&worker_round("t1")));
    assert_eq!(app.tree.selected, Some(task_key("t1")));
}

#[test]
fn a_text_filter_in_the_run_view_repairs_against_its_rows() {
    let mut app = three();
    open_run_view(&mut app, RUN_ID);
    select_nav(&mut app, worker_round("t1"));

    assert!(tap(&mut app, KeyCode::Char('/')).is_empty());
    for c in "t2".chars() {
        assert!(tap(&mut app, KeyCode::Char(c)).is_empty());
    }
    let rows = app.nav_rows();
    assert_eq!(keys(&rows), vec![run_key(), task_key("t2")]);
    assert!(
        app.tree.selected_index(&rows).is_some(),
        "{:?} is not a canvas row",
        app.tree.selected
    );
    assert!(app.run_view.is_some());

    // Review m3 (M9): closing the view clears the text filter too.
    assert!(tap(&mut app, KeyCode::Enter).is_empty());
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.run_view, None);
    assert!(app.tree.filter.is_empty(), "{:?}", app.tree.filter);
}

/// Review I2 (M10, M38): a snapshot or a window list that changes none of the run
/// view's rows is not a reveal edge, so the pan stays where the wheel left it.
#[test]
fn an_unchanged_snapshot_or_window_list_keeps_the_run_view_pan() {
    let (mut app, _) = two_hundred();
    open_run_view(&mut app, RUN_ID);
    let panned = crate::graph::Pan {
        x: app.graph_pan.x,
        y: app.graph_pan.y + 200,
    };
    app.graph_pan = panned;

    let run = app.runs.runs[0].clone();
    deliver(&mut app, snapshot(10_001, vec![run]));
    assert!(app.run_view.is_some());
    assert_eq!(app.tree.selected, Some(run_key()));
    assert_eq!(app.graph_pan, panned, "a snapshot");

    let windows = app.windows.clone();
    let _ = app.on_daemon(proto::DaemonMsg::WindowsChanged { windows });
    assert_eq!(app.graph_pan, panned, "a window list");
}

/// Review m1: the root cannot be folded inside the run view, and an old fold of it
/// never hides the view on opening.
#[test]
fn the_run_view_root_does_not_fold() {
    let mut app = three();
    open_run_view(&mut app, RUN_ID);
    let len = app.nav_rows().len();
    assert!(tap(&mut app, KeyCode::Char(' ')).is_empty());
    assert!(
        !app.tree.is_collapsed(&run_key()),
        "{:?}",
        app.tree.collapsed
    );
    assert_eq!(app.nav_rows().len(), len);

    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    app.tree.collapsed.insert(run_key());
    open_run_view(&mut app, RUN_ID);
    assert!(!app.tree.is_collapsed(&run_key()));
    assert_eq!(app.nav_rows().len(), len);
}

/// Review m2: a snapshot that closes the view selects the run row's neighbour, the
/// project row now where the run's node was, not a row at the run view's index.
#[test]
fn close_on_leave_selects_the_run_rows_neighbour() {
    for state in [None, Some(RunState::Discarded)] {
        let mut app = three();
        open_run_view(&mut app, RUN_ID);
        let at = tree::row_index(&app.rows(), &run_key()).expect("the run's node");
        select_nav(&mut app, worker_round("t1"));
        let (mut snap, _) = three_task_fixture();
        match state {
            Some(state) => snap.runs[0].state = state,
            None => snap.runs.clear(),
        }
        deliver(&mut app, snap);
        assert_eq!(app.run_view, None, "{state:?}");
        let rows = app.rows();
        assert_eq!(
            keys(&rows),
            vec![
                NodeKey::Project(crate::tree::run_fixtures::PROJECT.into()),
                NodeKey::Window(1),
                NodeKey::Window(3),
                NodeKey::Window(6),
            ],
            "{state:?}"
        );
        assert_eq!(at, 1);
        assert_eq!(app.tree.selected, Some(NodeKey::Window(1)), "{state:?}");
    }
}

/// Review m3 (M28): a snapshot that closes the view mid-typing ends the typing too.
#[test]
fn close_on_leave_ends_filter_typing() {
    let mut app = three();
    open_run_view(&mut app, RUN_ID);
    assert!(tap(&mut app, KeyCode::Char('/')).is_empty());
    assert!(tap(&mut app, KeyCode::Char('t')).is_empty());
    assert_eq!(app.tree_input, Some(TreeInput::Filter));
    deliver(&mut app, snapshot(10_001, vec![]));
    assert_eq!(app.run_view, None);
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
    assert!(app.tree.filter.is_empty());
}

/// Review m3 (M14, M20, M21): opening, leaving and `f` each reveal the selection. The
/// project has forty plain windows as well, so its overview is taller than the canvas
/// and a pan left over from the run view could hide the run's node.
#[test]
fn opening_leaving_and_f_reveal_the_selection() {
    let windows = (10..50)
        .map(|id| {
            crate::tree::run_fixtures::pty(
                id,
                &format!("w{id}"),
                crate::tree::run_fixtures::PROJECT,
                proto::Status::Idle,
            )
        })
        .collect();
    let mut app = app_with_runs(windows, snapshot(10_000, vec![two_hundred_task_run()]));
    assert!(toggle(&mut app).is_empty());
    let layout = laid_out(&mut app, 120, 40);
    open_run_view(&mut app, RUN_ID);
    assert_selection_shown(&app, layout.main);

    // `f` changes the rows, so it reveals the selection even after the wheel moved
    // the pan away from it.
    select_nav(&mut app, task_key("t100"));
    assert_selection_shown(&app, layout.main);
    app.graph_pan.y += 300;
    assert!(tap(&mut app, KeyCode::Char('f')).is_empty());
    assert_eq!(app.tree.selected, Some(task_key("t100")));
    assert_selection_shown(&app, layout.main);

    // Leaving from the bottom of the run view lands on the run's node, on screen.
    for _ in 0..4 {
        assert!(tap(&mut app, KeyCode::Char('f')).is_empty());
    }
    assert_eq!(
        app.run_view.as_ref().map(|view| view.filter),
        Some(crate::tree::RunFilter::All)
    );
    for _ in 0..800 {
        tap(&mut app, KeyCode::Char('j'));
    }
    assert!(app.graph_pan.y > 0);
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.run_view, None);
    assert_eq!(app.tree.selected, Some(run_key()));
    assert_selection_shown(&app, layout.main);
}
