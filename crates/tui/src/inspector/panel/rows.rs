//! A run inspection laid out one field per row (milestone 8c decision 29): the title
//! with its right-aligned muted text, then each field's label padded to
//! `RUN_LABEL_WIDTH` and its value elided to the rest of the row. The one wrapping
//! field (a scout's `question`) wraps under itself, indented by the label column,
//! onto as many rows as it needs while one row is left for each field after it.
//! Fields past the last row are dropped from the end.
//!
//! Pure like the rest of the panel: it lays out what it was handed.

use super::{Inspection, fitted_name, pad, sections, title_line, wrap_value};
use crate::inspector::{FieldLayout, INSPECTOR_HEIGHT, RUN_LABEL_WIDTH};
use crate::theme::{self, Palette, Role};
use crate::ui::tree_view::truncate_in;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// The two spaces between the name and the right-hand text.
const RIGHT_GAP: usize = 2;

/// The panel's interior, row by row, never more than `height` lines.
pub(super) fn lines(
    inspection: &Inspection,
    width: usize,
    height: usize,
    p: Palette,
) -> Vec<Line<'static>> {
    let mut out = vec![title(inspection, width, p)];
    let mut left = height.saturating_sub(1);
    let label_width = RUN_LABEL_WIDTH.min(width);
    let value_width = width - label_width;
    for (index, field) in inspection.fields.iter().enumerate() {
        if left == 0 {
            break;
        }
        if field.wrap {
            // One row for each later field, and at least one for this one: past
            // that the later fields are dropped from the end like any others.
            let later = inspection.fields.len() - index - 1;
            let rows = left.saturating_sub(later).max(1);
            let chunks = wrap_value(&field.value, value_width, rows, p.ascii);
            let taken = chunks.len().max(1);
            let mut spans = marked(&chunks, &field.value, &field.marks, p).into_iter();
            out.push(row_spans(field.label, label_width, spans.next(), p));
            for chunk in spans {
                out.push(row_spans("", label_width, Some(chunk), p));
            }
            left -= taken;
        } else {
            out.push(row(
                field.label,
                label_width,
                Some(truncate_in(&field.value, value_width, p.ascii)),
                p,
            ));
            left -= 1;
        }
    }
    out
}

/// Milestone 9.0.7 decision 17: the interior rows `inspection` takes at `width` columns,
/// laid out as the panel lays it out. `Sections`: the title row or rows, every body row
/// and the footer; `Rows`: the title and each field row, the wrapping field's rows
/// counted; `Columns` (milestone 4.7's, never in the run view): its fixed six. The
/// strings are already folded for ASCII by `inspect`, and no count here depends on the
/// palette, so the plain one measures them.
pub fn panel_rows(inspection: &Inspection, width: u16) -> u16 {
    let (width, p) = (usize::from(width), Palette::PLAIN);
    let rows = match inspection.layout {
        FieldLayout::Columns => usize::from(INSPECTOR_HEIGHT - 2),
        FieldLayout::Rows => {
            let value_width = width - RUN_LABEL_WIDTH.min(width);
            let field_rows = |field: &crate::inspector::Field| {
                if field.wrap {
                    wrap_value(&field.value, value_width, usize::MAX, false)
                        .len()
                        .max(1)
                } else {
                    1
                }
            };
            1 + inspection.fields.iter().map(field_rows).sum::<usize>()
        }
        FieldLayout::Sections => {
            title_lines(inspection, width, p).len()
                + sections::body_lines(&inspection.sections, width, p).len()
                + usize::from(inspection.footer.is_some())
        }
    };
    u16::try_from(rows).unwrap_or(u16::MAX)
}

/// The glyph and the bold name, then the muted right-hand text flush with the right
/// edge. When the name, two spaces and the right text do not fit, the right text is
/// dropped and the name truncated as milestone 4.7's title is.
pub(super) fn title(inspection: &Inspection, width: usize, p: Palette) -> Line<'static> {
    if let Some(right) = inspection.right.as_deref() {
        let used = name_width(inspection) + RIGHT_GAP + UnicodeWidthStr::width(right);
        if used <= width {
            let mut line = title_line(inspection, inspection.name.clone());
            line.spans
                .push(Span::raw(" ".repeat(width - used + RIGHT_GAP)));
            line.spans
                .push(Span::styled(right.to_owned(), theme::role(Role::Muted, p)));
            return line;
        }
    }
    title_line(inspection, fitted_name(inspection, width, p.ascii))
}

