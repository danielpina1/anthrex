//! A task's sections (milestone 9.0.7 decision 12, after 9.0.5 decision 22). The title
//! row (or rows, when the state word moves under it) stays on top and the footer on the
//! last row; between them each section is a bold title row, then each field as a muted
//! label in the label column and its value wrapped under itself in the value column,
//! its client-written marks coloured (decision 13). The body scrolls by
//! `Inspection.scroll` (9.0.5 decision 25). Every value is agent- or daemon-written text
//! (a brief and its acceptance arrive raw): it goes through `multi_line`, then
//! `one_line` per line, before it is wrapped (9.0.5 decision 27).
//!
//! Pure like the rest of the panel: it lays out what it was handed.

use super::{Inspection, marks::marked, pad, rows, wrap_value};
use crate::inspector::{BRIEF_LINES, RUN_LABEL_WIDTH, Section, SectionField};
use crate::safe_text::{multi_line, one_line};
use crate::theme::{self, Palette, Role};
use crate::ui::tree_view::truncate_in;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

/// The marker a collapsed brief ends with.
pub(crate) const MORE: &str = "… (b: more)";

/// A value's lines, sanitised, each wrapped to `width` (a blank line stays one row),
/// each marked when it begins one of the value's own lines.
fn value_lines(value: &str, width: usize, ascii: bool) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    for raw in multi_line(value).split('\n') {
        let line = one_line(raw);
        let wrapped = wrap_value(&line, width, usize::MAX, ascii);
        if wrapped.is_empty() {
            out.push((String::new(), true));
        } else {
            out.extend(wrapped.into_iter().enumerate().map(|(n, l)| (l, n == 0)));
        }
    }
    out
}

fn field_lines(field: &SectionField, width: usize, p: Palette) -> Vec<Line<'static>> {
    let muted = theme::role(Role::Muted, p);
    let truncate = |text: &str, width| truncate_in(text, width, p.ascii);
    if field.label.is_empty() {
        return value_lines(&field.value, width, p.ascii)
            .into_iter()
            .map(|(line, _)| Line::from(Span::styled(line, muted)))
            .collect();
    }
    let label_width = RUN_LABEL_WIDTH.min(width);
    let value_width = width - label_width;
    let mut values = value_lines(&field.value, value_width.max(1), p.ascii);
    let more = theme::fold(MORE, p.ascii);
    if field.collapse && values.len() > BRIEF_LINES {
        values.truncate(BRIEF_LINES);
        values.push((more.clone(), false));
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
        out.push(Line::from(Span::styled(head, muted)));
        for (value, _) in values {
            out.push(Line::from(vec![
                Span::raw(" ".repeat(label_width)),
                Span::raw(truncate(&value, value_width)),
            ]));
        }
        return out;
    }
    for (n, (value, starts)) in values.into_iter().enumerate() {
        let head = if n == 0 { label.as_str() } else { "" };
        let mut spans = vec![Span::styled(pad(head, label_width), muted)];
        if value == more {
            spans.push(Span::styled(value, muted));
        } else {
            spans.extend(marked(
                truncate(&value, value_width),
                field.marks,
                starts,
                p,
            ));
        }
        out.push(Line::from(spans));
    }
    out
}

/// Every body row of `sections` at `width` columns: the scrollable part of the panel.
/// Its length is what `inspector::task_panel_rows` counts.
pub(crate) fn body_lines(sections: &[Section], width: usize, p: Palette) -> Vec<Line<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let mut out = Vec::new();
    for section in sections {
        out.push(Line::from(Span::styled(
            truncate_in(section.title, width, p.ascii),
            bold,
        )));
        for field in &section.fields {
            out.extend(field_lines(field, width, p));
        }
    }
    out
}

/// The first body row drawn: `scroll`, clamped so the last body row is the last
/// panel row.
fn first_row(len: usize, room: usize, scroll: u16) -> usize {
    usize::from(scroll).min(len.saturating_sub(room))
}

/// The title rows and the footer an interior of `width` x `height` draws, and the body
/// rows left between them: the title row first, then the state word's row, then the
/// footer, each only while a row is left for it (decision 12).
fn chrome(
    inspection: &Inspection,
    width: usize,
    height: usize,
    p: Palette,
) -> (Vec<Line<'static>>, Option<Line<'static>>, usize) {
    let mut title = rows::title_lines(inspection, width, p);
    title.truncate(height);
    let footer = inspection
        .footer
        .as_deref()
        .filter(|_| height > title.len())
        .map(|text| {
            Line::from(Span::styled(
                truncate_in(text, width, p.ascii),
                theme::role(Role::Muted, p),
            ))
        });
    let room = height - title.len() - usize::from(footer.is_some());
    (title, footer, room)
}

/// The body rows an interior of `width` x `height` shows: what the title and the
/// footer leave (`inspector::task_panel_room`).
pub(crate) fn room(inspection: &Inspection, width: usize, height: usize, p: Palette) -> usize {
    chrome(inspection, width, height, p).2
}

/// The panel's interior: the title row or rows, the body rows from `scroll` (clamped to
/// the end), and the footer pinned to the last row, outside the scroll.
pub(super) fn lines(
    inspection: &Inspection,
    width: usize,
    height: usize,
    p: Palette,
) -> Vec<Line<'static>> {
    let (mut out, footer, room) = chrome(inspection, width, height, p);
    let body = body_lines(&inspection.sections, width, p);
    let first = first_row(body.len(), room, inspection.scroll);
    let shown = body.len().saturating_sub(first).min(room);
    out.extend(body.into_iter().skip(first).take(room));
    // The footer keeps the last row even when the body is short.
    out.extend((shown..room).map(|_| Line::default()));
    out.extend(footer);
    out
}

/// Ruling D-2: whether body rows lie above and below what an interior of `width` x
/// `height` draws.
pub(super) fn more(
    inspection: &Inspection,
    width: usize,
    height: usize,
    p: Palette,
) -> (bool, bool) {
    let room = room(inspection, width, height, p);
    let len = body_lines(&inspection.sections, width, p).len();
    let first = first_row(len, room, inspection.scroll);
    (first > 0, first + room < len)
}

#[cfg(test)]
#[path = "sections_tests.rs"]
mod tests;
