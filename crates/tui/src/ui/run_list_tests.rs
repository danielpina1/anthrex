//! M9.0.7.10: the run view as a compact list below 30 rows of interior (decision 22):
//! its rows, its window, its mouse and its keys.

use super::{RUN_LIST_BELOW, first_row};
use crate::app::App;
use crate::safe_text::tests::{first_hostile, hostile_text};
use crate::settings::UiSettings;
use crate::theme::{Role, role};
use crate::tree::run_fixtures::{PROJECT, RUN_ID, pty, run, snapshot, task};
use crate::tree::stage_fixtures::two_stage_fixture;
use crate::tree::{self, NodeKey};
use crate::ui::{audit, overview};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{DaemonMsg, RunReply, RunState, RunsSnapshot, Size, TaskState, WindowInfo};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Modifier;

pub(super) const MUL: &str = "add-mul-0723";

pub(super) fn app_of((snap, windows): (RunsSnapshot, Vec<WindowInfo>), ascii: bool) -> App {
    let mut settings = UiSettings::default();
    settings.badges.ascii = ascii;
    let mut app = App::new(windows, "/tmp".into(), settings);
    let _ = app.set_terminal_size(80, 24);
    let _ = app.run_subscription();
    app.on_daemon(DaemonMsg::Run(RunReply::Snapshot(snap)));
    app
}

pub(super) fn key(app: &mut App, code: KeyCode) {
    app.on_key(KeyEvent::new(code, KeyModifiers::NONE));
}

/// `C-b T`, the run's node selected, then `l`: the run view on `run_id`.
pub(super) fn run_view(mut app: App, run_id: &str) -> App {
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    key(&mut app, KeyCode::Char('T'));
    let rows = tree::build_with_runs(&app.windows, &app.runs.runs, &app.tree);
    app.tree.select(&rows, NodeKey::Run(run_id.into()));
    key(&mut app, KeyCode::Char('l'));
    assert!(app.run_view.is_some(), "the run view opened");
    app
}

pub(super) fn task_key(run: &str, id: &str) -> NodeKey {
    NodeKey::Task {
        run: run.into(),
        id: id.into(),
    }
}

pub(super) fn select(app: &mut App, key: NodeKey) {
    let rows = crate::app::nav_rows_of(&app.windows, &app.runs, &app.tree, app.run_view.as_ref());
    assert!(rows.iter().any(|row| row.key == key), "{key:?} is a row");
    app.tree.select(&rows, key);
}

/// The audit's run view on the two-stage run with task `id` selected (decision 22's
/// fixture: `t2` in review round 1 after `t1`, `t3` waiting after `t2`).
pub(crate) fn two_stage_run_selected(id: &str) -> App {
    let mut app = run_view(app_of(two_stage_fixture(), false), MUL);
    select(&mut app, task_key(MUL, id));
    app
}

/// One frame as `lib::draw` lays it out: the viewports set from it, then drawn whole.
pub(super) fn frame(app: &mut App, w: u16, h: u16) -> (Buffer, crate::ui::Layout) {
    let layout = crate::ui::layout_for(app, Rect::new(0, 0, w, h));
    app.set_tree_viewports(layout.sidebar_list.height, layout.main_inner.height);
    app.set_graph_viewport(layout.main);
    (audit::draw(app, w, h), layout)
}

pub(super) fn rows_in(buffer: &Buffer, rect: Rect) -> Vec<String> {
    (rect.y..rect.bottom())
        .map(|y| {
            let row: String = (rect.x..rect.right())
                .map(|x| buffer[(x, y)].symbol())
                .collect();
            row.trim_end().to_owned()
        })
        .collect()
}

/// The canvas's rows at `w`×`h`, trimmed, with trailing empty rows dropped.
pub(super) fn canvas_rows(app: &App, w: u16, h: u16) -> Vec<String> {
    let layout = crate::ui::layout_for(app, Rect::new(0, 0, w, h));
    let view = overview::view(app, layout.main);
    assert!(view.list, "the run view draws the list at {w}x{h}");
    let mut rows = rows_in(&audit::draw(app, w, h), view.canvas);
    while rows.last().is_some_and(String::is_empty) {
        rows.pop();
    }
    rows
}

/// A one-stage run of forty waiting tasks `t0`…`t39`, S, tdd, titled `work <n>`.
pub(super) fn forty_tasks() -> (RunsSnapshot, Vec<WindowInfo>) {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    info.tasks = (0..40)
        .map(|n| {
            task(
                &format!("t{n}"),
                &format!("work {n}"),
                Size::S,
                TaskState::Pending,
            )
        })
        .collect();
    (
        snapshot(10_000, vec![info]),
        vec![pty(1, "shell", PROJECT, proto::Status::Idle)],
    )
}

