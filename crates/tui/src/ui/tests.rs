//! The screen as a whole: the layout, what a frame draws into each pane, and
//! that the hit tests read the same geometry the frame was drawn with.
//!
//! Split out of `ui/mod.rs` when milestone 4.7 pushed that file past 600 lines.

use super::*;
use crate::app::{App, Modal, PendingAction};
use crate::graph::Pan;
use crate::keymap::Keymap;
use proto::{Runtime, Status, WindowInfo};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::style::Modifier;

fn win(id: u32, name: &str, runtime: Runtime, status: Status) -> WindowInfo {
    WindowInfo {
        id,
        name: name.into(),
        runtime,
        cwd: "/tmp/repo".into(),
        project: "/tmp/repo".into(),
        worktree: None,
        branch: Some("feat/x".into()),
        status,
        tool: None,
        since_secs: 75,
        last_output_secs: 1,
        session_id: None,
        model: None,
        subagents: vec![],
        exit: None,
    }
}

fn render(app: &App, width: u16, height: u16) -> (String, Layout) {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut layout = None;
    terminal.draw(|f| layout = Some(draw(f, app))).unwrap();
    (terminal.backend().to_string(), layout.unwrap())
}

fn example_app() -> App {
    let mut app = App::new(
        crate::tree::example_windows(),
        "/tmp".into(),
        Keymap::default_prefix(),
    );
    app.set_terminal_size(80, 24);
    app
}

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
    app.set_graph_viewport(layout(Rect::new(0, 0, 200, 50), app.sidebar_width).main);
    let (out, _) = render(&app, 200, 50);
    for expected in [
        " tree overview ",
        // Boxes: a project, a window and a sub-agent, each with its
        // glyph, between the borders only the graph draws.
        "│ ◆ shop   ├",
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

#[test]
fn sidebar_renders_the_example_tree_at_the_default_width() {
    let app = example_app();
    let (out, _) = render(&app, 120, 30);
    assert!(out.contains(" agents "), "{out}");
    for golden in [
        "▾ shop            ◆  cl 4 · cx 3",
        "├─▎ ⠋ 1 api-worker   cl opus  2m",
        "│ ├─⠋ Explore: map routes   Read",
        "▾ blog                   ○  cl 1",
    ] {
        assert!(out.contains(golden), "{golden:?}\n{out}");
    }
    assert!(out.contains("│ └─✓ tests: run unit suite"), "{out}");
    assert!(out.contains(" ◆ 2 billing"), "{out}");
    assert!(out.contains("8 agents · 2 working"), "{out}");
}

#[test]
fn a_sub_agent_asking_for_permission_shows_a_diamond() {
    let mut app = example_app();
    app.windows[0].subagents[0].needs_permission = true;
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|f| {
            draw(f, &app);
        })
        .unwrap();
    let out = terminal.backend().to_string();
    assert!(out.contains("│ ├─◆ Explore: map routes"), "{out}");
    assert!(out.contains("│ └─✓ tests: run unit suite"), "{out}");
    assert_eq!(
        terminal.backend().buffer()[(5, 3)].fg,
        crate::theme::status_color(Status::Attention)
    );
    app.windows[0].subagents[0].needs_permission = false;
    let (out, _) = render(&app, 120, 30);
    assert!(out.contains("│ ├─⠋ Explore: map routes"), "{out}");
}

#[test]
fn long_names_are_truncated_with_an_ellipsis() {
    let mut window = win(
        1,
        "a-very-long-window-name-that-overflows",
        Runtime::Shell,
        Status::Idle,
    );
    window.since_secs = 0;
    // This is about name truncation, not the sidebar branch marker `win()` happens to
    // set (decision 37, `ui/tree_view_tests.rs` covers the budget between the two) — so
    // it is turned off here to keep the two concerns from being tested at once.
    window.branch = None;
    let mut app = App::new(vec![window], "/tmp".into(), Keymap::default_prefix());
    app.set_terminal_size(80, 24);
    let (out, _) = render(&app, 120, 30);
    assert!(out.contains("… sh  0s│"), "{out}");
}

