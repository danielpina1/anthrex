//! The frame every pane draws (milestone 9.0.7 decision 2): the sidebar, the Alerts
//! box, the terminal pane, the overview, the conversation view, the inspector panel
//! and the generic modal box.

use crate::safe_text::one_line;
use crate::theme::{self, Palette, Role, role};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};

/// A rounded (or `+ - |`) border, in the accent while the keys are here
/// (`keys_here`, from `App::key_region`), else muted (§5.1 principle 2). The title's
/// spans as given, one space each side, bold in the default colour: titles use
/// weight, not the accent. Each span passes `safe_text::one_line`, as every kit
/// widget's text does, and `theme::fold` in ASCII. An empty title draws none. No
/// padding.
pub fn pane_frame(title: Line<'static>, keys_here: bool, p: Palette) -> Block<'static> {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_set(theme::border_set(p.ascii))
        .border_style(role(if keys_here { Role::Accent } else { Role::Muted }, p));
    if title.width() == 0 {
        return block;
    }
    let mut spans = Vec::with_capacity(title.spans.len() + 2);
    spans.push(Span::raw(" "));
    spans.extend(
        title
            .spans
            .into_iter()
            .map(|span| Span::styled(theme::fold(&one_line(&span.content), p.ascii), span.style)),
    );
    spans.push(Span::raw(" "));
    // The default foreground, set, so the accented border under the title does not
    // tint it.
    let weight = Style {
        fg: Style::reset().fg,
        ..Style::default().add_modifier(Modifier::BOLD)
    };
    block.title(Line::from(spans).style(weight.patch(title.style)))
}

/// The selection bar (final fix wave M3: one idiom for every selectable row). A
/// selected row is reversed, and `▌` (`>` in ASCII) in the accent stands in the column
/// left of it: a frame's left border cell on that row, as a graph box's (milestone
/// 9.0.7 decision 20). A cell off the buffer is left alone.
pub fn selection_bar(buf: &mut ratatui::buffer::Buffer, x: u16, y: u16, p: Palette) {
    if let Some(cell) = buf.cell_mut((x, y)) {
        cell.set_symbol(theme::glyph(theme::Glyph::Selection, p.ascii));
        cell.set_style(role(Role::Accent, p));
    }
}
