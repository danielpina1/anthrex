//! Milestone 9.3 decisions 4, 7 and 33 (KG §1.2, §1.3): the editor's rows and its
//! position row. The rows wrap as the editor's Up, Down and PgUp/PgDn move through them
//! (`TextArea::drawn`), so the cursor is drawn where the keys put it. Every drawn row
//! passes `safe_text::one_line` here, whatever `TextArea` already cleaned (decision
//! 33). Pure: no I/O, no clock (`AGENTS.md` hard rule 5).

use super::cut;
use crate::safe_text::one_line;
use crate::text_area::TextArea;
use crate::theme::{Palette, Role, fold, role};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// The editor as exactly `rows` lines (padded when the text is shorter; no scroll
/// marks, the position row says where the cursor is), wrapped one column short of
/// `width`, drawn from the editor's viewport top (decision 4 as amended) as
/// `TextArea::view` fits it to these rows, so the cursor's row is always in view: a
/// resize, the custom-model row or a restored draft cannot hide it. The cursor is drawn reversed
/// when `focused`, in every palette (the kit's text areas draw it so in ASCII too).
pub fn editor(area: &TextArea, rows: u16, width: u16, focused: bool) -> Vec<Line<'static>> {
    let rows = usize::from(rows);
    let (visual, (cursor_row, cursor_col), top) = area.view(width, rows);
    // The per-row sanitiser (decision 33): every piece of a row drawn goes through it.
    let safe = |graphemes: &[&str]| one_line(&graphemes.concat());
    let mut out = Vec::with_capacity(rows);
    for r in top..top + rows {
        let Some(row) = visual.get(r) else {
            out.push(Line::default());
            continue;
        };
        if r != cursor_row || !focused {
            out.push(Line::from(safe(row)));
            continue;
        }
        let col = cursor_col.min(row.len());
        let under = match row.get(col) {
            Some(g) => safe(&[g]),
            None => String::new(),
        };
        let after = row.get(col + 1..).map(safe).unwrap_or_default();
        out.push(Line::from(vec![
            Span::raw(safe(&row[..col])),
            Span::styled(
                if under.is_empty() {
                    " ".to_string()
                } else {
                    under
                },
                Style::default().add_modifier(Modifier::REVERSED),
            ),
            Span::raw(after),
        ]));
    }
    out
}

/// `n` with a comma every three digits.
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// The position row (decision 7, exact): `ln <line>, col <col> · <count> / <cap>`,
/// muted, the count in `Attention` once it is above 90% of the cap; while the last
/// key or paste stopped at the cap, `goal is at its <cap>-character limit` in
/// `Attention` instead. Narrower than the whole row, the line and column go first.
pub fn editor_position(area: &TextArea, width: u16, p: Palette) -> Line<'static> {
    let width = usize::from(width);
    let limit = area.limit();
    let attention = role(Role::Attention, p);
    if area.at_cap() {
        let text = format!("goal is at its {}-character limit", grouped(limit));
        return Line::from(Span::styled(
            cut(&text, width, &fold("…", p.ascii)),
            attention,
        ));
    }
    let muted = role(Role::Muted, p);
    let count = area.text().chars().count();
    let count_style = if count > limit * 9 / 10 {
        attention
    } else {
        muted
    };
    let (line, col) = area.position();
    let place = fold(&format!("ln {line}, col {col} · "), p.ascii);
    let count = grouped(count);
    let of = format!(" / {}", grouped(limit));
    let mut spans = Vec::with_capacity(3);
    if place.width() + count.width() + of.width() <= width {
        spans.push(Span::styled(place, muted));
    }
    spans.push(Span::styled(count, count_style));
    spans.push(Span::styled(of, muted));
    Line::from(spans)
}

#[cfg(test)]
#[path = "editor_tests.rs"]
mod tests;
