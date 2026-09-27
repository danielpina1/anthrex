//! M8c.6: opening, walking and leaving the run view (decisions 11, 21, 22 and 23). Enter
//! inside it, and the input guard, are in `run_enter.rs`.

use super::*;
use crate::app::RunView;
use crate::tree::RunFilter;
use crate::tree::run_fixtures::{
    RUN_ID, gate_fixture, snapshot, three_task_fixture, two_hundred_task_run,
};
use proto::{RunReply, RunState};

use super::super::runs::{app_with_runs, deliver, open_run_view};

pub(super) fn run_key() -> NodeKey {
    NodeKey::Run(RUN_ID.into())
}

pub(super) fn gate() -> App {
    let (snapshot, windows) = gate_fixture();
    app_with_runs(windows, snapshot)
}

pub(super) fn three() -> App {
    let (snapshot, windows) = three_task_fixture();
    app_with_runs(windows, snapshot)
}

/// Selects `key` among the canvas's rows (the run view's while it is open), as a
/// selection move would, and reveals it.
pub(super) fn select_nav(app: &mut App, key: NodeKey) {
    let rows = match app.run_view.as_ref().map(|view| view.filter) {
        Some(filter) => {
            let run = app
                .runs
                .runs
                .iter()
                .find(|run| run.run_id == RUN_ID)
                .expect("the run");
            tree::run_rows(run, &app.windows, &app.tree, filter)
        }
        None => tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree),
    };
    assert!(
        tree::row_index(&rows, &key).is_some(),
        "{key:?} is not a canvas row: {:?}",
        keys(&rows)
    );
    app.tree.select(&rows, key);
    app.reveal_tree_anchor();
}

pub(super) fn keys(rows: &[tree::Row<'_>]) -> Vec<NodeKey> {
    rows.iter().map(|row| row.key.clone()).collect()
}

fn tap(app: &mut App, code: KeyCode) -> Vec<Effect> {
    press(app, code, KeyModifiers::NONE)
}

/// The project overview with the run's node selected, not yet opened.
fn overview_on_the_run(app: &mut App) {
    assert!(toggle(app).is_empty());
    select(app, run_key());
    assert_eq!(app.tree.selected, Some(run_key()));
    assert_eq!(app.run_view, None);
}

fn assert_opened(app: &App, effects: &[Effect]) {
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        app.run_view,
        Some(RunView {
            run_id: RUN_ID.into(),
            filter: RunFilter::All,
        })
    );
    assert_eq!(app.tree.selected, Some(run_key()));
    assert_eq!(app.graph_pan, Pan::default());
    let rows = app.nav_rows();
    assert_eq!(rows[0].key, run_key());
    assert!(
        rows.iter().any(|row| row.key
            == NodeKey::Task {
                run: RUN_ID.into(),
                id: "t1".into()
            }),
        "the canvas shows the run's tasks"
    );
    assert!(app.overview);
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
}

#[test]
fn l_on_a_run_node_opens_the_run_view() {
    let mut app = gate();
    overview_on_the_run(&mut app);
    app.graph_pan = Pan { x: 5, y: 7 };
    app.tree.filter = "reset".into();
    let effects = tap(&mut app, KeyCode::Char('l'));
    assert_opened(&app, &effects);
    assert!(app.tree.filter.is_empty(), "opening resets the text filter");
}

#[test]
fn enter_on_a_run_node_opens_the_run_view() {
    let mut app = gate();
    overview_on_the_run(&mut app);
    app.graph_pan = Pan { x: 5, y: 7 };
    let effects = tap(&mut app, KeyCode::Enter);
    assert_opened(&app, &effects);

    // A double click on the node in the project overview opens it the same way.
    let mut app = gate();
    let layout = crate::ui::layout(Rect::new(0, 0, 120, 30), app.sidebar_width);
    assert!(toggle(&mut app).is_empty());
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    app.set_graph_viewport(layout.main);
    let (x, y) = box_middle(&app, layout.main, &run_key());
    assert!(app.on_click(x, y, &layout).is_empty());
    assert_eq!(app.run_view, None, "a single click only selects");
    let effects = app.on_click(x, y, &layout);
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(
        app.run_view.as_ref().map(|v| v.run_id.as_str()),
        Some(RUN_ID)
    );
    assert_eq!(app.tree.selected, Some(run_key()));
}

#[test]
fn enter_on_a_run_row_in_the_sidebar_tree_opens_the_overview_on_it() {
    let mut app = gate();
    prefix(&mut app);
    assert!(tap(&mut app, KeyCode::Char('t')).is_empty());
    assert!(!app.overview);
    let rows = tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, run_key());
    app.graph_pan = Pan { x: 5, y: 7 };
    let effects = tap(&mut app, KeyCode::Enter);
    assert_opened(&app, &effects);

    // A click on the run's sidebar row, outside tree mode, does the same, and turns
    // tree navigation on with it (the reversed box and the highlight need it).
    let mut app = gate();
    let layout = crate::ui::layout(Rect::new(0, 0, 120, 30), app.sidebar_width);
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    assert_eq!(app.tree_input, None);
    let index = tree::row_index(&app.rows(), &run_key()).expect("the run has a sidebar row");
    let effects = app.on_click(
        layout.sidebar_list.x + 2,
        layout.sidebar_list.y + index as u16,
        &layout,
    );
    assert_opened(&app, &effects);
    assert!(app.tree_input.is_some());
    assert!(app.keymap.tree_mode());
}

