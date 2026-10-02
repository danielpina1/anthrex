//! The full-body screens' frame (milestone 9.0.6 decisions 5 and 33), shared by the
//! Profile, Settings and stats screens; their scrolled views are `scroll.rs`'s.

use crate::safe_text::one_line;
use crate::theme::{Palette, Role, role};
use ratatui::style::Modifier;
use ratatui::text::Span;
use ratatui::widgets::{Block, Borders};

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
