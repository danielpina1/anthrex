//! The task edit form's rows (milestone 9.0.7 decision 35): a field's label and value,
//! and the brief's four-row text area. `ui/run_edit.rs` frames and scrolls them.

use super::{BRIEF_ROWS, VALUE_X};
use crate::run_edit::{EditField, TaskEditForm, field_label};
use crate::safe_text::one_line;
use crate::theme::{Glyph, Palette, Role, glyph, role};
use crate::ui::dialog::{LABEL_WIDTH, MARKER_WIDTH};
use crate::ui::kit;
use crate::ui::tree_view::truncate_in;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

fn is_text(field: EditField) -> bool {
    matches!(
        field,
        EditField::Model | EditField::Reason | EditField::Brief
    )
}

/// `marker label` for `field`: the focused field's label is accented and bold and led
/// by the selection glyph (a cue that does not depend on colour), as the goal form's.
pub(super) fn label(form: &TaskEditForm, field: EditField, p: Palette) -> Vec<Span<'static>> {
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
pub(super) fn brief_lines(form: &TaskEditForm, value_w: u16, p: Palette) -> Vec<Line<'static>> {
    let focused = form.focus == EditField::Brief && !form.submitting;
    let mut area = kit::text_area_focus(&form.brief, BRIEF_ROWS, value_w, focused, p);
    area.resize(usize::from(BRIEF_ROWS) + 2, Line::default());
    let first = usize::from(kit::starts_with_mark(&area));
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
pub(super) fn field_line(
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
            // The column on the text as drawn, sanitised (a dropped character takes none).
            let before: String = visible.graphemes(true).take(usize::from(column)).collect();
            spans.push(Span::raw(one_line(&visible)));
            cursor = focused.then_some(one_line(&before).width() as u16);
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
