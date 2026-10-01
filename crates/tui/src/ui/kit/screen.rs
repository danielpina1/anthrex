//! The full-body screens' frame and their scrolled view (milestone 9.0.6 decisions 5
//! and 33), shared by the Profile, Settings and stats screens.

use super::rows::scroll_marks;
use crate::safe_text::one_line;
use crate::theme::{Palette, Role, role};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};

/// `lines` cut to `rows`, keeping line `at` in view; what is cut is marked.
pub fn window(lines: Vec<Line<'static>>, at: usize, rows: usize, p: Palette) -> Vec<Line<'static>> {
    if lines.len() <= rows {
        return lines;
    }
    if rows < 3 {
        return lines.into_iter().skip(at).take(rows).collect();
    }
    // From the top while `at` fits above the one `↓` mark; else both marks.
    let (top, room) = if at + 1 < rows {
        (0, rows - 1)
    } else {
        let room = rows - 2;
        ((at + 1).saturating_sub(room).min(lines.len() - room), room)
    };
    let below = lines.len() - top - room;
    let (up, down) = scroll_marks(top, below, p.ascii);
    let muted = role(Role::Muted, p);
    let mut out: Vec<Line<'static>> = up.map(|m| Line::styled(m, muted)).into_iter().collect();
    out.extend(lines.into_iter().skip(top).take(room));
    out.extend(down.map(|m| Line::styled(m, muted)));
    out
}

/// A full-body screen's frame (Profile, Settings, Stats): a bold title, and a border
/// in the accent while the screen has the keys (`accent`), else muted (decision 5).
pub fn screen_frame(title: &str, accent: bool, p: Palette) -> Block<'static> {
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_style(role(if accent { Role::Accent } else { Role::Muted }, p))
        .title(Span::styled(
            format!(" {} ", crate::theme::fold(&one_line(title), p.ascii)),
            ratatui::style::Style::default().add_modifier(Modifier::BOLD),
        ));
    if p.ascii {
        block = block.border_set(crate::theme::ASCII_BORDER);
    }
    block
}
