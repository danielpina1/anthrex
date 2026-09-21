//! The panel: an `Inspection` laid into a rounded box the graph's own boxes
//! would recognise — the title row, then the fields in columns below it.
//!
//! The projection half is `super`. Nothing here reads an `App`; everything it
//! draws it was handed, which is what lets it be asserted by exact rendered
//! strings the way the graph's painter is (decision 13).

use super::{Field, Inspection};
use crate::theme;
use crate::ui::tree_view::{cut, truncate};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Padding, Paragraph};
use unicode_width::UnicodeWidthStr;

/// The two spaces between a label and its value — spec §2's `model  opus`.
const LABEL_GAP: usize = 2;

/// The two spaces between one column and the next (decision 4).
const GUTTER: usize = 2;

/// Lays an `Inspection` into a rounded panel: the title row, then the fields in
/// columns below it.
pub fn render(frame: &mut Frame, inspection: &Inspection, area: Rect) {
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(theme::border())
        // One column in from the border on each side, so the fields do not sit
        // flush against it the way the graph's boxes never do.
        .padding(Padding::horizontal(1));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let lines = lines(
        inspection,
        usize::from(inner.width),
        usize::from(inner.height),
    );
    frame.render_widget(Paragraph::new(lines), inner);
}

/// The panel's interior, row by row: the title, the fields that fit in columns,
/// and the wrapping field across whatever rows are left.
///
/// Never more than `height` lines: fields past the last slot are dropped from
/// the end rather than spilling out of the panel (decision 2).
fn lines(inspection: &Inspection, width: usize, height: usize) -> Vec<Line<'static>> {
    let name = fitted_name(inspection, width);
    // Whether the title had to cut the name short is what decides below whether
    // the wrapping field is worth a row. It is known only here: the width is
    // the panel's business, and the projection is blind to it (decision 13).
    let title_says_it_all = name == inspection.name;
    let mut out = vec![title_line(inspection, name)];
    let rows = height.saturating_sub(1);
    if rows == 0 || width == 0 {
        return out;
    }

    // The wrapping field leaves the column flow: it takes the rows the other
    // fields do not, at the panel's full width, which is what "wraps across the
    // remaining rows" means (decision 5).
    //
    // Unless the title already showed it. A sub-agent's title is its label and
    // its `task` field is the same label, so on a panel wide enough for the
    // title the field would print the same words a row below — and cost the
    // column flow a row to do it. It earns its place exactly when the title
    // could not say it, which is what it is for.
    let mut flow: Vec<&Field> = Vec::new();
    let mut wrapping: Option<&Field> = None;
    for field in &inspection.fields {
        if field.wrap {
            if wrapping.is_none() && !title_says_it_all && !field.value.is_empty() {
                wrapping = Some(field);
            }
        } else {
            flow.push(field);
        }
    }
    // One row is always held back for it, so the field the panel exists for
    // survives even when the column flow would fill every row. Dropped, it
    // hands that row back.
    //
    // Held back only when the field could actually use it: `wrap_lines` gives
    // the value `width - label_width - LABEL_GAP` columns, and at zero or
    // fewer it wraps to nothing at all (its own `width == 0` guard). A row
    // reserved for a field that renders blank is a row taken from the flow
    // for nothing — the fields it would have shown are what is lost instead.
    let wrapping_fits = wrapping.is_some_and(|field| {
        let label_width = UnicodeWidthStr::width(field.label).min(width);
        width > label_width + LABEL_GAP
    });
    let flow_rows = if wrapping_fits { rows - 1 } else { rows };

    let (columns, widths) = pack(&flow, flow_rows, width);
    let shown = &flow[..flow.len().min(flow_rows.saturating_mul(columns))];
    let used_rows = shown.len().div_ceil(columns);
    for row in 0..used_rows {
        out.push(flow_line(shown, row, columns, &widths));
    }
    if let Some(field) = wrapping {
        out.extend(wrap_lines(field, width, rows - used_rows));
    }
    out
}

