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
use crate::theme;
use crate::ui::tree_view::truncate;
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// The two spaces between the name and the right-hand text.
const RIGHT_GAP: usize = 2;

/// The panel's interior, row by row, never more than `height` lines.
pub(super) fn lines(inspection: &Inspection, width: usize, height: usize) -> Vec<Line<'static>> {
    let mut out = vec![title(inspection, width)];
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
            let chunks = wrap_value(&field.value, value_width, rows);
            let taken = chunks.len().max(1);
            out.push(row(field.label, label_width, chunks.first().cloned()));
            for chunk in chunks.into_iter().skip(1) {
                out.push(row("", label_width, Some(chunk)));
            }
            left -= taken;
        } else {
            out.push(row(
                field.label,
                label_width,
                Some(truncate(&field.value, value_width)),
            ));
            left -= 1;
        }
    }
    out
}

/// The glyph and the bold name, then the muted right-hand text flush with the right
/// edge. When the name, two spaces and the right text do not fit, the right text is
/// dropped and the name truncated as milestone 4.7's title is.
pub(super) fn title(inspection: &Inspection, width: usize) -> Line<'static> {
    let glyph_width = UnicodeWidthStr::width(inspection.glyph.content.as_ref());
    let name_width = UnicodeWidthStr::width(inspection.name.as_str());
    if let Some(right) = inspection.right.as_deref() {
        let right_width = UnicodeWidthStr::width(right);
        let used = glyph_width + 1 + name_width + RIGHT_GAP + right_width;
        if used <= width {
            let mut line = title_line(inspection, inspection.name.clone());
            line.spans
                .push(Span::raw(" ".repeat(width - used + RIGHT_GAP)));
            line.spans
                .push(Span::styled(right.to_owned(), theme::muted()));
            return line;
        }
    }
    title_line(inspection, fitted_name(inspection, width))
}

/// One field row: the muted label padded to the label column (cut with `…` when it
/// would reach the value), then the value.
fn row(label: &str, label_width: usize, value: Option<String>) -> Line<'static> {
    let label = if UnicodeWidthStr::width(label) >= label_width {
        truncate(label, label_width.saturating_sub(1))
    } else {
        label.to_owned()
    };
    let mut spans = vec![Span::styled(pad(&label, label_width), theme::muted())];
    if let Some(value) = value {
        spans.push(Span::raw(value));
    }
    Line::from(spans)
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
