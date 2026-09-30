//! Milestone 9.0.5 decision 22: a task's GOAL, STATUS and RESULT. The title row stays;
//! below it each section is a bold title row, then each field as a muted label in the
//! label column and its value wrapped under itself in the value column. The body
//! scrolls by `Inspection.scroll` (decision 25). Every value is agent- or daemon-
//! written text (a brief and its acceptance arrive raw): it goes through
//! `multi_line`, then `one_line` per line, before it is wrapped (decision 27).
//!
//! Pure like the rest of the panel: it lays out what it was handed.

use super::{Inspection, pad, rows, wrap_value};
use crate::inspector::{BRIEF_LINES, RUN_LABEL_WIDTH, Section, SectionField};
use crate::safe_text::{multi_line, one_line};
use crate::theme;
use crate::ui::tree_view::truncate;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// The marker a collapsed brief ends with.
pub(crate) const MORE: &str = "… (b: more)";

/// A value's lines, sanitised, each wrapped to `width` (a blank line stays one row).
fn value_lines(value: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for raw in multi_line(value).split('\n') {
        let line = one_line(raw);
        let wrapped = wrap_value(&line, width, usize::MAX);
        if wrapped.is_empty() {
            out.push(String::new());
        } else {
            out.extend(wrapped);
        }
    }
    out
}

fn field_lines(field: &SectionField, width: usize) -> Vec<Line<'static>> {
    if field.label.is_empty() {
        return value_lines(&field.value, width)
            .into_iter()
            .map(|line| Line::from(Span::styled(line, theme::muted())))
            .collect();
    }
    let label_width = RUN_LABEL_WIDTH.min(width);
    let value_width = width - label_width;
    let mut values = value_lines(&field.value, value_width.max(1));
    if field.collapse && values.len() > BRIEF_LINES {
        values.truncate(BRIEF_LINES);
        values.push(MORE.to_owned());
    }
    let label = if field.label.len() >= label_width {
        truncate(field.label, label_width.saturating_sub(1))
    } else {
        field.label.to_owned()
    };
    let mut out = Vec::new();
    if let Some(note) = field.note {
        // The label and its note on a row of their own, the value under it.
        let head = truncate(&format!("{} ({note})", field.label), width);
        out.push(Line::from(Span::styled(head, theme::muted())));
        for value in values {
            out.push(Line::from(vec![
                Span::raw(" ".repeat(label_width)),
                Span::raw(truncate(&value, value_width)),
            ]));
        }
        return out;
    }
    for (n, value) in values.into_iter().enumerate() {
        let head = if n == 0 { label.as_str() } else { "" };
        let style = if value == MORE {
            theme::muted()
        } else {
            Style::default()
        };
        out.push(Line::from(vec![
            Span::styled(pad(head, label_width), theme::muted()),
            Span::styled(truncate(&value, value_width), style),
        ]));
    }
    out
}

/// Every body row of `sections` at `width` columns: the scrollable part of the panel.
/// Its length is what `inspector::task_panel_rows` counts.
pub(crate) fn body_lines(sections: &[Section], width: usize) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut out = Vec::new();
    for section in sections {
        out.push(Line::from(Span::styled(
            truncate(section.title, width),
            bold,
        )));
        for field in &section.fields {
            out.extend(field_lines(field, width));
        }
    }
    out
}

/// The first body row drawn: `scroll`, clamped so the last body row is the last
/// panel row.
fn first_row(len: usize, room: usize, scroll: u16) -> usize {
    usize::from(scroll).min(len.saturating_sub(room))
}

/// The panel's interior: the title row, then `height − 1` body rows from `scroll`,
/// clamped to the end.
pub(super) fn lines(inspection: &Inspection, width: usize, height: usize) -> Vec<Line<'static>> {
    let mut out = vec![rows::title(inspection, width)];
    let room = height.saturating_sub(1);
    let body = body_lines(&inspection.sections, width);
    let first = first_row(body.len(), room, inspection.scroll);
    out.extend(body.into_iter().skip(first).take(room));
    out
}

/// The marks ruling D-2 puts in the borders.
pub(super) const MORE_ABOVE: &str = " ↑ PgUp ";
pub(super) const MORE_BELOW: &str = " ↓ PgDn ";

/// Ruling D-2: whether body rows lie above and below what an interior of `width` x
/// `height` draws.
pub(super) fn more(inspection: &Inspection, width: usize, height: usize) -> (bool, bool) {
    let room = height.saturating_sub(1);
    let len = body_lines(&inspection.sections, width).len();
    let first = first_row(len, room, inspection.scroll);
    (first > 0, first + room < len)
}

#[cfg(test)]
#[path = "sections_tests.rs"]
mod tests;
