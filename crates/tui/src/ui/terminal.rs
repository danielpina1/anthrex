use crate::app::App;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use std::path::Path;
use tui_term::widget::{Cursor, PseudoTerminal};

pub fn shorten_home(path: &Path) -> String {
    if let Some(home) = dirs::home_dir() {
        if let Ok(rest) = path.strip_prefix(&home) {
            return format!("~/{}", rest.display());
        }
    }
    path.display().to_string()
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let title = match app.focused_window() {
        Some(w) => {
            let branch = w.branch.as_ref().map(|b| format!(" ({b})")).unwrap_or_default();
            format!(" {} · {} · {}{branch} ", w.name, w.runtime.label(), shorten_home(&w.cwd))
        }
        None => " no window ".to_string(),
    };
    let border = if app.modal.is_none() { theme::border_focused() } else { theme::border() };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::from(Span::styled(title, theme::title())));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.focused.is_none() {
        let hint = vec![
            Line::raw(""),
            Line::styled("  No agents. Press C-b c to open a shell here, or run `anthrex new`.", theme::muted()),
        ];
        frame.render_widget(Paragraph::new(hint), inner);
        return;
    }

    let screen = app.parser.screen();
    // We place the hardware cursor ourselves, so the widget's drawn cursor stays hidden.
    let widget = PseudoTerminal::new(screen).cursor(Cursor::default().visibility(false));
    frame.render_widget(widget, inner);

    if !screen.hide_cursor() && app.scroll_offset == 0 && app.modal.is_none() {
        let (row, col) = screen.cursor_position();
        if row < inner.height && col < inner.width {
            frame.set_cursor_position((inner.x + col, inner.y + row));
        }
    }
}