/// As much of the node's name as the title row can hold, beside its glyph.
/// Equal to the name itself when all of it fits, which is how the caller knows
/// nothing was lost.
fn fitted_name(inspection: &Inspection, width: usize) -> String {
    let glyph_width = UnicodeWidthStr::width(inspection.glyph.content.as_ref());
    truncate(&inspection.name, width.saturating_sub(glyph_width + 1))
}

/// The status glyph in its status colour, then the name in bold (decision 3).
fn title_line(inspection: &Inspection, name: String) -> Line<'static> {
    Line::from(vec![
        inspection.glyph.clone(),
        Span::raw(" "),
        Span::styled(name, Style::default().add_modifier(Modifier::BOLD)),
    ])
}

/// How much of its value a column must be able to say for opening another
/// column to be worth it. Below this the row is stubs and ellipses, which says
/// less than one fewer column would.
const MIN_VALUE_WIDTH: usize = 8;

/// How the fields pack into columns at this width: how many columns, and each
/// one's label and value width (decision 4).
///
/// Fields fill left to right and wrap to the next row, so column `j` holds
/// fields `j`, `j + columns`, `j + 2 * columns` and so on, and each column is
/// as wide as its own widest label and widest value. Columns are opened while
/// the width takes them, up to the number it takes to show every field: past
/// that a new column buys nothing and costs every other column the room it
/// took, which is how a window's `status` and `session` end up elided beside
/// three columns of blank rows.
///
/// A column that cannot have its full value is not a reason to close it: it is
/// elided instead (decision 5). Refusing any packing that needs an ellipsis
/// would let one long value — a session id, or the label of a deeply nested
/// parent — collapse the panel to a single column and push the fields below it
/// off the bottom, which is the opposite of what the panel is for. So a
/// packing is judged by whether every column can say `MIN_VALUE_WIDTH` of its
/// value (or all of it, when it is shorter), and the room left over is handed
/// out in fair shares, which gives the long values what the short ones do not
/// need.
fn pack(fields: &[&Field], rows: usize, width: usize) -> (usize, Vec<(usize, usize)>) {
    // One column renders even in a panel too narrow to have asked for it: the
    // label is cut to the panel and the value to whatever is left.
    let mut forced = natural_widths(&fields[..fields.len().min(rows)], 1);
    if let Some((label, value)) = forced.first_mut() {
        *label = (*label).min(width);
        *value = (*value).min(width.saturating_sub(*label + LABEL_GAP));
    }
    let mut best = (1, forced);
    if fields.is_empty() || rows == 0 {
        return best;
    }

    for columns in 1..=fields.len().div_ceil(rows) {
        let shown = &fields[..fields.len().min(rows.saturating_mul(columns))];
        let natural = natural_widths(shown, columns);
        let fixed: usize = natural
            .iter()
            .map(|(label, _)| label + LABEL_GAP)
            .sum::<usize>()
            + (columns - 1) * GUTTER;
        let floor: usize = natural
            .iter()
            .map(|(_, value)| (*value).min(MIN_VALUE_WIDTH))
            .sum();
        if fixed + floor > width {
            continue;
        }
        let wants: Vec<usize> = natural.iter().map(|(_, value)| *value).collect();
        let given = distribute(width - fixed, &wants);
        best = (
            columns,
            natural.iter().map(|(label, _)| *label).zip(given).collect(),
        );
    }
    best
}

/// Hands `budget` columns of value width out among `wants`, a fair share at a
/// time: no column takes more than it asked for, and what a short value leaves
/// behind goes to the long ones.
fn distribute(budget: usize, wants: &[usize]) -> Vec<usize> {
    let mut given = vec![0usize; wants.len()];
    let mut remaining = budget;
    loop {
        let needy: Vec<usize> = (0..wants.len())
            .filter(|index| given[*index] < wants[*index])
            .collect();
        if needy.is_empty() || remaining == 0 {
            return given;
        }
        let share = (remaining / needy.len()).max(1);
        let mut spent = 0;
        for index in needy {
            let take = (wants[index] - given[index])
                .min(share)
                .min(remaining - spent);
            given[index] += take;
            spent += take;
        }
        // The share is at least one and at least one column is needy, so this
        // only happens when the budget is exhausted — but it is what stops the
        // loop, so it is checked rather than assumed.
        if spent == 0 {
            return given;
        }
        remaining -= spent;
    }
}

