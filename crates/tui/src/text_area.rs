//! Milestone 9.0.6 decision 22: a multi-line text field, pure (`AGENTS.md` hard rule 5).
//! The cursor is a grapheme index, as `dialog::TextInput`'s is. Characters are typed with
//! `on_key`, pasted with `on_paste`; Ctrl-J inserts a newline and Enter is left to the
//! form. Every way in drops control and invisible format characters (keeping `\n`), and
//! the text never passes [`TEXT_MAX_CHARS`]. Rendering is `ui::kit::text_area`.

use crate::run_edit::TEXT_MAX_CHARS;
use crate::safe_text::is_hidden_format;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TextArea {
    text: String,
    cursor: usize,
}

/// Newlines kept (`\r\n` and `\r` become one), a tab a space, every other control and
/// hidden format character dropped.
fn clean(text: &str) -> String {
    let text = text.replace("\r\n", "\n").replace('\r', "\n");
    text.chars()
        .filter_map(|c| match c {
            '\n' => Some('\n'),
            '\t' => Some(' '),
            c if c.is_control() || is_hidden_format(c) => None,
            '\u{2028}' | '\u{2029}' => Some('\n'),
            c => Some(c),
        })
        .collect()
}

impl TextArea {
    pub fn new() -> Self {
        Self::default()
    }

    /// A text area holding `text` (cleaned, cut to the bound), the cursor at the end.
    pub fn from_text(text: &str) -> Self {
        let mut area = Self::new();
        area.on_paste(text);
        area
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// The cursor, as a grapheme index into `text()`.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    pub fn lines(&self) -> std::str::Split<'_, char> {
        self.text.split('\n')
    }

    fn graphemes(&self) -> Vec<&str> {
        self.text.graphemes(true).collect()
    }

    fn len(&self) -> usize {
        self.text.graphemes(true).count()
    }

    fn byte_offset(&self, index: usize) -> usize {
        self.text
            .grapheme_indices(true)
            .nth(index)
            .map(|(offset, _)| offset)
            .unwrap_or(self.text.len())
    }

    fn insert(&mut self, text: &str) {
        let room = TEXT_MAX_CHARS.saturating_sub(self.text.chars().count());
        let cut: String = clean(text).chars().take(room).collect();
        if cut.is_empty() {
            return;
        }
        let offset = self.byte_offset(self.cursor);
        self.text.insert_str(offset, &cut);
        let end = offset + cut.len();
        // Past the whole cluster the insertion ended in, as `TextInput::insert` does.
        self.cursor = self
            .text
            .grapheme_indices(true)
            .position(|(at, _)| at >= end)
            .unwrap_or_else(|| self.len());
    }

    /// Pasted text: newlines kept, other control characters dropped, bounded.
    pub fn on_paste(&mut self, text: &str) {
        self.insert(text);
    }

    /// Whether the key was the text area's; Enter, Tab, Esc and the rest are not.
    pub fn on_key(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        match key.code {
            KeyCode::Char(c) if ctrl && c.eq_ignore_ascii_case(&'j') => self.insert("\n"),
            KeyCode::Char(_) if ctrl || alt => return false,
            KeyCode::Char(c) => self.insert(&c.to_string()),
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let (start, end) = (
                        self.byte_offset(self.cursor - 1),
                        self.byte_offset(self.cursor),
                    );
                    self.text.replace_range(start..end, "");
                    self.cursor -= 1;
                }
            }
            KeyCode::Delete => {
                if self.cursor < self.len() {
                    let (start, end) = (
                        self.byte_offset(self.cursor),
                        self.byte_offset(self.cursor + 1),
                    );
                    self.text.replace_range(start..end, "");
                }
            }
            KeyCode::Left => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right => self.cursor = (self.cursor + 1).min(self.len()),
            KeyCode::Home => {
                let g = self.graphemes();
                let mut at = self.cursor;
                while at > 0 && g[at - 1] != "\n" {
                    at -= 1;
                }
                self.cursor = at;
            }
            KeyCode::End => {
                let g = self.graphemes();
                let mut at = self.cursor;
                while at < g.len() && g[at] != "\n" {
                    at += 1;
                }
                self.cursor = at;
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
#[path = "text_area_tests.rs"]
mod tests;
