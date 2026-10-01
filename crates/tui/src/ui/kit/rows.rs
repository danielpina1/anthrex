//! Labelled rows, run names and scroll marks.

use super::text::{cut, wrap_words};
use crate::safe_text::one_line;
use crate::theme::{Palette, Role, role};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// `label  value` rows: labels muted and padded to the longest label plus two spaces;
/// a value wraps at `width` under itself. Every string is sanitised.
pub fn labelled_rows(rows: &[(String, String)], width: u16, p: Palette) -> Vec<Line<'static>> {
    let clean: Vec<(String, String)> = rows
        .iter()
        .map(|(l, v)| (one_line(l), one_line(v)))
        .collect();
    let label_w = clean.iter().map(|(l, _)| l.width()).max().unwrap_or(0) + 2;
    let value_w = usize::from(width).saturating_sub(label_w).max(1);
    let label_style = role(Role::Muted, p);
    let mut out = Vec::new();
    for (label, value) in clean {
        let pad = " ".repeat(label_w - label.width());
        let mut parts = wrap_words(&value, value_w).into_iter();
        let first = parts.next().unwrap_or_default();
        out.push(Line::from(vec![
            Span::styled(format!("{label}{pad}"), label_style),
            Span::raw(first),
        ]));
        for part in parts {
            out.push(Line::from(vec![
                Span::raw(" ".repeat(label_w)),
                Span::raw(part),
            ]));
        }
    }
    out
}

/// `<goal> · <short>`: the goal cut with `…` so the whole name fits `width`, then the
/// run id's last four characters.
pub fn run_name(goal: &str, id: &str, width: u16) -> String {
    let id = one_line(id);
    let chars: Vec<char> = id.chars().collect();
    let short: String = chars[chars.len().saturating_sub(4)..].iter().collect();
    let suffix = format!(" · {short}");
    let room = usize::from(width).saturating_sub(suffix.width());
    format!("{}{suffix}", cut(one_line(goal).trim(), room))
}

/// `↑ n more` and `↓ n more` (`^`, `v` in ASCII); `None` for a side with nothing.
pub fn scroll_marks(above: usize, below: usize, ascii: bool) -> (Option<String>, Option<String>) {
    let (up, down) = if ascii { ("^", "v") } else { ("↑", "↓") };
    let mark = |glyph: &str, n: usize| (n > 0).then(|| format!("{glyph} {n} more"));
    (mark(up, above), mark(down, below))
}
