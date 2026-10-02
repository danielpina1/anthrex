//! Milestone 8c decision 33 and 9.0.7 decision 35: rendering for the plan gate's task
//! edit form (`crate::run_edit`, the pure model), on the kit's dialog grammar: one
//! accented frame titled `edit <task>` (`kit::dialog_frame`, at most 64 wide), lower-case
//! labels, `‹ value ›` choices (`< value >` in ASCII), the brief in a four-row text area,
//! then the error, a blank row and the hints `⏎ save · tab next · ←/→ change · ^J
//! newline · esc cancel`. The plan's text passes `safe_text`.

use crate::run_edit::{EditField, TaskEditForm, field_label};
use crate::safe_text::one_line;
use crate::theme::{self, Glyph, Palette, Role, glyph, role};
use crate::ui::dialog::{LABEL_WIDTH, MARKER_WIDTH, busy_hint, hint, interior};
use crate::ui::kit;
use crate::ui::run_goal::starts_with_mark;
use crate::ui::tree_view::truncate_in;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

#[cfg(test)]
#[path = "run_edit_tests.rs"]
mod tests;

/// Rows of the brief's text area (decision 35), as the goal field's.
pub const BRIEF_ROWS: u16 = 4;
const VALUE_X: usize = MARKER_WIDTH + LABEL_WIDTH;

/// The width the brief's text area is drawn at in a terminal `width` wide, which its Up
/// and Down move by (`TaskEditForm::on_key_in`); 0 for a terminal not yet drawn.
pub fn brief_width(width: u16) -> u16 {
    interior(width).saturating_sub(VALUE_X as u16)
}

fn is_text(field: EditField) -> bool {
    matches!(
        field,
        EditField::Model | EditField::Reason | EditField::Brief
    )
}

/// `marker label` for `field`: the focused field's label is accented and bold and led
/// by the selection glyph (a cue that does not depend on colour), as the goal form's.
fn label(form: &TaskEditForm, field: EditField, p: Palette) -> Vec<Span<'static>> {
    let focused = form.focus == field && !form.submitting;
    let (mark, style) = if focused {
        (
            glyph(Glyph::Selection, p.ascii),
            role(Role::Accent, p).add_modifier(Modifier::BOLD),
        )
    } else {
        (" ", role(Role::Muted, p))
    };
    vec![
        Span::styled(format!("{mark:<MARKER_WIDTH$}"), style),
        Span::styled(format!("{:<LABEL_WIDTH$}", field_label(field)), style),
    ]
}

/// The brief's rows: the text area's, the label on its first text row, always
/// `BRIEF_ROWS + 2` so the dialog keeps its height when a scroll mark appears.
fn brief_lines(form: &TaskEditForm, value_w: u16, p: Palette) -> Vec<Line<'static>> {
    let focused = form.focus == EditField::Brief && !form.submitting;
    let mut area = kit::text_area_focus(&form.brief, BRIEF_ROWS, value_w, focused, p);
    area.resize(usize::from(BRIEF_ROWS) + 2, Line::default());
    let first = usize::from(starts_with_mark(&area));
    area.into_iter()
        .enumerate()
        .map(|(i, line)| {
            let mut spans = if i == first {
                label(form, EditField::Brief, p)
            } else {
                vec![Span::raw(" ".repeat(VALUE_X))]
            };
            spans.extend(line.spans);
            Line::from(spans)
        })
        .collect()
}

/// One field's row, and the cursor's column in it when it is the focused text field.
fn field_line(
    form: &TaskEditForm,
    field: EditField,
    value_w: usize,
    p: Palette,
) -> (Line<'static>, Option<u16>) {
    let focused = form.focus == field && !form.submitting;
    let mut spans = label(form, field, p);
    let (value, resolved) = form.value_parts_in(field, p);
    let input = match field {
        EditField::Model => Some(&form.model),
        EditField::Reason => Some(&form.reason),
        _ => None,
    };
    let mut cursor = None;
    match input.filter(|input| !input.text().is_empty()) {
        Some(input) => {
            let (visible, column) = input.visible(value_w as u16);
            spans.push(Span::raw(one_line(&visible)));
            cursor = focused.then_some(column);
        }
        None => {
            let value = truncate_in(&one_line(&value), value_w, p.ascii);
            let style = if focused && !is_text(field) {
                role(Role::Accent, p).add_modifier(Modifier::BOLD)
            } else {
                Style::default()
            };
            let room = value_w.saturating_sub(value.width() + 2);
            spans.push(Span::styled(value, style));
            if let Some(resolved) = resolved.filter(|_| room > 0) {
                spans.push(Span::raw("  "));
                spans.push(Span::styled(
                    truncate_in(&one_line(&resolved), room, p.ascii),
                    role(Role::Muted, p),
                ));
            }
            if focused && is_text(field) {
                cursor = Some(0);
            }
        }
    }
    (Line::from(spans), cursor)
}

pub fn render(frame: &mut Frame, form: &TaskEditForm, area: Rect, p: theme::Palette) {
    let width = interior(area.width);
    let value_w = brief_width(area.width);
    let mut body = Vec::new();
    let mut cursor = None;
    for field in form.visible_fields() {
        if field == EditField::Brief {
            body.extend(brief_lines(form, value_w, p));
            continue;
        }
        let (line, column) = field_line(form, field, usize::from(value_w), p);
        if let Some(column) = column {
            cursor = Some((body.len() as u16, column));
        }
        body.push(line);
    }
    if let Some(error) = &form.error {
        let ellipsis = if p.ascii { "..." } else { "…" };
        body.push(Line::styled(
            kit::cut(&one_line(error), usize::from(width), ellipsis),
            role(Role::Failed, p),
        ));
    }
    body.push(Line::raw(""));
    body.push(if form.submitting {
        busy_hint("saving…", width, p)
    } else {
        let keys = [
            hint("⏎", "save", 9),
            hint("tab", "next", 6),
            hint("←/→", "change", 5),
            hint("^J", "newline", 4),
            hint("esc", "cancel", 1),
        ];
        kit::hints_joined(width, &keys, " · ", p)
    });

    let rect = kit::dialog_area(area, body.len() as u16);
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Clear, rect);
    let block = kit::dialog_frame(&format!("edit {}", form.task_id), false, p);
    let inner = block.inner(rect);
    frame.render_widget(Paragraph::new(body).block(block), rect);
    if let Some((row, column)) = cursor
        && row < inner.height
        && value_w > 0
    {
        frame.set_cursor_position((inner.x + VALUE_X as u16 + column, inner.y + row));
    }
}
