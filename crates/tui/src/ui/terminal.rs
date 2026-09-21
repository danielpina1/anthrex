use crate::app::App;
use crate::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph};
use std::path::Path;
use tui_term::widget::{Cursor, PseudoTerminal};

/// Pure variant of [`shorten_home`]: takes the home directory as a parameter instead of
/// reading it from the environment, so callers with no filesystem access — the new-agent
/// form's defaults (decision 31) among them — can use it too.
pub fn shorten_home_with(path: &Path, home: Option<&Path>) -> String {
    if let Some(home) = home
        && let Ok(rest) = path.strip_prefix(home)
    {
        return format!("~/{}", rest.display());
    }
    path.display().to_string()
}

pub fn shorten_home(path: &Path) -> String {
    shorten_home_with(path, dirs::home_dir().as_deref())
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    let title = match app.focused_window() {
        // Decision 37: the branch comes from `branch_text` alone, never a direct read
        // of `WindowInfo.branch` here — `None` is also how this tells a worktree
        // window from a plain one, since `branch_text` is `None` for exactly the
        // windows this daemon made no worktree for.
        Some(w) => match super::tree_view::branch_text(w, app) {
            Some(branch) => format!(
                " {} · {} · {} ({branch}, worktree) ",
                w.name,
                w.runtime.label(),
                shorten_home(&w.project)
            ),
            None => format!(
                " {} · {} · {} ",
                w.name,
                w.runtime.label(),
                shorten_home(&w.cwd)
            ),
        },
        None => " no window ".to_string(),
    };
    let border = if app.modal.is_none() {
        theme::border_focused(app.settings.accent)
    } else {
        theme::border()
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border)
        .title(Line::from(Span::styled(
            title,
            theme::title(app.settings.accent),
        )));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.focused.is_none() {
        let hint = vec![
            Line::raw(""),
            Line::styled(
                "  No agents. Press C-b c to create one, or run `anthrex new`.",
                theme::muted(),
            ),
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