#[test]
fn space_on_a_run_node_in_the_project_tree_does_nothing() {
    let mut app = gate();
    overview_on_the_run(&mut app);
    assert!(tap(&mut app, KeyCode::Char(' ')).is_empty());
    assert!(app.tree.collapsed.is_empty(), "{:?}", app.tree.collapsed);
    assert_eq!(app.run_view, None);

    // Nor in the sidebar tree.
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    prefix(&mut app);
    assert!(tap(&mut app, KeyCode::Char('t')).is_empty());
    let rows = tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, run_key());
    assert!(tap(&mut app, KeyCode::Char(' ')).is_empty());
    assert!(app.tree.collapsed.is_empty(), "{:?}", app.tree.collapsed);
}

#[test]
fn h_at_the_root_and_esc_return_to_the_project_overview() {
    let mut app = three();
    open_run_view(&mut app, RUN_ID);
    assert!(tap(&mut app, KeyCode::Char('h')).is_empty());
    assert_eq!(app.run_view, None);
    assert_eq!(app.tree.selected, Some(run_key()));
    assert!(app.overview);
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));
    assert_eq!(
        keys(&app.nav_rows()),
        keys(&app.rows()),
        "the canvas is the project tree again"
    );

    // `Esc` from anywhere in the run view, with a filter on.
    open_run_view(&mut app, RUN_ID);
    assert!(tap(&mut app, KeyCode::Char('f')).is_empty());
    tap(&mut app, KeyCode::Char('j'));
    assert_ne!(app.tree.selected, Some(run_key()));
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_eq!(app.run_view, None);
    assert_eq!(app.tree.selected, Some(run_key()));
    assert!(app.overview);
    assert_eq!(app.tree_input, Some(TreeInput::Navigate));

    // A second `Esc` leaves tree mode, as in the project overview today.
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    assert_closed(&app);
    assert_eq!(app.run_view, None);
}

#[test]
fn h_below_the_root_selects_the_parent() {
    let mut app = three();
    open_run_view(&mut app, RUN_ID);
    let t1 = NodeKey::Task {
        run: RUN_ID.into(),
        id: "t1".into(),
    };
    let round = NodeKey::AgentRound {
        run: RUN_ID.into(),
        task: "t1".into(),
        role: proto::AgentRole::Worker,
        session: 1,
        round: 1,
    };
    select_nav(&mut app, round);
    assert!(tap(&mut app, KeyCode::Char('h')).is_empty());
    assert_eq!(app.tree.selected, Some(t1.clone()));
    assert!(app.run_view.is_some());
    assert!(tap(&mut app, KeyCode::Left).is_empty());
    assert_eq!(app.tree.selected, Some(run_key()));
    assert!(app.run_view.is_some(), "h from a task stops at the root");
    // `l` walks back down to the first child.
    assert!(tap(&mut app, KeyCode::Char('l')).is_empty());
    assert_eq!(app.tree.selected, Some(app.nav_rows()[1].key.clone()));
}

#[test]
fn j_k_walk_the_run_view_in_order() {
    let mut app = three();
    open_run_view(&mut app, RUN_ID);
    let expected: Vec<_> =
        tree::run_rows(&app.runs.runs[0], &app.windows, &app.tree, RunFilter::All)
            .into_iter()
            .map(|row| row.key)
            .collect();
    assert!(expected.len() > 4, "{expected:?}");
    let mut walked = vec![app.tree.selected.clone().expect("the root")];
    for _ in 1..expected.len() {
        tap(&mut app, KeyCode::Char('j'));
        walked.push(app.tree.selected.clone().expect("a selection"));
    }
    assert_eq!(walked, expected);
    for key in expected.iter().rev().skip(1) {
        tap(&mut app, KeyCode::Char('k'));
        assert_eq!(app.tree.selected.as_ref(), Some(key));
    }
}

