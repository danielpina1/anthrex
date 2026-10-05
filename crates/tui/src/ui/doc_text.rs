//! Milestone 9.6 decision 36 and Review focus 4: a design document drawn as styled plain
//! text. Headings are bold, `R<n>` at a line's start is in the accent, fenced code is
//! dim; the diff's removed and added lines take the conversation's `Failed` and `Done`
//! roles (milestone 9.0.7 decision 3). No markdown crate.
//!
//! Every document is untrusted: whatever the daemon stored, its text passes
//! `safe_text::multi_line` and each line `one_line` here, so no control character or
//! escape sequence reaches the terminal; at most [`MAX_LINES`] lines are kept and each
//! line at most [`MAX_LINE_CHARS`] characters before wrapping, each cut marked. Lines
//! wrap at the width they are drawn in, so nothing is drawn outside its area. Pure.

use crate::safe_text::{multi_line, one_line};
use crate::theme::{Palette, Role, ellipsis, role};
use crate::ui::kit::wrap_words;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// The most source lines of one document drawn; past it one muted `[cut: <n> more
/// lines]` row.
pub const MAX_LINES: usize = 4_000;
/// The most characters of one source line drawn, before wrapping; a longer line ends
/// in `…`.
pub const MAX_LINE_CHARS: usize = 2_000;

/// `text`'s lines, cleaned and capped, and how many lines past [`MAX_LINES`] were left
/// out.
fn clean_lines(text: &str, p: Palette) -> (Vec<String>, usize) {
    let text = multi_line(text);
    let mut out = Vec::new();
    let mut cut = 0;
    for line in text.lines() {
        if out.len() == MAX_LINES {
            cut += 1;
            continue;
        }
        let line = one_line(line);
        let line = if line.chars().count() > MAX_LINE_CHARS {
            let kept: String = line.chars().take(MAX_LINE_CHARS).collect();
            format!("{kept}{}", ellipsis(p))
        } else {
            line
        };
        out.push(line);
    }
    (out, cut)
}

/// `line` broken into rows of at most `width` columns at grapheme boundaries, its
/// spaces kept (code and diff lines).
fn hard_wrap(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = Vec::new();
    let mut row = String::new();
    let mut used = 0;
    for g in line.graphemes(true) {
        let w = g.width();
        if used + w > width && used > 0 {
            rows.push(std::mem::take(&mut row));
            used = 0;
        }
        if w > width {
            // A cluster wider than the whole area is not drawn.
            continue;
        }
        row.push_str(g);
        used += w;
    }
    rows.push(row);
    rows
}

/// `line` word-wrapped at `width`, each row keeping the line's indent (at most half the
/// width), so lists and nested text stay readable.
fn wrap_keep(line: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let body = line.trim_start_matches(' ');
    if body.is_empty() {
        return vec![String::new()];
    }
    let indent = (line.len() - body.len()).min(width / 2);
    let pad = " ".repeat(indent);
    wrap_words(body, width - indent)
        .into_iter()
        .map(|row| format!("{pad}{row}"))
        .collect()
}

/// A heading: one to six `#` and a space.
fn is_heading(line: &str) -> bool {
    let hashes = line.len() - line.trim_start_matches('#').len();
    (1..=6).contains(&hashes) && line[hashes..].starts_with(' ')
}

/// `R<n>` at the line's start: its length in bytes.
fn requirement_id(line: &str) -> Option<usize> {
    let rest = line.strip_prefix('R')?;
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    let end = 1 + digits;
    let boundary = line[end..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_alphanumeric());
    (digits > 0 && boundary).then_some(end)
}

/// A fence line (```` ``` ```` or `~~~`, up to three spaces in).
fn is_fence(line: &str) -> bool {
    let t = line.trim_start_matches(' ');
    line.len() - t.len() <= 3 && (t.starts_with("```") || t.starts_with("~~~"))
}

fn cut_note(n: usize, p: Palette) -> Line<'static> {
    Line::styled(format!("[cut: {n} more lines]"), role(Role::Muted, p))
}

/// A document's rows at `width` columns: headings bold, `R<n>` at a line's start in the
/// accent, fenced code (its fences too) dim, everything else plain.
pub fn doc_lines(text: &str, width: u16, p: Palette) -> Vec<Line<'static>> {
    let width = usize::from(width).max(1);
    let (lines, cut) = clean_lines(text, p);
    let dim = role(Role::Muted, p);
    let bold = Style::default().add_modifier(Modifier::BOLD);
    let accent = role(Role::Accent, p);
    let mut out = Vec::new();
    let mut code = false;
    for line in lines {
        if is_fence(&line) {
            code = !code;
            out.extend(
                hard_wrap(&line, width)
                    .into_iter()
                    .map(|r| Line::styled(r, dim)),
            );
            continue;
        }
        if code {
            out.extend(
                hard_wrap(&line, width)
                    .into_iter()
                    .map(|r| Line::styled(r, dim)),
            );
            continue;
        }
        if is_heading(&line) {
            out.extend(
                wrap_keep(&line, width)
                    .into_iter()
                    .map(|r| Line::styled(r, bold)),
            );
            continue;
        }
        let rows = wrap_keep(&line, width);
        match requirement_id(&line) {
            Some(end) => {
                for (i, row) in rows.into_iter().enumerate() {
                    if i == 0 && row.len() >= end && row.is_char_boundary(end) {
                        let (id, rest) = row.split_at(end);
                        out.push(Line::from(vec![
                            Span::styled(id.to_owned(), accent),
                            Span::raw(rest.to_owned()),
                        ]));
                    } else {
                        out.push(Line::from(row));
                    }
                }
            }
            None => out.extend(rows.into_iter().map(Line::from)),
        }
    }
    if cut > 0 {
        out.push(cut_note(cut, p));
    }
    out
}

/// A line diff's rows at `width` columns (`changes::line_diff`'s text): `@@` headers
/// dim, removed lines in `Failed`, added lines in `Done`, context plain.
pub fn diff_lines(diff: &str, width: u16, p: Palette) -> Vec<Line<'static>> {
    let width = usize::from(width).max(1);
    let (lines, cut) = clean_lines(diff, p);
    let mut out = Vec::new();
    for line in lines {
        let style = if line.starts_with("@@") {
            role(Role::Muted, p)
        } else if line.starts_with('-') {
            role(Role::Failed, p)
        } else if line.starts_with('+') {
            role(Role::Done, p)
        } else {
            Style::default()
        };
        out.extend(
            hard_wrap(&line, width)
                .into_iter()
                .map(|r| Line::styled(r, style)),
        );
    }
    if cut > 0 {
        out.push(cut_note(cut, p));
    }
    out
}

/// Any text the daemon or an agent wrote, word-wrapped at `width` in `style`, every
/// line cleaned: the panel's and the message line's rows.
pub fn plain_lines(text: &str, width: u16, style: Style) -> Vec<Line<'static>> {
    let width = usize::from(width).max(1);
    multi_line(text)
        .lines()
        .flat_map(|l| wrap_keep(&one_line(l), width))
        .map(|r| Line::styled(r, style))
        .collect()
}

#[cfg(test)]
#[path = "doc_text_tests.rs"]
mod tests;
