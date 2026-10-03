//! Milestone 9.3 decisions 4–6 (KG §1.2, §1.3): the nano-like editor's keys on
//! [`TextArea`], pure (`AGENTS.md` hard rule 5). The goal, iterate and next-goal dialogs
//! send every key in their text to [`TextArea::on_editor_key`] and every bracketed
//! paste to [`TextArea::on_editor_paste`]; `on_key`, `on_key_in` and `on_paste` keep
//! their own keys for every other caller (the task edit form's brief, the action
//! forms, the profile pages). Rendering is `ui::kit::editor` and
//! `ui::kit::editor_position`.

use super::{TextArea, clean};
use crate::run_edit::TEXT_MAX_CHARS;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use unicode_segmentation::UnicodeSegmentation;

/// What [`TextArea::on_editor_key`] did with a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditorKey {
    /// The editor's key, and the text changed.
    Edited,
    /// The editor's key, and the text did not change: the cursor moved, or stayed put
    /// (Up on the first row, Backspace at the start, a paste at the cap).
    Moved,
    /// Not the editor's: Tab, Shift-Tab, Esc, Ctrl-S, Ctrl-C and the rest belong to the
    /// dialog. Nothing changed but the end of a Ctrl-K run and the cap notice.
    Unhandled,
}

impl TextArea {
    /// An editor holding `text` (cleaned, cut to the cap), the cursor at its end, bounded
    /// at the daemon's goal cap, [`proto::GOAL_MAX_CHARS`] characters.
    pub fn editor(text: &str) -> Self {
        Self::with_cap(text, proto::GOAL_MAX_CHARS)
    }

    /// The most characters this area holds.
    pub fn limit(&self) -> usize {
        if self.cap == 0 {
            TEXT_MAX_CHARS
        } else {
            self.cap
        }
    }

    /// Whether the last editor key or paste stopped at the cap (KG §1.3): the position
    /// row says so until the next key that inserts nothing past it.
    pub fn at_cap(&self) -> bool {
        self.at_cap
    }

    /// The cursor as a 1-based logical line and column, the column in grapheme
    /// clusters (a wrapped line is still one line).
    pub fn position(&self) -> (usize, usize) {
        let mut line = 1;
        let mut column = 1;
        for g in self.text.graphemes(true).take(self.cursor) {
            if g == "\n" {
                line += 1;
                column = 1;
            } else {
                column += 1;
            }
        }
        (line, column)
    }

    /// A bracketed paste: line breaks kept, control and hidden format characters
    /// dropped (as [`TextArea::on_paste`]), stopped at the cap. It ends a Ctrl-K run.
    pub fn on_editor_paste(&mut self, text: &str) {
        self.cutting = false;
        self.at_cap = self.insert_capped(text);
    }

    /// KG §1.2's keys, for an area drawn `width` columns wide (0: unwrapped) with a
    /// page of `page` drawn rows (the visible rows less one; 0 moves one row).
    pub fn on_editor_key(&mut self, key: KeyEvent, width: u16, page: usize) -> EditorKey {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        // Every key ends a run of Ctrl-Ks but a Ctrl-K, and clears the cap notice
        // unless it inserts past the cap again.
        let appending = std::mem::take(&mut self.cutting);
        self.at_cap = false;
        let before = self.text.len();
        let is =
            |c: char| matches!(key.code, KeyCode::Char(k) if ctrl && k.eq_ignore_ascii_case(&c));
        if is('j') {
            self.at_cap = self.insert_capped("\n");
        } else if is('k') {
            self.cut_line(appending);
        } else if is('u') {
            let cut = self.cut.clone();
            self.at_cap = self.insert_capped(&cut);
        } else if is('a') {
            self.edit_key(KeyEvent::new(KeyCode::Home, KeyModifiers::NONE));
        } else if is('e') {
            self.edit_key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
        } else {
            match key.code {
                KeyCode::Char(_) if ctrl || alt => return EditorKey::Unhandled,
                KeyCode::Char(c) => self.at_cap = self.insert_capped(&c.to_string()),
                KeyCode::Enter => self.at_cap = self.insert_capped("\n"),
                KeyCode::Up | KeyCode::Down => {
                    self.move_row(key.code == KeyCode::Down, width);
                }
                KeyCode::PageUp | KeyCode::PageDown => {
                    for _ in 0..page.max(1) {
                        if !self.move_row(key.code == KeyCode::PageDown, width) {
                            break;
                        }
                    }
                }
                KeyCode::Home if ctrl => self.cursor = 0,
                KeyCode::End if ctrl => self.cursor = self.len(),
                KeyCode::Backspace
                | KeyCode::Delete
                | KeyCode::Left
                | KeyCode::Right
                | KeyCode::Home
                | KeyCode::End => {
                    self.edit_key(KeyEvent::new(key.code, KeyModifiers::NONE));
                }
                _ => return EditorKey::Unhandled,
            }
        }
        if self.text.len() == before {
            EditorKey::Moved
        } else {
            EditorKey::Edited
        }
    }

    /// Inserts `text` through the area's own capped insert; whether any of it (after
    /// cleaning) was past the cap.
    fn insert_capped(&mut self, text: &str) -> bool {
        let wanted = clean(text).chars().count();
        let room = self.limit().saturating_sub(self.text.chars().count());
        self.insert(text);
        wanted > room
    }

    /// Ctrl-K: the cursor's logical line and its newline out of the text into the cut
    /// buffer, appended when `appending`. The last line, which has no newline of its
    /// own, is cut with one, as nano does; an empty last line cuts nothing.
    fn cut_line(&mut self, appending: bool) {
        self.cutting = true;
        let g = self.graphemes();
        let mut start = self.cursor;
        while start > 0 && g[start - 1] != "\n" {
            start -= 1;
        }
        let mut end = self.cursor;
        while end < g.len() && g[end] != "\n" {
            end += 1;
        }
        let (to, newline) = if end < g.len() {
            (end + 1, "")
        } else if start == end {
            return;
        } else {
            (end, "\n")
        };
        let (from, to) = (self.byte_offset(start), self.byte_offset(to));
        let piece = format!("{}{newline}", &self.text[from..to]);
        self.text.replace_range(from..to, "");
        self.cursor = start;
        if appending {
            self.cut.push_str(&piece);
        } else {
            self.cut = piece;
        }
    }

    /// The text's drawn rows at `width` (as `ui::kit::text_area` and Up and Down wrap
    /// them: one column short of the width, 0 unwrapped) and the cursor's `(row,
    /// grapheme in that row)`, for `ui::kit::editor`.
    pub(crate) fn drawn(&self, width: u16) -> (Vec<Vec<&str>>, (usize, usize)) {
        let rows = self.rows(width);
        // A cursor on a wrap boundary is drawn at the start of the next row.
        let at = rows
            .iter()
            .rposition(|(start, _, _)| *start <= self.cursor)
            .unwrap_or(0);
        let column = self.cursor.saturating_sub(rows.get(at).map_or(0, |r| r.0));
        let rows = rows.into_iter().map(|(_, row, _)| row).collect();
        (rows, (at, column))
    }

    /// A text area holding `text` as given, past `clean`: what a renderer test plants
    /// to show the renderer's own sanitiser at work.
    #[cfg(test)]
    pub(crate) fn unclean(text: &str) -> Self {
        Self {
            text: text.to_string(),
            cursor: text.graphemes(true).count(),
            ..Self::default()
        }
    }
}

#[cfg(test)]
#[path = "text_area_editor_tests.rs"]
mod tests;