#[test]
fn empty_state_and_hidden_sidebar() {
    let mut app = App::new(vec![], "/tmp".into(), Keymap::default_prefix());
    let _ = app.set_terminal_size(80, 24);
    let (out, _) = render(&app, 100, 20);
    assert!(out.contains("no agents yet"));
    assert!(out.contains("No agents. Press C-b c to create one"));
    app.sidebar_visible = false;
    let (out, l) = render(&app, 100, 20);
    assert!(!out.contains("no agents yet"));
    assert_eq!(l.sidebar.width, 0);
}

#[test]
fn main_title_of_a_worktree_window_names_project_and_branch() {
    let mut window = win(1, "wt-api", Runtime::Shell, Status::Idle);
    window.project = "/tmp/shop".into();
    window.cwd = "/tmp/data/worktrees/shop-abcd/feat-x".into();
    window.branch = Some("feat/x".into());
    let mut app = App::new(vec![window], "/tmp".into(), Keymap::default_prefix());
    let _ = app.set_terminal_size(80, 24);
    let (out, _) = render(&app, 100, 20);
    assert!(out.contains("(feat/x, worktree)"), "{out}");
    assert!(out.contains("/tmp/shop"), "{out}");
    assert!(!out.contains("worktrees/shop-abcd"), "{out}");

    let mut plain = win(2, "plain", Runtime::Shell, Status::Idle);
    plain.branch = None;
    let mut app = App::new(vec![plain], "/tmp".into(), Keymap::default_prefix());
    let _ = app.set_terminal_size(80, 24);
    let (out, _) = render(&app, 100, 20);
    assert!(out.contains(" plain · shell · "), "{out}");
    assert!(!out.contains("worktree"), "{out}");
}

#[test]
fn help_lists_new_agent() {
    let mut app = App::new(
        vec![win(1, "a", Runtime::Shell, Status::Idle)],
        "/tmp".into(),
        Keymap::default_prefix(),
    );
    let _ = app.set_terminal_size(80, 24);
    app.modal = Some(Modal::Help);
    let (out, _) = render(&app, 100, 30);
    assert!(out.contains("C-b c"), "{out}");
    assert!(out.contains("new agent"), "{out}");
}

#[test]
fn statusbar_shows_prefix_state_and_toast() {
    let mut app = App::new(
        vec![win(1, "a", Runtime::Shell, Status::Idle)],
        "/tmp".into(),
        Keymap::default_prefix(),
    );
    let _ = app.set_terminal_size(80, 24);
    let (out, _) = render(&app, 100, 20);
    assert!(out.contains("C-b ?"));
    assert!(!out.contains("PREFIX"));
    app.on_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('b'),
        crossterm::event::KeyModifiers::CONTROL,
    ));
    app.toast("tests needs attention");
    let (out, _) = render(&app, 100, 20);
    assert!(out.contains("PREFIX"));
    assert!(out.contains("tests needs attention"));
}

#[test]
fn tree_mode_shows_the_badge_the_title_and_the_selection() {
    let mut app = example_app();
    app.enter_tree();
    let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
    terminal
        .draw(|f| {
            draw(f, &app);
        })
        .unwrap();
    let out = terminal.backend().to_string();
    assert!(out.contains(" TREE "), "{out}");
    assert!(out.contains("agents · tree"), "{out}");

    let selected = app.tree.selected.as_ref().unwrap();
    let index = app
        .rows()
        .iter()
        .position(|row| &row.key == selected)
        .unwrap();
    let l = layout(Rect::new(0, 0, 120, 30), app.sidebar_width);
    let y = l.sidebar_list.y + index as u16;
    for x in l.sidebar_list.x..l.sidebar_list.right() {
        assert!(
            terminal.backend().buffer()[(x, y)]
                .modifier
                .contains(Modifier::REVERSED),
            "cell ({x}, {y}) was not selected"
        );
    }

    app.tree_input = Some(crate::app::TreeInput::Filter);
    app.tree.filter = "sty".into();
    let (out, _) = render(&app, 120, 30);
    assert!(out.contains(" FILTER "), "{out}");
    assert!(out.contains("/sty"), "{out}");
}

