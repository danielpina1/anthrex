//! The full-screen graph overview's own render and hit-test coverage. Split out of
//! `ui/tests.rs` (task M6.10's file-size finding B, `AGENTS.md` hard rule 8) — the same
//! shape `app_tests/overview.rs` already gives the equivalent `app`-level tests.

use super::*;

fn open_overview(app: &mut App) {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    app.on_key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.on_key(KeyEvent::new(KeyCode::Char('T'), KeyModifiers::NONE));
}

#[test]
fn overview_replaces_the_terminal_with_the_graph() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut app = example_app();
    app.on_daemon(proto::DaemonMsg::Snapshot {
        window_id: 1,
        cols: 80,
        rows: 24,
        bytes: b"TERMINAL-TEXT".to_vec(),
    });
    assert!(render(&app, 200, 50).0.contains("TERMINAL-TEXT"));
    open_overview(&mut app);
    app.set_graph_viewport(layout(Rect::new(0, 0, 200, 50), app.sidebar_width, 0).main);
    let (out, _) = render(&app, 200, 50);
    for expected in [
        " tree overview ",
        // Boxes: a project, a window and a sub-agent, each with its
        // glyph, between the borders only the graph draws.
        "│ ⚑ shop   ├",
        "┤ ⠋ 1 api-worker ├",
        "┤ ⠋ Explore: map routes      ├",
        // The inspector panel spells the selected window out below it.
        "│ model    claude-opus-5  sub-agents  3, 2 running",
    ] {
        assert!(out.contains(expected), "missing {expected:?}:\n{out}");
    }
    assert!(!out.contains("TERMINAL-TEXT"), "{out}");
    app.on_key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let (out, _) = render(&app, 200, 50);
    assert!(out.contains("TERMINAL-TEXT"), "{out}");
    assert!(!out.contains(" tree overview "), "{out}");
}

#[test]
fn overview_geometry_matches_its_hit_test() {
    // Once with the whole canvas on screen, and once in a viewport too
    // small for it, panned on both axes. A hit test that forgot the pan
    // passes the first and fails the second.
    //
    // Both sizes draw the inspector panel, so this is also the guard that a
    // click still lands on the box under it once the panel has taken the
    // canvas's bottom rows. The second terminal is the panel's height taller
    // than milestone 4.6 wrote it, which leaves the canvas under test the same
    // sixteen rows it had then — at twenty rows the canvas is nine, and none of
    // the fixture's boxes are inside it at this pan for the sweep to check.
    let mut app = example_app();
    open_overview(&mut app);
    assert_eq!(hits_every_visible_box(&app, 200, 50), Pan::default());
    app.graph_pan = Pan { x: 20, y: 6 };
    assert_eq!(hits_every_visible_box(&app, 120, 27), Pan { x: 20, y: 6 });
}

/// Draws the overview at `width` x `height` and asserts that the middle of
/// every box on screen hit-tests to that box and that its left border was
/// really drawn there; returns the pan it drew at.
fn hits_every_visible_box(app: &App, width: u16, height: u16) -> Pan {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut l = None;
    terminal.draw(|f| l = Some(draw(f, app))).unwrap();
    let l = l.unwrap();
    let view = overview::view(app, l.main);
    let geometry = view.geometry();
    let mut checked = 0;
    for node in &view.layout.nodes {
        let Some(x) = (node.rect.x + node.rect.width / 2).checked_sub(view.pan.x) else {
            continue;
        };
        let Some(y) = (node.rect.y + 1).checked_sub(view.pan.y) else {
            continue;
        };
        let (x, y) = (view.canvas.x + x, view.canvas.y + y);
        // Only boxes whose left border is on screen too: a box clipped by
        // the left edge has no border cell to check.
        if !view.canvas.contains((x, y).into()) || node.rect.x < view.pan.x {
            continue;
        }
        assert_eq!(
            geometry.node_at(&view.layout, x, y).as_ref(),
            Some(&node.key),
            "the cell at ({x}, {y}) should hit {:?}",
            node.key
        );
        // The box's left border stands exactly where the layout put it,
        // as a plain border or as the junction an edge turned it into.
        let border = view.canvas.x + node.rect.x - view.pan.x;
        let symbol = terminal.backend().buffer()[(border, y)].symbol().to_owned();
        assert!(
            symbol == "│" || symbol == "┤",
            "{:?}'s left border at ({border}, {y}) was {symbol:?}",
            node.key
        );
        checked += 1;
    }
    assert!(checked > 1, "only {checked} boxes were on screen");
    // The block's own border, and the footer, are not the canvas.
    for (x, y) in [
        (l.main.x, view.canvas.y),
        (l.main.right() - 1, view.canvas.y),
        (view.canvas.x, l.main.y),
        (view.canvas.x, view.footer.y),
    ] {
        assert_eq!(geometry.node_at(&view.layout, x, y), None, "({x}, {y})");
    }
    view.pan
}

#[test]
fn overview_handles_zero_and_one_cell_areas() {
    let mut app = example_app();
    open_overview(&mut app);
    for sidebar_visible in [false, true] {
        app.sidebar_visible = sidebar_visible;
        for (width, height) in [(0, 0), (0, 1), (1, 0), (1, 1), (0, 30), (160, 0)] {
            let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
            terminal
                .draw(|frame| {
                    draw(frame, &app);
                })
                .unwrap();
        }
    }
}

/// The run view's single line (M8c.5, replaced in M8c.8): the canvas's own live glyph,
/// in its colour, then the inspector's name and, after two spaces, its right-hand text,
/// for a task and for one of its rounds. M8c.5's version showed the canvas text and
/// no right-hand text; M8c.8's `the_single_line_for_a_task` pins the Gemini task.
#[test]
fn the_run_views_single_line_is_the_live_glyph_the_name_and_the_right_text() {
    use crate::tree::run_fixtures::{RUN_ID, three_task_fixture};
    use crate::tree::{NodeKey, RunFilter, run_rows};
    let (snapshot, windows) = three_task_fixture();
    let mut app = App::new(windows, "/tmp".into(), UiSettings::default());
    app.on_daemon(proto::DaemonMsg::Run(proto::RunReply::Snapshot(snapshot)));
    let rows = run_rows(&app.runs.runs[0], &app.windows, &app.tree, RunFilter::All);
    let line = |key: NodeKey| {
        let row = rows
            .iter()
            .find(|row| row.key == key)
            .unwrap_or_else(|| panic!("{key:?} is a row"));
        let (glyph, label, right) = super::overview::footer_parts(row, &app);
        let canvas = crate::graph::paint::style::node_glyph(row, &app);
        assert_eq!(
            (glyph.content.as_ref(), glyph.style.fg),
            (canvas.0, Some(crate::theme::fg(canvas.1))),
            "the canvas's own glyph and colour"
        );
        (glyph.content.into_owned(), glyph.style.fg, label, right)
    };
    let live = Some(crate::theme::fg(crate::theme::Role::Working));
    let task = NodeKey::Task {
        run: RUN_ID.into(),
        id: "t1".into(),
    };
    assert_eq!(
        line(task),
        ("●".into(), live, "t1  spawn".into(), "  working".into())
    );
    let round = NodeKey::AgentRound {
        run: RUN_ID.into(),
        task: "t1".into(),
        role: proto::AgentRole::Worker,
        session: 1,
        round: 1,
    };
    let (glyph, color, label, right) = line(round);
    assert_eq!((glyph.as_str(), color), ("●", live));
    assert!(label.starts_with("worker #1  claude"), "{label}");
    assert!(right.starts_with("  idle · "), "{right}");
    assert!(right.ends_with(" · t1"), "{right}");
}
