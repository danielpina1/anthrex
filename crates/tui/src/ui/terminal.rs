use crate::app::{App, region::KeyRegion};
use crate::theme;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::Paragraph;
use std::path::Path;
use tui_term::widget::{Cursor, PseudoTerminal};
use unicode_width::UnicodeWidthStr;

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

/// Decision 49's pane text for a headless run session's window.
pub fn headless_placeholder(w: &proto::WindowInfo, prefix: &str) -> String {
    format!(
        "  headless session · {} · {} · {prefix} m shows its conversation",
        w.runtime.label(),
        w.status.label()
    )
}

/// Milestone 9.0.7 decision 29's ` <head> · <dir><suffix> ` in `room` columns
/// (principle 6): `<tag> <name>` is kept; `<dir>` is cut with `…` first, then the
/// worktree suffix with it.
fn fit_title(head: &str, dir: &str, suffix: &str, room: usize, ascii: bool) -> String {
    use super::tree_view::truncate_in;
    use crate::safe_text::one_line;
    // Measured as drawn: `pane_frame` would sanitise them anyway.
    let (head, dir, suffix) = (one_line(head), one_line(dir), one_line(suffix));
    let full = format!("{head} · {dir}{suffix}");
    let fixed = UnicodeWidthStr::width(format!("{head} · {suffix}").as_str());
    if UnicodeWidthStr::width(full.as_str()) <= room {
        full
    } else if room >= fixed + 2 {
        let dir = truncate_in(&dir, room - fixed, ascii);
        format!("{head} · {dir}{suffix}")
    } else {
        truncate_in(&full, room, ascii)
    }
}

pub fn render(frame: &mut Frame, app: &App, area: Rect) {
    // The title and its two spaces inside the corners.
    let room = usize::from(area.width.saturating_sub(4));
    let ascii = app.palette().ascii;
    let title = match app.focused_window() {
        // Decision 37: the branch comes from `branch_text` alone, never a direct read
        // of `WindowInfo.branch` here — `None` is also how this tells a worktree
        // window from a plain one, since `branch_text` is `None` for exactly the
        // windows this daemon made no worktree for.
        // Milestone 9.0.7 decision 29: ` <tag> <name> · <dir> `, the runtime once.
        Some(w) => {
            let head = format!("{} {}", theme::runtime_tag(w.runtime), w.name);
            match super::tree_view::branch_text(w, app) {
                Some(branch) => {
                    let suffix = format!(" ({branch}, worktree)");
                    fit_title(&head, &shorten_home(&w.project), &suffix, room, ascii)
                }
                None => fit_title(&head, &shorten_home(&w.cwd), "", room, ascii),
            }
        }
        None => "no window".to_string(),
    };
    let keys_here = app.key_region() == KeyRegion::Pane;
    let block = super::kit::pane_frame(Line::from(title), keys_here, app.palette());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if app.focused.is_none() {
        let hint = format!(
            "No agents. Press {} c to create one, or run `anthrex new`.",
            app.settings.prefix_label
        );
        super::splash::render(frame, inner, &hint, app.palette());
        return;
    }

    if let Some(w) = app.focused_window().filter(|w| app.is_headless(w.id)) {
        // Decision 49: no terminal to draw, and no keys reach it; the prefix still works.
        let hint = vec![
            Line::raw(""),
            Line::styled(
                theme::fold(
                    &headless_placeholder(w, &app.settings.prefix_label),
                    app.palette().ascii,
                ),
                theme::role(theme::Role::Muted, app.palette()),
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

#[cfg(test)]
#[path = "terminal_tests.rs"]
mod tests;
