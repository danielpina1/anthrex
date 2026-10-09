//! Milestone 8c decision 33 and 9.0.7 decision 35: rendering for the plan gate's task
//! edit form (`crate::run_edit`, the pure model), on the kit's dialog grammar: one
//! accented frame titled `edit <task>` (`kit::dialog_frame`, at most 64 wide), lower-case
//! labels, `‹ value ›` choices (`< value >` in ASCII), the brief in a four-row text area,
//! then the error, a blank row and the hints `⏎ save · tab next · ←/→ change · ^J
//! newline · esc cancel` (`⏎ choose model` on the model row, milestone 9.8), kept on a
//! short terminal while the fields scroll; the model picker is drawn over it
//! (`ui/modal.rs`). The plan's text passes `safe_text`.

use crate::run_edit::{EditField, TaskEditForm};
use crate::safe_text::one_line;
use crate::theme::{Palette, Role, role};
use crate::ui::dialog::{LABEL_WIDTH, MARKER_WIDTH, busy_hint, hint, interior};
use crate::ui::kit;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};

#[path = "run_edit_rows.rs"]
mod rows;

#[cfg(test)]
#[path = "run_edit_tests.rs"]
mod tests;

/// Rows of the brief's text area (decision 35), as the goal field's.
pub const BRIEF_ROWS: u16 = 4;
pub(crate) const VALUE_X: usize = MARKER_WIDTH + LABEL_WIDTH;

/// The width the brief's text area is drawn at in a terminal `width` wide, which its Up
/// and Down move by (`TaskEditForm::on_key_in`); 0 for a terminal not yet drawn.
pub fn brief_width(width: u16) -> u16 {
    interior(width).saturating_sub(VALUE_X as u16)
}

pub fn render(frame: &mut Frame, form: &TaskEditForm, area: Rect, p: Palette) {
    let width = interior(area.width);
    let value_w = brief_width(area.width);
    let (mut lines, mut at, mut cursor) = (Vec::new(), 0, None);
    for field in form.visible_fields() {
        let first = lines.len();
        if field == EditField::Brief {
            lines.extend(rows::brief_lines(form, value_w, p));
        } else {
            let (line, column) = rows::field_line(form, field, usize::from(value_w), p);
            cursor = column.map(|c| (first, c)).or(cursor);
            lines.push(line);
        }
        if form.focus == field {
            // The brief's label row and its four text rows, when they fit.
            at = (first + usize::from(BRIEF_ROWS) * usize::from(field == EditField::Brief))
                .min(lines.len() - 1);
        }
    }
    let mut tail = Vec::new();
    if let Some(error) = &form.error {
        let ellipsis = crate::theme::ellipsis(p);
        let text = kit::cut(&one_line(error), usize::from(width), ellipsis);
        tail.push(Line::styled(text, role(Role::Failed, p)));
    }
    tail.push(Line::raw(""));
    tail.push(if form.submitting {
        busy_hint("saving…", width, p)
    } else {
        // Milestone 9.8 decision 39: on the model row `⏎` opens the picker.
        let enter = if form.focus == EditField::Model {
            "choose model"
        } else {
            "save"
        };
        let keys = [
            hint("⏎", enter, 9),
            hint("tab", "next", 6),
            hint("←/→", "change", 5),
            hint("^J", "newline", 4),
            hint("esc", "cancel", 1),
        ];
        kit::hints_joined(width, &keys, " · ", p)
    });
    // A short terminal keeps the hints (then the error) and scrolls the fields, the
    // focused one in view.
    let rows = usize::from(area.height.saturating_sub(2));
    while tail.len() > rows.max(1) {
        tail.remove(tail.len() - 2);
    }
    let room = rows.saturating_sub(tail.len());
    let (top, shown) = kit::window_span(lines.len(), at, room);
    let shift = usize::from(top > 0 && room >= 3 && lines.len() > room);
    let mut body = kit::window(lines, at, room, p);
    body.extend(tail);

    let rect = kit::dialog_area(area, body.len() as u16);
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Clear, rect);
    let block = kit::dialog_frame(&format!("edit {}", form.task_id), false, p);
    let inner = block.inner(rect);
    frame.render_widget(Paragraph::new(body).block(block), rect);
    if let Some((line, column)) = cursor
        && line >= top
        && line - top < shown
        && value_w > 0
    {
        let row = (line + shift - top) as u16;
        frame.set_cursor_position((inner.x + VALUE_X as u16 + column, inner.y + row));
    }
}
