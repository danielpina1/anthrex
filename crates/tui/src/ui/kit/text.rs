//! Wrapping and the multi-line text area's rendering.

use super::rows::scroll_marks;
use crate::text_area::TextArea;
use crate::theme::{Palette, Role, role};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// `text` cut to `max` columns, ending in `ellipsis` when it was cut.
pub(super) fn cut(text: &str, max: usize, ellipsis: &str) -> String {
    if text.width() <= max {
        return text.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for g in text.graphemes(true) {
        let w = g.width();
        if used + w + ellipsis.width() > max {
            break;
        }
        out.push_str(g);
        used += w;
    }
    if max >= ellipsis.width() {
        out.push_str(ellipsis);
    }
    out
}

/// Word-wraps one line of `text` at `width` columns; a word longer than the width is
/// broken. Always returns at least one (possibly empty) line.
pub(super) fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut used = 0;
    for word in text.split_whitespace() {
        let mut word = word;
        loop {
            let w = word.width();
            let gap = usize::from(used > 0);
            if used + gap + w <= width {
                if gap == 1 {
                    line.push(' ');
                }
                line.push_str(word);
                used += gap + w;
                break;
            }
            if used > 0 && w <= width {
                lines.push(std::mem::take(&mut line));
                used = 0;
                continue;
            }
            // A word wider than the whole column: break it.
            if used > 0 {
                lines.push(std::mem::take(&mut line));
                used = 0;
            }
            let mut taken = 0;
            let mut head = String::new();
            for g in word.graphemes(true) {
                if taken + g.width() > width && taken > 0 {
                    break;
                }
                taken += g.width();
                head.push_str(g);
            }
            word = &word[head.len()..];
            lines.push(head);
            if word.is_empty() {
                break;
            }
        }
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// The text area as exactly `rows` text lines (padded when the text is shorter), with a
/// leading `↑ n more` and/or a trailing `↓ n more` line when text is out of view: the
/// marks are not counted in `rows`, so a form reserves `rows + 2`. Lines wrap at one
/// column short of `width` (so the cursor always has a cell) and the view scrolls to
/// keep the cursor's row visible. The cursor is drawn reversed.
pub fn text_area(area: &TextArea, rows: u16, width: u16, p: Palette) -> Vec<Line<'static>> {
    let rows = usize::from(rows);
    let wrap_at = usize::from(width).saturating_sub(1).max(1);
    // Visual rows of graphemes, and where the cursor is.
    let mut visual: Vec<Vec<&str>> = Vec::new();
    let mut cursor_at = (0, 0);
    let mut index = 0;
    for logical in area.text().split('\n') {
        let mut row: Vec<&str> = Vec::new();
        let mut used = 0;
        for g in logical.graphemes(true) {
            let w = g.width();
            if used + w > wrap_at && !row.is_empty() {
                visual.push(std::mem::take(&mut row));
                used = 0;
            }
            if index == area.cursor() {
                cursor_at = (visual.len(), row.len());
            }
            row.push(g);
            used += w;
            index += 1;
        }
        if index == area.cursor() {
            cursor_at = (visual.len(), row.len());
        }
        visual.push(row);
        index += 1; // the newline
    }
    let top = (cursor_at.0 + 1).saturating_sub(rows);
    let above = top;
    let below = visual.len().saturating_sub(top + rows);
    let (up, down) = scroll_marks(above, below, p.ascii);
    let muted = role(Role::Muted, p);
    let mut out = Vec::new();
    if let Some(mark) = up {
        out.push(Line::from(Span::styled(mark, muted)));
    }
    for r in top..top + rows {
        let Some(row) = visual.get(r) else {
            out.push(Line::default());
            continue;
        };
        if r != cursor_at.0 {
            out.push(Line::from(row.concat()));
            continue;
        }
        let col = cursor_at.1;
        let before = row[..col].concat();
        let (under, after) = match row.get(col) {
            Some(g) => ((*g).to_string(), row[col + 1..].concat()),
            None => (" ".to_string(), String::new()),
        };
        out.push(Line::from(vec![
            Span::raw(before),
            Span::styled(
                under,
                ratatui::style::Style::default().add_modifier(Modifier::REVERSED),
            ),
            Span::raw(after),
        ]));
    }
    if let Some(mark) = down {
        out.push(Line::from(Span::styled(mark, muted)));
    }
    out
}