pub(super) fn forty_selected(id: &str) -> App {
    let mut app = run_view(app_of(forty_tasks(), false), RUN_ID);
    select(&mut app, task_key(RUN_ID, id));
    app
}

/// At 80x24 the default 34-column sidebar leaves the canvas 44 columns: the titles
/// shrink, the state word is cut, the size and mode go, and what that frees goes back
/// to the state word first, then the titles (decision 22's order; fix round 1, I3).
/// The brief's own literal is a 58-column canvas (a 60-column terminal, no sidebar),
/// where every column shows; there `t2`'s title is cut to the 18 columns `t3`'s takes.
#[test]
fn below_30_rows_the_run_view_is_a_list() {
    let mut app = two_stage_run_selected("t2");
    let rows = canvas_rows(&app, 80, 24); // the canvas's rows, trimmed
    assert_eq!(
        rows,
        vec![
            "◉ run 0723  1/3",
            "  ✓ stage 1/2  tier 3 ✓ 38s",
            "    ✓ t1 add mul(…  merged",
            "  ⠋ stage 2/2  tier 3 running",
            "▌   ◐ t2 report_p…  in review · r1  after t1",
            "    ◌ t3 docs for…  waiting         after t2",
        ]
    );
    app.sidebar_visible = false;
    assert_eq!(
        canvas_rows(&app, 60, 24),
        vec![
            "◉ run 0723  1/3",
            "  ✓ stage 1/2  tier 3 ✓ 38s",
            "    ✓ t1 add mul() to a   S tdd   merged",
            "  ⠋ stage 2/2  tier 3 running",
            "▌   ◐ t2 report_product…  M tdd   in review · r1  after t1",
            "    ◌ t3 docs for repor…  S none  waiting         after t2",
        ]
    );
}

#[test]
fn the_selection_is_the_bar_in_the_accent_and_the_row_reversed() {
    let app = two_stage_run_selected("t2");
    let layout = crate::ui::layout_for(&app, Rect::new(0, 0, 80, 24));
    let canvas = overview::view(&app, layout.main).canvas;
    let buffer = audit::draw(&app, 80, 24);
    let y = canvas.y + 4;
    let bar = &buffer[(canvas.x, y)];
    assert_eq!(bar.symbol(), "▌");
    assert_eq!(Some(bar.fg), role(Role::Accent, app.palette()).fg);
    assert!(!bar.modifier.contains(Modifier::REVERSED));
    let glyph = &buffer[(canvas.x + 4, y)];
    assert_eq!(glyph.symbol(), "◐");
    assert!(glyph.modifier.contains(Modifier::REVERSED));
    // No other row wears the bar.
    for row in [0, 1, 2, 3, 5] {
        assert_ne!(buffer[(canvas.x, canvas.y + row)].symbol(), "▌");
    }
}

#[test]
fn a_selected_root_keeps_its_glyph() {
    let mut app = two_stage_run_selected("t2");
    select(&mut app, NodeKey::Run(MUL.into()));
    let rows = canvas_rows(&app, 80, 24);
    assert_eq!(rows[0], "▌◉ run 0723  1/3");
    assert_eq!(rows[1], "  ✓ stage 1/2  tier 3 ✓ 38s");
}

#[test]
fn at_37_rows_the_graph_stays() {
    let mut app = two_stage_run_selected("t2");
    let (buffer, layout) = frame(&mut app, 120, 40);
    let view = overview::view(&app, layout.main);
    assert!(!view.list, "an interior of 37 rows keeps the graph");
    let rows = rows_in(&buffer, view.canvas).join("\n");
    assert!(rows.contains('╭'), "boxes are drawn:\n{rows}");
    assert!(!rows.contains("in review · r1"), "no list columns:\n{rows}");
    // The boundary: an interior of 29 rows is a list, 30 is the graph.
    for (h, list) in [(32, true), (33, false)] {
        let layout = crate::ui::layout_for(&app, Rect::new(0, 0, 120, h));
        assert_eq!(layout.main.height - 2 < RUN_LIST_BELOW, list);
        assert_eq!(overview::view(&app, layout.main).list, list, "at 120x{h}");
    }
}

