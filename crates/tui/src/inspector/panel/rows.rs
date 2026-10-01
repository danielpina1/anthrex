//! A run inspection laid out one field per row (milestone 8c decision 29): the title
//! with its right-aligned muted text, then each field's label padded to
//! `RUN_LABEL_WIDTH` and its value elided to the rest of the row. The one wrapping
//! field (a scout's `question`) wraps under itself, indented by the label column,
//! onto as many rows as it needs while one row is left for each field after it.
//! Fields past the last row are dropped from the end.
//!
//! Pure like the rest of the panel: it lays out what it was handed.

use super::{Inspection, fitted_name, pad, title_line, wrap_value};
use crate::inspector::RUN_LABEL_WIDTH;
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
            out.push(row(field.label, label_width, chunks.first().cloned(), p));
            for chunk in chunks.into_iter().skip(1) {
                out.push(row("", label_width, Some(chunk), p));
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
    let label = if UnicodeWidthStr::width(label) >= label_width {
        truncate_in(label, label_width.saturating_sub(1), p.ascii)
    } else {
        label.to_owned()
    };
    let muted = theme::role(Role::Muted, p);
    let mut spans = vec![Span::styled(pad(&label, label_width), muted)];
    if let Some(value) = value {
        spans.push(Span::raw(value));
    }
    Line::from(spans)
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
