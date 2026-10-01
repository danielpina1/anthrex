//! The frame every pane draws (milestone 9.0.7 decision 2): the sidebar, the Alerts
//! box, the terminal pane, the overview, the conversation view, the inspector panel
//! and the generic modal box.

use crate::safe_text::one_line;
use crate::theme::{self, Palette, Role, role};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};

/// A rounded (or `+ - |`) border, in the accent while the keys are here
/// (`keys_here`, from `App::key_region`), else muted (§5.1 principle 2). The title's
/// spans as given, one space each side, bold in the default colour: titles use
/// weight, not the accent. Each span passes `safe_text::one_line`, as every kit
/// widget's text does. An empty title draws none. No padding.
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
            .map(|span| Span::styled(one_line(&span.content), span.style)),
    );
    spans.push(Span::raw(" "));
    let weight = Style::default()
        .fg(Color::Reset)
        .add_modifier(Modifier::BOLD);
    block.title(Line::from(spans).style(weight.patch(title.style)))
}