/// Milestone 9.0.7 decision 12: a task's title, whose state word is never dropped:
/// flush right when the name, two spaces and it fit, else on a row of its own under the
/// title, indented two columns (cut only by the width itself).
pub(super) fn title_lines(inspection: &Inspection, width: usize, p: Palette) -> Vec<Line<'static>> {
    let first = title(inspection, width, p);
    let right = inspection.right.as_deref().unwrap_or_default();
    let used = name_width(inspection) + RIGHT_GAP + UnicodeWidthStr::width(right);
    if right.is_empty() || used <= width {
        return vec![first];
    }
    let state = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            truncate_in(right, width.saturating_sub(2), p.ascii),
            theme::role(Role::Muted, p),
        ),
    ]);
    vec![first, state]
}

/// The glyph, a space and the whole name, in display columns.
fn name_width(inspection: &Inspection) -> usize {
    UnicodeWidthStr::width(inspection.glyph.content.as_ref())
        + 1
        + UnicodeWidthStr::width(inspection.name.as_str())
}

/// One field row: the muted label padded to the label column (cut with `…` when it
/// would reach the value), then the value.
fn row(label: &str, label_width: usize, value: Option<String>, p: Palette) -> Line<'static> {
    row_spans(label, label_width, value.map(|v| vec![Span::raw(v)]), p)
}

fn row_spans(
    label: &str,
    label_width: usize,
    value: Option<Vec<Span<'static>>>,
    p: Palette,
) -> Line<'static> {
    let label = if UnicodeWidthStr::width(label) >= label_width {
        truncate_in(label, label_width.saturating_sub(1), p.ascii)
    } else {
        label.to_owned()
    };
    let muted = theme::role(Role::Muted, p);
    let mut spans = vec![Span::styled(pad(&label, label_width), muted)];
    spans.extend(value.unwrap_or_default());
    Line::from(spans)
}

/// `chunks` (`value` wrapped) as spans, the words `marks` names in their roles (deferred
/// from task 15). Each piece of a chunk is matched against `value`'s words in order; a
/// word the wrap cut is never a mark, and at the first piece that does not follow
/// (`wrap_value`'s closing ellipsis) the rest is drawn plain.
fn marked(
    chunks: &[String],
    value: &str,
    marks: &[(usize, Role)],
    p: Palette,
) -> Vec<Vec<Span<'static>>> {
    if marks.is_empty() {
        return chunks.iter().map(|c| vec![Span::raw(c.clone())]).collect();
    }
    let mut words = value.split_whitespace().enumerate();
    let (mut index, mut rest) = (0, "");
    let mut lost = false;
    let mut out = Vec::new();
    for chunk in chunks {
        let mut spans: Vec<Span<'static>> = Vec::new();
        for (n, piece) in chunk.split(' ').enumerate() {
            if n > 0 {
                spans.push(Span::raw(" "));
            }
            let mut role = None;
            if !lost && !piece.is_empty() {
                if rest.is_empty() {
                    match words.next() {
                        Some((i, word)) => (index, rest) = (i, word),
                        None => lost = true,
                    }
                }
                match rest.strip_prefix(piece) {
                    Some("") if !lost && piece.len() == value_word_len(value, index) => {
                        role = marks.iter().find(|(i, _)| *i == index).map(|(_, r)| *r);
                        rest = "";
                    }
                    Some(left) if !lost => rest = left,
                    _ => lost = true,
                }
            }
            spans.push(match role {
                Some(role) => Span::styled(piece.to_owned(), theme::role(role, p)),
                None => Span::raw(piece.to_owned()),
            });
        }
        out.push(spans);
    }
    out
}

/// The length of `value`'s `index`th word.
fn value_word_len(value: &str, index: usize) -> usize {
    value.split_whitespace().nth(index).map_or(0, str::len)
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
