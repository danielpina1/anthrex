//! Screen layout and the top-level draw. Spec section 6.1.

pub mod modal;
pub mod sidebar;
pub mod statusbar;
pub mod terminal;

use crate::app::App;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout as RLayout, Rect};

pub const SIDEBAR_WIDTH: u16 = 30;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Layout {
    pub sidebar: Rect,
    pub sidebar_inner: Rect,
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

pub fn layout(area: Rect, sidebar_visible: bool) -> Layout {
    let [body, statusbar] =
        RLayout::vertical([Constraint::Min(3), Constraint::Length(1)]).areas(area);
    let (sidebar, main) = if sidebar_visible {
        let [s, m] = RLayout::horizontal([Constraint::Length(SIDEBAR_WIDTH), Constraint::Min(10)])
            .areas(body);
        (s, m)
    } else {
        (Rect::new(body.x, body.y, 0, body.height), body)
    };
    Layout {
        sidebar,
        sidebar_inner: inset(sidebar),
        main,
        main_inner: inset(main),
        statusbar,
    }
}

/// Draws everything and returns the layout so the caller can size the PTY and hit-test the mouse.
pub fn draw(frame: &mut Frame, app: &App) -> Layout {
    let l = layout(frame.area(), app.sidebar_visible);
    if app.sidebar_visible {
        sidebar::render(frame, app, l.sidebar);
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

    #[test]
    fn layout_splits_sidebar_main_and_statusbar() {
        let l = layout(ratatui::layout::Rect::new(0, 0, 120, 40), true);
        assert_eq!(l.sidebar.width, SIDEBAR_WIDTH);
        assert_eq!(l.main.x, SIDEBAR_WIDTH);
        assert_eq!(l.main.width, 120 - SIDEBAR_WIDTH);
        assert_eq!(l.statusbar, ratatui::layout::Rect::new(0, 39, 120, 1));
        assert_eq!(
            l.main_inner,
            ratatui::layout::Rect::new(SIDEBAR_WIDTH + 1, 1, 120 - SIDEBAR_WIDTH - 2, 37)
        );
        let hidden = layout(ratatui::layout::Rect::new(0, 0, 120, 40), false);
        assert_eq!(hidden.sidebar.width, 0);
        assert_eq!(hidden.main.width, 120);
    }

    #[test]
    fn sidebar_lists_cards_with_glyph_runtime_status_and_elapsed() {
        let mut app = App::new(
            vec![
                win(1, "api-worker", Runtime::Claude, Status::Working),
                win(2, "tests", Runtime::Codex, Status::Attention),
            ],
            "/tmp".into(),
            Keymap::default_prefix(),
        );
        let _ = app.set_terminal_size(80, 24);
        let (out, _) = render(&app, 100, 20);
        assert!(out.contains("anthrex"));
        assert!(out.contains("1 api-worker"));
        assert!(out.contains("claude · working · 1m"));
        assert!(out.contains("◆ 2 tests"));
        assert!(out.contains("codex · attention · 1m"));
        assert!(
            out.contains("2 agents · 1 working"),
            "footer is truncated to the 28-column sidebar\n{out}"
        );
        assert!(
            out.contains("api-worker · claude · /tmp/repo (feat/x)"),
            "main title\n{out}"
        );
    }

    #[test]
    fn a_working_card_shows_its_tool_on_a_third_line() {
        let mut window = win(1, "api-worker", Runtime::Claude, Status::Working);
        window.tool = Some("Bash".into());
        let mut app = App::new(
            vec![window.clone()],
            "/tmp".into(),
            Keymap::default_prefix(),
        );
        let _ = app.set_terminal_size(80, 24);
        let (out, _) = render(&app, 100, 20);
        let rows: Vec<_> = out.lines().collect();
        let detail_row = rows
            .iter()
            .position(|row| row.contains("claude · working"))
            .expect("working detail row");
        assert!(rows[detail_row + 1].contains("Bash"), "{out}");

        window.status = Status::Idle;
        let mut app = App::new(vec![window], "/tmp".into(), Keymap::default_prefix());
        let _ = app.set_terminal_size(80, 24);
        let (out, _) = render(&app, 100, 20);
        assert!(
            !out.contains("Bash"),
            "idle cards do not show a tool\n{out}"
        );
    }

    #[test]
    fn long_tool_names_are_cut_to_the_sidebar() {
        let mut window = win(1, "api-worker", Runtime::Claude, Status::Working);
        window.tool = Some("界".repeat(20));
        let mut app = App::new(vec![window], "/tmp".into(), Keymap::default_prefix());
        let _ = app.set_terminal_size(80, 24);
        let (out, _) = render(&app, 100, 20);
        assert!(
            out.contains("▎  界界界界界界界界界界界界…│"),
            "the 28-column inner width is measured in terminal columns\n{out}"
        );
    }

    #[test]
    fn emoji_tool_names_are_cut_at_grapheme_boundaries() {
        let emoji = "👩🏽‍💻";
        let mut window = win(1, "api-worker", Runtime::Claude, Status::Working);
        window.tool = Some(emoji.repeat(13));
        let mut app = App::new(vec![window], "/tmp".into(), Keymap::default_prefix());
        let _ = app.set_terminal_size(80, 24);
        let (out, _) = render(&app, 100, 20);
        let expected = format!("▎  {}…│", emoji.repeat(12));
        assert!(
            out.contains(&expected),
            "emoji modifiers and ZWJ sequences stay intact\n{out}"
        );
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
    fn sidebar_hit_test_maps_rows_to_cards() {
        let mut first = win(1, "a", Runtime::Claude, Status::Working);
        first.tool = Some("Bash".into());
        let mut app = App::new(
            vec![first, win(2, "b", Runtime::Shell, Status::Idle)],
            "/tmp".into(),
            Keymap::default_prefix(),
        );
        let _ = app.set_terminal_size(80, 24);
        let (_, l) = render(&app, 100, 20);
        for offset in 0..sidebar::card_height(&app.windows[0]) {
            assert_eq!(
                sidebar::hit_test(l.sidebar_inner, &app, 3, l.sidebar_inner.y + offset),
                Some(0),
                "first card row {offset}"
            );
        }
        for offset in 3..5 {
            assert_eq!(
                sidebar::hit_test(l.sidebar_inner, &app, 3, l.sidebar_inner.y + offset),
                Some(1),
                "second card row {offset}"
            );
        }
        assert_eq!(
            sidebar::hit_test(l.sidebar_inner, &app, 3, l.sidebar_inner.y + 5),
            None
        );
        assert_eq!(
            sidebar::hit_test(l.sidebar_inner, &app, 60, l.sidebar_inner.y),
            None
        );
        assert_eq!(sidebar::format_elapsed(59), "59s");
        assert_eq!(sidebar::format_elapsed(3600), "1h");
    }

    #[test]
    fn sidebar_hit_test_ignores_footer_and_undrawn_cards() {
        let mut first = win(1, "a", Runtime::Claude, Status::Working);
        first.tool = Some("Bash".into());
        let mut third = win(3, "c", Runtime::Claude, Status::Working);
        third.tool = Some("Read".into());
        let mut app = App::new(
            vec![
                first,
                win(2, "b", Runtime::Shell, Status::Idle),
                third,
                win(4, "d", Runtime::Shell, Status::Idle),
                win(5, "e", Runtime::Shell, Status::Idle),
            ],
            "/tmp".into(),
            Keymap::default_prefix(),
        );
        let _ = app.set_terminal_size(80, 24);
        // Six list rows fit the first two cards (3 + 2), but not the next three-row card.
        let inner = ratatui::layout::Rect::new(1, 1, 28, 8);
        let expected = [Some(0), Some(0), Some(0), Some(1), Some(1), None];
        for (offset, want) in expected.into_iter().enumerate() {
            assert_eq!(
                sidebar::hit_test(inner, &app, 3, inner.y + offset as u16),
                want,
                "row offset {offset}"
            );
        }
        assert_eq!(
            sidebar::hit_test(inner, &app, 3, inner.y + 6),
            None,
            "spacer row"
        );
        assert_eq!(
            sidebar::hit_test(inner, &app, 3, inner.y + 7),
            None,
            "footer row"
        );
        assert_eq!(
            sidebar::hit_test(inner, &app, 3, inner.y + 8),
            None,
            "below the rect"
        );
    }
}
