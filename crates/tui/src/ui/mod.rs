//! Screen layout and the top-level draw. Spec section 6.1.

pub mod modal;
pub mod sidebar;
pub mod statusbar;
pub mod terminal;
pub mod tree_view;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout as RLayout, Rect};

pub const DEFAULT_SIDEBAR_WIDTH: u16 = 34;
pub const MIN_SIDEBAR_WIDTH: u16 = 24;
pub const MAX_SIDEBAR_WIDTH: u16 = 60;
pub const SIDEBAR_WIDTH_STEP: u16 = 4;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub sidebar: Rect,
    pub sidebar_inner: Rect,
    pub sidebar_list: Rect,
    pub sidebar_footer: Rect,
    pub main: Rect,
    pub main_inner: Rect,
    pub statusbar: Rect,
}

fn inset(r: Rect) -> Rect {
    Rect {
        x: r.x.saturating_add(1),
        y: r.y.saturating_add(1),
        width: r.width.saturating_sub(2),
        height: r.height.saturating_sub(2),
    }
}

pub fn layout(area: Rect, sidebar_width: u16) -> Layout {
    let [body, statusbar] =
        RLayout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
    let (sidebar, main) = if sidebar_width > 0 {
        let [s, m] = RLayout::horizontal([Constraint::Length(sidebar_width), Constraint::Min(10)])
            .areas(body);
        (s, m)
    } else {
        (Rect::new(body.x, body.y, 0, body.height), body)
    };
    let sidebar_inner = inset(sidebar);
    Layout {
        sidebar,
        sidebar_inner,
        sidebar_list: Rect {
            height: sidebar_inner.height.saturating_sub(2),
            ..sidebar_inner
        },
        sidebar_footer: Rect {
            y: sidebar_inner.y + sidebar_inner.height.saturating_sub(1),
            height: sidebar_inner.height.min(1),
            ..sidebar_inner
        },
        main,
        main_inner: inset(main),
        statusbar,
    }
}

/// Draws everything and returns the layout so the caller can size the PTY and hit-test the mouse.
pub fn draw(frame: &mut Frame, app: &App) -> Layout {
    let l = layout(
        frame.area(),
        if app.sidebar_visible {
            app.sidebar_width
        } else {
            0
        },
    );
    if app.sidebar_visible {
        sidebar::render(frame, app, &l);
    }
    terminal::render(frame, app, l.main);
    statusbar::render(frame, app, l.statusbar);
    if let Some(m) = &app.modal {
        modal::render(frame, m, frame.area());
    }
    l
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::{App, Modal, PendingAction};
    use crate::keymap::Keymap;
    use proto::{Runtime, Status, WindowInfo};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn win(id: u32, name: &str, runtime: Runtime, status: Status) -> WindowInfo {
        WindowInfo {
            id,
            name: name.into(),
            runtime,
            cwd: "/tmp/repo".into(),
            project: "/tmp/repo".into(),
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

    #[test]
    fn sidebar_renders_the_example_tree_at_the_default_width() {
        let app = example_app();
        let (out, _) = render(&app, 120, 30);
        assert!(out.contains(" agents "), "{out}");
        for golden in [
            "▾ shop            ◆  cl 4 · cx 3",
            "▎ ⠋ 1 api-worker     cl opus  2m",
            "  │ ├ ⠋ Explore: map routes Read",
            "▾ blog                   ○  cl 1",
        ] {
            assert!(out.contains(golden), "{golden:?}\n{out}");
        }
        assert!(out.contains("│ └ ✓ tests: run unit suite"), "{out}");
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
        assert!(out.contains("│ ├ ◆ Explore: map routes"), "{out}");
        assert!(out.contains("│ └ ✓ tests: run unit suite"), "{out}");
        assert_eq!(
            terminal.backend().buffer()[(7, 3)].fg,
            crate::theme::status_color(Status::Attention)
        );
        app.windows[0].subagents[0].needs_permission = false;
        let (out, _) = render(&app, 120, 30);
        assert!(out.contains("│ ├ ⠋ Explore: map routes"), "{out}");
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
        assert!(out.contains("No agents"));
        app.sidebar_visible = false;
        let (out, l) = render(&app, 100, 20);
        assert!(!out.contains("no agents yet"));
        assert_eq!(l.sidebar.width, 0);
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
        let windows = app.windows.clone();
        app.on_daemon(proto::DaemonMsg::WindowsChanged { windows });
        assert_eq!(app.tree.sidebar.top, 12);
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
            let mut app = App::new(vec![window], "/tmp".into(), Keymap::default_prefix());
            app.set_terminal_size(80, 24);
            let (out, _) = render(&app, 120, 30);
            assert!(out.contains("… sh  0s│"), "{out}");
            assert!(!out.contains('\u{fffd}'));
        }
    }
}