#[test]
fn modals_render_on_top() {
    let mut app = App::new(
        vec![win(1, "a", Runtime::Shell, Status::Idle)],
        "/tmp".into(),
        Keymap::default_prefix(),
    );
    let _ = app.set_terminal_size(80, 24);
    app.modal = Some(Modal::Confirm {
        message: "Kill 'a'?".into(),
        action: PendingAction::Kill(1),
    });
    let (out, _) = render(&app, 100, 20);
    assert!(out.contains("Kill 'a'?"));
    assert!(out.contains("y / Enter = yes"));
    app.modal = Some(Modal::Help);
    let (out, _) = render(&app, 100, 24);
    assert!(out.contains("send a literal C-b"));
    assert!(out.contains("tree mode"));
    assert!(out.contains("sidebar width"));
}

#[test]
fn layout_uses_the_sidebar_width() {
    let l = layout(Rect::new(0, 0, 120, 40), 34);
    assert_eq!(l.sidebar.width, 34);
    assert_eq!(l.main.x, 34);
    assert_eq!(l.sidebar_list, Rect::new(1, 1, 32, 35));
    assert_eq!(l.sidebar_footer, Rect::new(1, 37, 32, 1));
    assert_eq!(l.statusbar, Rect::new(0, 39, 120, 1));
    let hidden = layout(Rect::new(0, 0, 120, 40), 0);
    assert_eq!(hidden.sidebar.width, 0);
    assert_eq!(hidden.sidebar_list.width, 0);
    assert_eq!(hidden.main.width, 120);
    let narrow = layout(Rect::new(0, 0, 80, 24), 24);
    assert_eq!(narrow.sidebar.width, 24);
    assert_eq!(narrow.main_inner.width, 54);
}

fn sidebar_text(buffer: &ratatui::buffer::Buffer, rect: Rect, y: u16) -> String {
    (rect.x..rect.right())
        .map(|x| buffer[(x, y)].symbol())
        .collect()
}

#[test]
fn hit_test_uses_the_render_geometry() {
    let mut app = example_app();
    app.set_tree_viewports(10, 12);
    app.tree.sidebar.top = 4;
    let mut terminal = Terminal::new(TestBackend::new(120, 15)).unwrap();
    let mut l = None;
    terminal.draw(|f| l = Some(draw(f, &app))).unwrap();
    let l = l.unwrap();
    assert_eq!(l.sidebar_list.height, 10);
    let g = tree_view::geometry(l.sidebar_list, app.rows().len(), app.tree.sidebar.top);
    for (offset, expected) in [
        "tests: run unit suite",
        "2 billing",
        "3 search",
        "4 frontend",
        "general-purpose:",
        "Explore: find tokens",
        "Explore: list files",
        "5 docs",
        "6 infra",
        "7 perf",
    ]
    .iter()
    .enumerate()
    {
        let y = l.sidebar_list.y + offset as u16;
        assert_eq!(g.index_at(2, y), Some(4 + offset));
        let text = sidebar_text(terminal.backend().buffer(), l.sidebar_list, y);
        assert!(
            text.contains(expected),
            "{text:?} should contain {expected:?}"
        );
    }
    for (x, y) in [(0, 1), (33, 1), (1, 0), (1, 11), (1, 12), (1, 13)] {
        assert_eq!(g.index_at(x, y), None, "outside list ({x},{y})");
    }
    let short = tree_view::geometry(Rect::new(2, 3, 10, 8), 2, 99);
    assert_eq!((short.first, short.count), (0, 2));
    assert_eq!(short.index_at(2, 5), None);
    let empty = tree_view::geometry(Rect::new(2, 3, 10, 0), 10, 100);
    assert_eq!((empty.first, empty.count), (10, 0));
    assert_eq!(empty.index_at(2, 3), None);
    let out = terminal.backend().to_string();
    assert!(
        !sidebar_text(
            terminal.backend().buffer(),
            l.sidebar_footer,
            l.sidebar_footer.y
        )
        .contains("attention"),
        "{out}"
    );
}

fn shells_app() -> App {
    let mut app = App::new(
        (1..=20)
            .map(|id| win(id, &format!("shell-{id}"), Runtime::Shell, Status::Idle))
            .collect(),
        "/tmp".into(),
        Keymap::default_prefix(),
    );
    app.set_terminal_size(80, 24);
    app
}

