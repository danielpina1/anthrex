//! The hint line: `key word  key word`, keys in the accent, words muted.

use crate::safe_text::one_line;
use crate::theme::{Palette, Role, role};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// One key and what it does. Higher `priority` survives a narrow line longer; the hint
/// whose key is `esc` is never dropped before the others.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hint {
    pub key: String,
    pub word: String,
    pub priority: u8,
}

const SEPARATOR: &str = "  ";

/// The hint line for `width` columns. Whole hints are dropped, lowest `priority` first
/// and the rightmost among equals, until the line fits; `esc` goes last, and only if it
/// alone does not fit, which leaves an empty line.
pub fn hints(width: u16, hints: &[Hint], p: Palette) -> Line<'static> {
    let mut kept: Vec<(String, String, u8)> = hints
        .iter()
        .map(|h| (one_line(&h.key), one_line(&h.word), h.priority))
        .collect();
    let total = |kept: &[(String, String, u8)]| -> usize {
        let words: usize = kept.iter().map(|(k, w, _)| k.width() + 1 + w.width()).sum();
        words + SEPARATOR.len() * kept.len().saturating_sub(1)
    };
    while total(&kept) > usize::from(width) {
        let victim = kept
            .iter()
            .enumerate()
            .filter(|(_, (k, _, _))| k != "esc")
            .min_by_key(|(i, (_, _, pri))| (*pri, std::cmp::Reverse(*i)))
            .map(|(i, _)| i);
        match victim {
            Some(i) => {
                kept.remove(i);
            }
            None => return Line::default(),
        }
    }
    let key_style = role(Role::Accent, p);
    let word_style = role(Role::Muted, p);
    let mut spans = Vec::new();
    for (i, (key, word, _)) in kept.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(SEPARATOR));
        }
        spans.push(Span::styled(key, key_style));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(word, word_style));
    }
    Line::from(spans)
}