/// Each column's widest label and widest value, in display columns
/// (decision 14).
fn natural_widths(fields: &[&Field], columns: usize) -> Vec<(usize, usize)> {
    let mut widths = vec![(0usize, 0usize); columns];
    for (index, field) in fields.iter().enumerate() {
        let (label, value) = &mut widths[index % columns];
        *label = (*label).max(UnicodeWidthStr::width(field.label));
        *value = (*value).max(UnicodeWidthStr::width(field.value.as_str()));
    }
    widths
}

/// One row of the column flow: each column's label, the gap, its value padded
/// to the column's width, and the gutter before the next.
fn flow_line(
    fields: &[&Field],
    row: usize,
    columns: usize,
    widths: &[(usize, usize)],
) -> Line<'static> {
    let mut spans = Vec::new();
    for (column, (label_width, value_width)) in widths.iter().copied().enumerate() {
        if column > 0 {
            spans.push(Span::raw(" ".repeat(GUTTER)));
        }
        let cell_width = label_width
            + if value_width > 0 {
                LABEL_GAP + value_width
            } else {
                0
            };
        match fields.get(row * columns + column) {
            Some(field) => {
                spans.push(Span::styled(
                    pad(&truncate(field.label, label_width), label_width),
                    theme::muted(),
                ));
                if value_width > 0 {
                    spans.push(Span::raw(" ".repeat(LABEL_GAP)));
                    spans.push(Span::raw(pad(
                        &truncate(&field.value, value_width),
                        value_width,
                    )));
                }
            }
            None => spans.push(Span::raw(" ".repeat(cell_width))),
        }
    }
    Line::from(spans)
}

/// The wrapping field across the rows the column flow left: its label on the
/// first line, its value wrapped to the panel's width under it.
fn wrap_lines(field: &Field, width: usize, rows: usize) -> Vec<Line<'static>> {
    let label_width = UnicodeWidthStr::width(field.label).min(width);
    let value_width = width.saturating_sub(label_width + LABEL_GAP);
    wrap_value(&field.value, value_width, rows)
        .into_iter()
        .enumerate()
        .map(|(index, chunk)| {
            let label = if index == 0 {
                Span::styled(
                    pad(&truncate(field.label, label_width), label_width),
                    theme::muted(),
                )
            } else {
                Span::raw(" ".repeat(label_width))
            };
            Line::from(vec![
                label,
                Span::raw(" ".repeat(LABEL_GAP)),
                Span::raw(chunk),
            ])
        })
        .collect()
}

/// Greedy word wrap to `width` display columns over at most `rows` lines. A
/// word longer than the width is broken; text past the last line is elided,
/// because the panel's height is fixed whatever the field would rather do.
fn wrap_value(text: &str, width: usize, rows: usize) -> Vec<String> {
    if width == 0 || rows == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        let joined = UnicodeWidthStr::width(current.as_str()) + 1 + UnicodeWidthStr::width(word);
        if current.is_empty() {
            current.push_str(word);
        } else if joined <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }
        while UnicodeWidthStr::width(current.as_str()) > width {
            let head = cut(&current, width);
            current = current[head.len()..].to_owned();
            lines.push(head);
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.len() > rows {
        lines.truncate(rows);
        let last = lines.len() - 1;
        lines[last] = truncate(&format!("{}…", lines[last]), width);
    }
    lines
}

/// `text` followed by enough spaces to fill `width` display columns.
fn pad(text: &str, width: usize) -> String {
    let used = UnicodeWidthStr::width(text);
    format!("{text}{}", " ".repeat(width.saturating_sub(used)))
}

#[cfg(test)]
mod tests;