#[test]
fn sidebar_scrolls_to_keep_the_focused_window_visible() {
    let mut app = shells_app();
    app.focus(20);
    let l = layout(Rect::new(0, 0, 120, 14), 34);
    app.set_tree_viewports(l.sidebar_list.height, l.main_inner.height);
    let (out, _) = render(&app, 120, 14);
    assert!(out.contains("20 shell-20"), "{out}");
    assert!(!out.contains(" 1 shell-1 "), "{out}");
    assert_eq!(app.tree.sidebar.top, 12);
    // Same-size draws must preserve wheel scrolling, not snap back to the anchor.
    app.on_scroll(true, 2, 2, &l);
    app.set_tree_viewports(l.sidebar_list.height, l.main_inner.height);
    assert_eq!(app.tree.sidebar.top, 9);
    // Nor may a window list that changes nothing the view depends on: the
    // daemon republishes one on every status flip and every output event.
    let windows = app.windows.clone();
    app.on_daemon(proto::DaemonMsg::WindowsChanged { windows });
    assert_eq!(app.tree.sidebar.top, 9);
    // A list that really changed reveals the anchor again: with window 1
    // gone the focused window 20 is row 19 of 20, and nine rows of list
    // put its top at 11.
    let windows = app.windows[1..].to_vec();
    app.on_daemon(proto::DaemonMsg::WindowsChanged { windows });
    assert_eq!(app.tree.sidebar.top, 11);
}

#[test]
fn wheel_over_the_sidebar_scrolls_the_tree() {
    let mut app = shells_app();
    let l = layout(Rect::new(0, 0, 120, 14), 34);
    app.set_tree_viewports(l.sidebar_list.height, l.main_inner.height);
    assert!(app.on_scroll(false, 2, 2, &l).is_empty());
    assert_eq!(app.tree.sidebar.top, 3);
    assert_eq!(app.focused, Some(1));
    app.parser.process(b"\x1b[?1000h\x1b[?1006h");
    assert_eq!(
        app.on_scroll(false, l.main_inner.x, l.main_inner.y, &l),
        vec![crate::app::Effect::Send(proto::ClientMsg::Input {
            window_id: 1,
            bytes: b"\x1b[<65;1;1M".to_vec()
        })]
    );
    assert!(app.on_scroll(false, 2, l.sidebar_footer.y, &l).is_empty());
    assert_eq!(app.tree.sidebar.top, 3);
}

#[test]
fn a_click_on_a_row_focuses_toggles_or_focuses_the_parent() {
    use crate::app::Effect;
    use proto::ClientMsg;
    let mut app = example_app();
    let l = layout(Rect::new(0, 0, 120, 30), 34);
    app.set_tree_viewports(l.sidebar_list.height, l.main_inner.height);
    assert!(app.on_click(2, 2, &l).is_empty());
    assert_eq!(
        app.on_click(2, 6, &l),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 2,
            cols: 80,
            rows: 24
        })]
    );
    assert_eq!(
        app.on_click(2, 3, &l),
        vec![Effect::Send(ClientMsg::Subscribe {
            window_id: 1,
            cols: 80,
            rows: 24
        })]
    );
    assert!(app.on_click(2, 15, &l).is_empty());
    assert!(
        app.tree
            .is_collapsed(&crate::tree::NodeKey::Project("/r/blog".into()))
    );
    app.focus(8);
    assert_eq!(app.tree.sidebar.top, 0);
    assert!(app.on_click(0, 6, &l).is_empty());
    assert!(app.on_click(2, l.sidebar_footer.y, &l).is_empty());
    app.modal = Some(Modal::Help);
    assert!(app.on_click(2, 6, &l).is_empty());
    assert_eq!(app.focused, Some(8));
}

#[test]
fn unicode_names_preserve_graphemes_and_right_fields() {
    for name in ["界".repeat(20), "👩🏽‍💻".repeat(20)] {
        let mut window = win(1, &name, Runtime::Shell, Status::Idle);
        window.since_secs = 0;
        // As in `long_names_are_truncated_with_an_ellipsis`: this is about grapheme-safe
        // truncation, not the branch marker `win()` happens to set.
        window.branch = None;
        let mut app = App::new(vec![window], "/tmp".into(), Keymap::default_prefix());
        app.set_terminal_size(80, 24);
        let (out, _) = render(&app, 120, 30);
        assert!(out.contains("… sh  0s│"), "{out}");
        assert!(!out.contains('\u{fffd}'));
    }
}