#[test]
fn f_cycles_the_filters() {
    let mut app = three();
    open_run_view(&mut app, RUN_ID);
    // `t2` is queued: `running` drops it, so the selection must be repaired.
    select_nav(
        &mut app,
        NodeKey::Task {
            run: RUN_ID.into(),
            id: "t2".into(),
        },
    );
    let order = [
        RunFilter::Running,
        RunFilter::Blocked,
        RunFilter::Runtime(proto::Runtime::Claude),
        RunFilter::Runtime(proto::Runtime::Codex),
        RunFilter::All,
    ];
    for filter in order {
        assert!(tap(&mut app, KeyCode::Char('f')).is_empty());
        assert_eq!(app.run_view.as_ref().map(|view| view.filter), Some(filter));
        let rows = app.nav_rows();
        let expected: Vec<_> = tree::run_rows(&app.runs.runs[0], &app.windows, &app.tree, filter)
            .into_iter()
            .map(|row| row.key)
            .collect();
        assert_eq!(
            rows.iter().map(|row| row.key.clone()).collect::<Vec<_>>(),
            expected,
            "{filter:?}"
        );
        assert!(
            app.tree.selected_index(&rows).is_some(),
            "{filter:?}: {:?} is not a row",
            app.tree.selected
        );
    }
    // Leaving and opening again starts from `all`.
    assert!(tap(&mut app, KeyCode::Char('f')).is_empty());
    assert!(tap(&mut app, KeyCode::Esc).is_empty());
    open_run_view(&mut app, RUN_ID);
    assert_eq!(
        app.run_view.as_ref().map(|view| view.filter),
        Some(RunFilter::All)
    );
}

#[test]
fn the_run_view_closes_when_its_run_leaves() {
    let mut app = three();
    open_run_view(&mut app, RUN_ID);
    tap(&mut app, KeyCode::Char('j'));
    deliver(&mut app, snapshot(10_001, vec![]));
    assert_eq!(app.run_view, None);
    assert!(app.overview, "back in the project overview");
    assert_eq!(app.toast_text(), Some("run add-reset-3f9a is gone"));
    let rows = app.nav_rows();
    assert_eq!(keys(&rows), keys(&app.rows()));
    assert!(app.tree.selected_index(&rows).is_some());

    for (state, text) in [
        (RunState::Discarded, "run add-reset-3f9a is discarded"),
        (RunState::Accepted, "run add-reset-3f9a is accepted"),
        (RunState::Failed, "run add-reset-3f9a is failed"),
    ] {
        let (mut snap, _) = three_task_fixture();
        deliver(&mut app, snap.clone());
        open_run_view(&mut app, RUN_ID);
        snap.runs[0].state = state;
        let effects = app.on_daemon(proto::DaemonMsg::Run(RunReply::Snapshot(snap)));
        assert!(effects.is_empty());
        assert_eq!(app.run_view, None, "{state:?}");
        assert!(app.overview);
        assert_eq!(app.toast_text(), Some(text));
    }

    // A snapshot that still shows the run keeps the view and its selection.
    let (snap, _) = three_task_fixture();
    deliver(&mut app, snap.clone());
    open_run_view(&mut app, RUN_ID);
    tap(&mut app, KeyCode::Char('j'));
    let selected = app.tree.selected.clone();
    assert_ne!(selected, Some(run_key()));
    deliver(&mut app, snap);
    assert!(app.run_view.is_some());
    assert_eq!(app.tree.selected, selected);
}

#[test]
fn reveal_follows_the_selection_in_the_run_view() {
    let mut app = app_with_runs(vec![], snapshot(10_000, vec![two_hundred_task_run()]));
    let layout = crate::ui::layout(Rect::new(0, 0, 120, 40), app.sidebar_width);
    assert!(toggle(&mut app).is_empty());
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    app.set_graph_viewport(layout.main);
    open_run_view(&mut app, RUN_ID);
    for _ in 0..800 {
        tap(&mut app, KeyCode::Char('j'));
    }
    let rows = app.nav_rows();
    let last = rows.last().expect("rows").key.clone();
    assert_eq!(rows.len(), 801);
    assert_eq!(app.tree.selected, Some(last.clone()));
    let view = overview::view(&app, layout.main);
    let rect = view
        .layout
        .node(&last)
        .expect("the last row is placed")
        .rect;
    assert!(app.graph_pan.y > 0, "{:?}", app.graph_pan);
    assert!(
        rect.y >= view.pan.y && rect.bottom() <= view.pan.y + view.canvas.height,
        "{rect:?} under {:?} in {:?}",
        view.pan,
        view.canvas
    );
    assert!(
        rect.x >= view.pan.x && rect.right() <= view.pan.x + view.canvas.width,
        "{rect:?} under {:?} in {:?}",
        view.pan,
        view.canvas
    );
}

/// A window list while the run view is open repairs the selection against the run
/// view's rows, not the project tree's.
#[test]
fn a_window_list_keeps_the_run_view_selection() {
    let mut app = three();
    open_run_view(&mut app, RUN_ID);
    let t2 = NodeKey::Task {
        run: RUN_ID.into(),
        id: "t2".into(),
    };
    select_nav(&mut app, t2.clone());
    let windows = app.windows.clone();
    let _ = app.on_daemon(proto::DaemonMsg::WindowsChanged { windows });
    assert!(app.run_view.is_some());
    assert_eq!(app.tree.selected, Some(t2));
}