#[test]
fn the_list_keeps_the_selection_in_view() {
    let app = forty_selected("t30");
    let layout = crate::ui::layout_for(&app, Rect::new(0, 0, 80, 24));
    let canvas = overview::view(&app, layout.main).canvas;
    let rows = rows_in(&audit::draw(&app, 80, 24), canvas);
    let height = usize::from(canvas.height);
    assert!(height >= 3, "the canvas has room for both marks");
    // `t30` is row 31 of 41 (the root first); centred.
    let first = first_row(41, 31, height);
    assert_eq!(rows[0], format!("↑ {} more", first + 1));
    assert_eq!(
        rows[height - 1],
        format!("↓ {} more", 41 - (first + height - 1))
    );
    let selected = rows
        .iter()
        .position(|row| row.contains("t30 work 30"))
        .expect("the selection is on screen");
    assert!(rows[selected].starts_with('▌'), "{rows:#?}");
    assert_eq!(selected, 31 - first);
    assert!(selected > 0 && selected < height - 1);
}

#[test]
fn the_window_is_centred_and_clamped() {
    assert_eq!(first_row(5, 4, 6), 0, "everything fits");
    assert_eq!(first_row(41, 0, 6), 0);
    assert_eq!(first_row(41, 2, 6), 0);
    assert_eq!(first_row(41, 20, 6), 17);
    assert_eq!(first_row(41, 40, 6), 35);
    assert_eq!(first_row(41, 99, 6), 35, "a stale index is clamped");
    assert_eq!(first_row(41, 20, 1), 20, "one row: the selection alone");
    assert_eq!(first_row(41, 40, 2), 39);
    assert_eq!(first_row(41, 20, 0), 0);
}

/// Pinning: the list changes the drawing only; `j`, `k`, `h`, `l`, `space` and `.`
/// select what they select over the graph.
#[test]
fn keys_are_the_run_views() {
    let mut list = two_stage_run_selected("t1");
    let mut graph = two_stage_run_selected("t1");
    let (_, layout) = frame(&mut list, 80, 24);
    assert!(overview::view(&list, layout.main).list);
    let (_, layout) = frame(&mut graph, 120, 40);
    assert!(!overview::view(&graph, layout.main).list);
    let keys = [
        KeyCode::Char('j'),
        KeyCode::Char('j'),
        KeyCode::Char('k'),
        KeyCode::Char('h'),
        KeyCode::Char(' '),
        KeyCode::Char('l'),
        KeyCode::Char(' '),
        KeyCode::Char('l'),
        KeyCode::Char('j'),
        KeyCode::Char('.'),
        KeyCode::Esc,
        KeyCode::Char('k'),
    ];
    for code in keys {
        let a = list.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        let b = graph.on_key(KeyEvent::new(code, KeyModifiers::NONE));
        assert_eq!(a, b, "{code:?}'s effects");
        let _ = frame(&mut list, 80, 24);
        let _ = frame(&mut graph, 120, 40);
        assert_eq!(list.tree.selected, graph.tree.selected, "after {code:?}");
        assert_eq!(
            list.modal.is_some(),
            graph.modal.is_some(),
            "after {code:?}"
        );
    }
}

#[test]
fn the_list_in_ascii() {
    let mut app = run_view(app_of(two_stage_fixture(), true), MUL);
    select(&mut app, task_key(MUL, "t2"));
    let buffer = audit::draw(&app, 80, 24);
    assert_eq!(
        audit::first_non_ascii(&buffer),
        None,
        "{}",
        audit::rows(&buffer).join("\n")
    );
    let rows = canvas_rows(&app, 80, 24);
    assert_eq!(rows[0], "@ run 0723  1/3");
    assert_eq!(rows[1], "  + stage 1/2  tier 3 + 38s");
    assert_eq!(rows[4], ">   % t2 report...  in review - r1  after t1");
    app.sidebar_visible = false;
    let rows = canvas_rows(&app, 60, 24);
    assert_eq!(
        rows[4],
        ">   % t2 report_produ...  M tdd   in review - r1  after t1"
    );
    let mut app = forty_selected("t30");
    app.settings.badges.ascii = true;
    let rows = canvas_rows(&app, 80, 24);
    assert!(rows[0].starts_with("^ ") && rows[0].ends_with(" more"));
    assert!(rows.last().unwrap().starts_with("v "));
}

