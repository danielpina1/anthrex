//! The dialog frame, its placement and the `‹ value ›` choice (decision 5).

use super::DIALOG_MAX;
use crate::safe_text::one_line;
use crate::theme::{Palette, Role, role};
use ratatui::layout::Rect;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders, Padding};

/// Where a dialog with `content_rows` interior rows goes in `area`: `min(64, width)`
/// columns, the content plus the two border rows tall (cut to the area), centred.
pub fn dialog_area(area: Rect, content_rows: u16) -> Rect {
    let width = area.width.min(DIALOG_MAX);
    let height = content_rows.saturating_add(2).min(area.height);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

/// The frame's one accented border (principle 2), one column of padding, and a title
/// in the accent, or in `Failed` when the dialog is destructive.
pub fn dialog_frame(title: &str, destructive: bool, p: Palette) -> Block<'static> {
    let title_role = if destructive {
        Role::Failed
    } else {
        Role::Accent
    };
    let set = if p.ascii {
        ratatui::symbols::border::Set {
            top_left: "+",
            top_right: "+",
            bottom_left: "+",
            bottom_right: "+",
            vertical_left: "|",
            vertical_right: "|",
            horizontal_top: "-",
            horizontal_bottom: "-",
        }
    } else {
        ratatui::symbols::border::PLAIN
    };
    Block::default()
        .borders(Borders::ALL)
        .border_set(set)
        .border_style(role(Role::Accent, p))
        .padding(Padding::horizontal(1))
        .title(Span::styled(
            format!(" {} ", one_line(title).trim()),
            role(title_role, p),
        ))
}

/// A choice field's value: `‹ value ›`.
pub fn choice(value: &str) -> String {
    format!("‹ {} ›", one_line(value))
}

/// [`choice`] honouring the palette's `ascii` flag: `< value >` in ASCII.
pub fn choice_in(value: &str, p: Palette) -> String {
    if p.ascii {
        format!("< {} >", one_line(value))
    } else {
        choice(value)
    }
}
