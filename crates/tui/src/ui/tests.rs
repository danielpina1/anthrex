//! The screen as a whole: the layout, what a frame draws into each pane, and
//! that the hit tests read the same geometry the frame was drawn with.
//!
//! Split out of `ui/mod.rs` when milestone 4.7 pushed that file past 600 lines. The
//! graph overview's own tests are split further still, into `ui_tests/overview.rs`
//! (task M6.10's file-size finding B), the same sibling-directory shape
//! `app/tests.rs` gives `app_tests/*.rs`.

#[path = "../ui_tests/overview.rs"]
mod overview_tests;

use super::*;
use crate::app::{App, Modal, PendingAction};
use crate::graph::Pan;
use crate::settings::UiSettings;
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
        UiSettings::default(),
    );
    app.set_terminal_size(80, 24);
    app
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
    let mut app = App::new(vec![window], "/tmp".into(), UiSettings::default());
    app.set_terminal_size(80, 24);
    let (out, _) = render(&app, 120, 30);
    assert!(out.contains("… sh  0s│"), "{out}");
}

#[test]
fn empty_state_and_hidden_sidebar() {
    let mut app = App::new(vec![], "/tmp".into(), UiSettings::default());
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
    let mut app = App::new(vec![window], "/tmp".into(), UiSettings::default());
    let _ = app.set_terminal_size(80, 24);
    let (out, _) = render(&app, 100, 20);
    assert!(out.contains("(feat/x, worktree)"), "{out}");
    assert!(out.contains("/tmp/shop"), "{out}");
    assert!(!out.contains("worktrees/shop-abcd"), "{out}");

    let mut plain = win(2, "plain", Runtime::Shell, Status::Idle);
    plain.branch = None;
    let mut app = App::new(vec![plain], "/tmp".into(), UiSettings::default());
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
        UiSettings::default(),
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
        UiSettings::default(),
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

/// Task M6.10: the rename box shows its title, the input text with a cursor block, and
/// (when set) the inline error, matching the brief's own mock verbatim.
#[test]
fn rename_modal_renders() {
    let mut app = App::new(
        vec![win(1, "api-worker", Runtime::Shell, Status::Idle)],
        "/tmp".into(),
        UiSettings::default(),
    );
    let _ = app.set_terminal_size(80, 24);
    app.modal = Some(Modal::Rename(crate::app::prompt::RenamePrompt::new(
        1,
        "api-worker",
    )));
    let (out, _) = render(&app, 100, 20);
    assert!(out.contains(" rename "), "{out}");
    assert!(out.contains("api-worker█"), "{out}");
    assert!(out.contains("Enter = rename"), "{out}");
    assert!(out.contains("Esc = cancel"), "{out}");

    if let Some(Modal::Rename(prompt)) = &mut app.modal {
        prompt.error = Some("a window named 'api' exists".to_string());
    }
    let (out, _) = render(&app, 100, 20);
    assert!(out.contains("a window named 'api' exists"), "{out}");
}

#[test]
fn modals_render_on_top() {
    let mut app = App::new(
        vec![win(1, "a", Runtime::Shell, Status::Idle)],
        "/tmp".into(),
        UiSettings::default(),
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

/// Task M6.9 decision 38: every piece of help or hint text takes the prefix from
/// `app.settings.prefix_label`, never a hard-coded `C-b`.
///
/// Task M6.10 finding A: the previous task's sweep fixed the *handled* surface (the
/// key the keymap matches) but missed the *displayed* surface in three places — the
/// empty main pane's hint (`ui/terminal.rs`), the empty sidebar's hint
/// (`ui/sidebar.rs`) and the disconnect toast (`lib.rs`, covered by
/// `app::tests::lifecycle::link_lost_toast_uses_the_configured_prefix` since it is pure
/// `App` state, not a render). Both empty-state hints are checked here so a hint that
/// regresses back to a hard-coded `C-b` fails a render test, not just a reading of the
/// source.
#[test]
fn help_and_hints_use_the_configured_prefix() {
    let settings = UiSettings {
        prefix_label: "C-a".into(),
        ..UiSettings::default()
    };
    let mut app = App::new(
        vec![win(1, "a", Runtime::Shell, Status::Idle)],
        "/tmp".into(),
        settings.clone(),
    );
    let _ = app.set_terminal_size(80, 24);

    app.modal = Some(Modal::Help);
    let (out, _) = render(&app, 100, 30);
    assert!(out.contains("C-a c"), "{out}");
    assert!(out.contains("send a literal C-a"), "{out}");
    assert!(!out.contains("C-b"), "{out}");

    app.modal = None;
    let (out, _) = render(&app, 100, 20);
    assert!(out.contains("C-a ?"), "{out}");
    assert!(!out.contains("C-b"), "{out}");

    let mut empty = App::new(vec![], "/tmp".into(), settings);
    let _ = empty.set_terminal_size(80, 24);
    let (out, _) = render(&empty, 100, 20);
    assert!(out.contains("C-a c to create one"), "{out}");
    assert!(out.contains("C-a c opens a shell"), "{out}");
    assert!(!out.contains("C-b"), "{out}");
}

/// Task M6.9: `app.settings.accent` colours the focused main pane's border, not the
/// built-in `theme::DEFAULT_ACCENT`.
#[test]
fn accent_colours_the_focused_border() {
    let accent = ratatui::style::Color::Rgb(0x11, 0x22, 0x33);
    let settings = UiSettings {
        accent,
        ..UiSettings::default()
    };
    let mut app = App::new(
        vec![win(1, "a", Runtime::Shell, Status::Idle)],
        "/tmp".into(),
        settings,
    );
    let _ = app.set_terminal_size(80, 24);
    let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
    let mut l = None;
    terminal.draw(|f| l = Some(draw(f, &app))).unwrap();
    let l = l.unwrap();
    assert_eq!(terminal.backend().buffer()[(l.main.x, l.main.y)].fg, accent);
}

#[test]
fn layout_uses_the_configured_sidebar_width() {
    let settings = UiSettings {
        sidebar_width: 40,
        ..UiSettings::default()
    };
    let mut app = App::new(
        vec![win(1, "a", Runtime::Shell, Status::Idle)],
        "/tmp".into(),
        settings,
    );
    assert_eq!(app.sidebar_width, 40);
    let _ = app.set_terminal_size(80, 24);
    let (_, l) = render(&app, 120, 30);
    assert_eq!(l.sidebar.width, 40);

    // `C-b <` still steps it by the built-in constant, unaffected by the starting width.
    app.on_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('b'),
        crossterm::event::KeyModifiers::CONTROL,
    ));
    app.on_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('<'),
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.sidebar_width, 36);
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
        UiSettings::default(),
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
        let mut app = App::new(vec![window], "/tmp".into(), UiSettings::default());
        app.set_terminal_size(80, 24);
        let (out, _) = render(&app, 120, 30);
        assert!(out.contains("… sh  0s│"), "{out}");
        assert!(!out.contains('\u{fffd}'));
    }
}