/// Global Constraint 4: a task's title, id and deps are agents' text. The hostile
/// characters sit inside the columns the cut keeps (fix round 1, I2), so dropping the
/// sanitiser shows on screen.
#[test]
fn list_text_is_sanitised() {
    let (mut snap, windows) = two_stage_fixture();
    let tasks = &mut snap.runs[0].tasks;
    tasks[1].title = "x\u{200D}y\u{202E}z\u{1b}w work".into();
    tasks[2].title = format!("x{}", hostile_text());
    tasks[2].deps = vec!["t\u{202E}2\u{1b}x".into()];
    for ascii in [false, true] {
        let mut app = run_view(app_of((snap.clone(), windows.clone()), ascii), MUL);
        select(&mut app, task_key(MUL, "t2"));
        for (w, h) in [(80, 24), (120, 24), (100, 20)] {
            let rows = canvas_rows(&app, w, h);
            for row in &rows {
                assert_eq!(first_hostile(row), None, "{row:?} at {w}x{h}");
            }
            assert!(rows[4].contains("t2 xyz w"), "{rows:#?}");
            assert!(rows[5].contains("after t2 x"), "{rows:#?}");
        }
    }
    let mut app = run_view(app_of((snap, windows), false), MUL);
    select(&mut app, task_key(MUL, "t2"));
    let rows = canvas_rows(&app, 80, 24);
    // The deps column is wider here, so the freed columns give the state word 13.
    assert_eq!(rows[4], "▌   ◐ t2 xyz w w…  in review · …  after t1");
}

/// Decision 22's cut order, on canvases `w` columns wide (the sidebar hidden): the
/// titles shrink to eight columns, then the state word to 12, then the size and mode
/// go (the titles take back what that frees), and only then are the deps cut.
#[test]
fn narrow_lists_cut_the_title_then_the_state_then_the_size() {
    let mut app = two_stage_run_selected("t1");
    app.sidebar_visible = false;
    let at = |w: u16| canvas_rows(&app, w + 2, 24)[4].clone();
    assert_eq!(
        at(66),
        "    ◐ t2 report_product in c      M tdd   in review · r1  after t1"
    );
    assert_eq!(
        at(58),
        "    ◐ t2 report_product…  M tdd   in review · r1  after t1"
    );
    assert_eq!(
        at(51),
        "    ◐ t2 report_…  M tdd   in review · r1  after t1"
    );
    assert_eq!(at(49), "    ◐ t2 report_…  M tdd   in review ·…  after t1");
    assert_eq!(at(48), "    ◐ t2 report_produ…  in review · r1  after t1");
    assert_eq!(at(41), "    ◐ t2 report_…  in review ·…  after t1");
    assert_eq!(at(39), "    ◐ t2 report_…  in review ·…  after…");
}

/// Fix round 1 (I3): narrowing the canvas never lengthens a title while the state word
/// is cut — the columns a cut frees go to the state word before the titles.
#[test]
fn narrowing_never_lengthens_a_title_while_the_state_is_cut() {
    let mut app = two_stage_run_selected("t1");
    app.sidebar_visible = false;
    let state = "in review · r1";
    let mut wider: Option<(u16, usize)> = None;
    for w in (30..=70u16).rev() {
        let row = canvas_rows(&app, w + 2, 24)[4].clone();
        let title = row
            .split("  ")
            .find(|part| part.contains("t2 "))
            .map_or(0, |part| part.trim().chars().count());
        let cut = !row.contains(state);
        if let (true, Some((at, longer))) = (cut, wider) {
            assert!(
                title <= longer,
                "at {w} columns the title is {title} wide, at {at} {longer}: {row:?}"
            );
        }
        wider = Some((w, title));
    }
}

#[test]
fn no_panic_at_tiny_sizes() {
    for (w, h) in [
        (20, 5),
        (1, 1),
        (5, 40),
        (80, 3),
        (80, 7),
        (30, 12),
        (12, 31),
    ] {
        for app in [two_stage_run_selected("t2"), forty_selected("t39")] {
            let mut app = app;
            let (_, layout) = frame(&mut app, w, h);
            let view = overview::view(&app, layout.main);
            let (x, y) = (view.canvas.x, view.canvas.y);
            let _ = app.on_click(x, y, &layout);
            let _ = app.on_scroll(false, x, y, &layout);
            let _ = audit::draw(&app, w, h);
        }
    }
}

/// Final fix wave M4: the list's deps column is the plan review's formatter, so a task
/// with only implicit deps reads `after t1 (implied)`, as the panel's DETAIL does.
#[test]
fn the_list_marks_implicit_deps() {
    let mut app = two_stage_run_selected("t2");
    let t3 = app.runs.runs[0]
        .tasks
        .iter_mut()
        .find(|t| t.id == "t3")
        .expect("t3");
    t3.deps.clear();
    t3.implicit_deps = vec!["t2".into()];
    app.sidebar_visible = false;
    let rows = canvas_rows(&app, 80, 24);
    assert!(
        rows.iter()
            .any(|r| r.contains("t3 docs") && r.ends_with("after t2 (implied)")),
        "{rows:#?}"
    );
}
