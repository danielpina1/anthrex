//! Milestone 9.3 decision 32 (KG §6): the idle orchestrator's menu on the kit's dialog
//! grammar, as the action menu draws its entries: the title `orchestrator <chain>`, the
//! two entries (`close` in `Failed`, its confirm page being destructive), a blank row
//! and the key hints. The chain id is the daemon's, so it is cleaned before it is cut
//! (`kit::dialog_frame` cleans the title again). Pure: rendering reads the menu.

use crate::app::idle_menu::{ENTRIES, IdleMenu};
use crate::safe_text::one_line;
use crate::theme::{Glyph, Palette, Role, dot_sep, ellipsis, glyph, role};
use crate::ui::kit::{self, Hint};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

fn hint(key: &str, word: &str, priority: u8) -> Hint {
    Hint {
        key: key.to_string(),
        word: word.to_string(),
        priority,
    }
}

/// The title, cleaned, then cut to `width` columns.
pub(crate) fn title(menu: &IdleMenu, width: u16, p: Palette) -> String {
    let text = format!("orchestrator {}", one_line(&menu.chain));
    kit::cut(&text, usize::from(width), ellipsis(p))
}

/// The entries, a blank row and the hints, `width` columns wide.
pub(crate) fn body(menu: &IdleMenu, width: u16, p: Palette) -> Vec<Line<'static>> {
    let mut body: Vec<Line<'static>> = (ENTRIES.iter().enumerate())
        .map(|(i, label)| {
            let selected = i == menu.selected;
            let mark = if selected {
                glyph(Glyph::Selection, p.ascii)
            } else {
                " "
            };
            let mut style = if i == 1 {
                role(Role::Failed, p)
            } else {
                Style::default()
            };
            if selected {
                style = style.add_modifier(Modifier::BOLD);
            }
            Line::from(vec![
                Span::styled(format!("{mark} "), role(Role::Accent, p)),
                Span::styled(*label, style),
            ])
        })
        .collect();
    body.push(Line::raw(""));
    body.push(kit::hints_joined(
        width,
        &[
            hint("⏎", "choose", 9),
            hint("j/k", "move", 5),
            hint("esc", "close", 1),
        ],
        dot_sep(p),
        p,
    ));
    body
}

/// Draws the menu over `area`.
pub fn render(frame: &mut Frame, menu: &IdleMenu, area: Rect, p: Palette) {
    let width = area.width.min(kit::DIALOG_MAX).saturating_sub(4);
    let body = body(menu, width, p);
    let rect = kit::dialog_area(area, body.len() as u16);
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Clear, rect);
    let title = title(menu, width.saturating_sub(2), p);
    frame.render_widget(
        Paragraph::new(body).block(kit::dialog_frame(&title, false, p)),
        rect,
    );
}

#[cfg(test)]
#[path = "idle_menu_tests.rs"]
mod tests;