/// Task M6.5.13: the open conversation view takes the main area in preference to both
/// the overview and the terminal, and help lists its key.
#[test]
fn the_conversation_view_takes_the_main_area() {
    let mut app = App::new(
        vec![win(1, "orchestrator", Runtime::Claude, Status::Idle)],
        "/tmp".into(),
        UiSettings::default(),
    );
    let _ = app.set_terminal_size(80, 24);
    app.overview = true;
    let (out, _) = render(&app, 120, 30);
    assert!(out.contains("tree overview"), "{out}");

    app.on_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('b'),
        crossterm::event::KeyModifiers::CONTROL,
    ));
    app.on_key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('m'),
        crossterm::event::KeyModifiers::NONE,
    ));
    let (out, _) = render(&app, 120, 30);
    assert!(out.contains("waiting for the conversation"), "{out}");
    assert!(!out.contains("tree overview"), "{out}");

    app.overview = false;
    let (out, _) = render(&app, 120, 30);
    assert!(out.contains("waiting for the conversation"), "{out}");

    app.modal = Some(Modal::Help);
    let (out, _) = render(&app, 120, 40);
    let line = out
        .lines()
        .find(|l| l.contains("C-b m"))
        .unwrap_or_else(|| panic!("{out}"));
    assert!(line.contains("conversation"), "{line}");
}
