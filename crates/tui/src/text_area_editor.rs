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
use unicode_width::UnicodeWidthStr;

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

    /// A bracketed paste into an area drawn `width` columns wide with `rows` visible
    /// rows: line breaks kept, control and hidden format characters dropped (as
    /// [`TextArea::on_paste`]), stopped at the cap. It ends a Ctrl-K run, and scrolls
    /// the view as little as the cursor needs.
    pub fn on_editor_paste(&mut self, text: &str, width: u16, rows: u16) {
        let rows = usize::from(rows.max(1));
        self.fit(width, rows);
        self.cutting = false;
        self.at_cap = self.insert_capped(text);
        self.fit(width, rows);
    }

    /// KG §1.2's keys, for an area drawn `width` columns wide (0: unwrapped) with `rows`
    /// visible rows (decision 4 as amended). PgUp and PgDn move a page, the visible
    /// rows less one (at least one), keeping the goal column, and shift the view by
    /// the rows they moved, as nano does; any other move scrolls the view as little as
    /// keeps the cursor in it, so crossing an edge scrolls one row.
    pub fn on_editor_key(&mut self, key: KeyEvent, width: u16, rows: u16) -> EditorKey {
        let rows = usize::from(rows.max(1));
        // From the view the last frame drew (a resize or a new row count may have moved
        // it since the last key).
        self.fit(width, rows);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        // Every key ends a run of Ctrl-Ks but a Ctrl-K, and clears the cap notice
        // unless it inserts past the cap again.
        let appending = std::mem::take(&mut self.cutting);
        self.at_cap = false;
        let before = self.text.len();
        // The editor's Ctrl chords; with Alt too they are not the editor's (review m4).
        let is = |c: char| matches!(key.code, KeyCode::Char(k) if ctrl && !alt && k.eq_ignore_ascii_case(&c));
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
                    let down = key.code == KeyCode::PageDown;
                    let moved = self.move_rows(down, (rows - 1).max(1), width);
                    self.top = if down {
                        self.top + moved
                    } else {
                        self.top.saturating_sub(moved)
                    };
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
        self.fit(width, rows);
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

    /// PgUp/PgDn (review m1): the cursor `n` drawn rows up or down at its display
    /// column, clamped to the target row only (a short row on the way does not lose
    /// the column), stopping at the first or last row. How many rows it moved.
    fn move_rows(&mut self, down: bool, n: usize, width: u16) -> usize {
        let rows = self.rows(width);
        let Some(at) = rows.iter().rposition(|(start, _, _)| *start <= self.cursor) else {
            return 0;
        };
        let target = if down {
            (at + n).min(rows.len() - 1)
        } else {
            at.saturating_sub(n)
        };
        let (start, row, _) = &rows[at];
        let column: usize = row[..self.cursor - start].iter().map(|g| g.width()).sum();
        let (start, row, last) = &rows[target];
        // As `move_row` places it: a wrapped row's end is the next row's start.
        let room = if *last {
            row.len()
        } else {
            row.len().saturating_sub(1)
        };
        let mut taken = 0;
        let mut used = 0;
        while taken < room && used + row[taken].width() <= column {
            used += row[taken].width();
            taken += 1;
        }
        self.cursor = start + taken;
        target.abs_diff(at)
    }

    /// Moves `top` as little as keeps the view drawn at `width` and `rows` honest
    /// ([`fit_top`]).
    fn fit(&mut self, width: u16, rows: usize) {
        let top = self.view(width, rows).2;
        self.top = top;
    }

    /// The text's drawn rows at `width` (as `ui::kit::text_area` and Up and Down wrap
    /// them: one column short of the width, 0 unwrapped), the cursor's `(row, grapheme
    /// in that row)`, and the first row a view of `rows` rows shows: the stored top
    /// through [`fit_top`], so `ui::kit::editor` never hides the cursor whatever
    /// changed since the last key (a resize, the custom-model row, a restored draft).
    pub(crate) fn view(&self, width: u16, rows: usize) -> (Vec<Vec<&str>>, (usize, usize), usize) {
        let drawn = self.rows(width);
        // A cursor on a wrap boundary is drawn at the start of the next row.
        let at = drawn
            .iter()
            .rposition(|(start, _, _)| *start <= self.cursor)
            .unwrap_or(0);
        let column = self.cursor.saturating_sub(drawn.get(at).map_or(0, |r| r.0));
        let top = fit_top(self.top, at, drawn.len(), rows);
        let drawn = drawn.into_iter().map(|(_, row, _)| row).collect();
        (drawn, (at, column), top)
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

/// The first row of a `rows`-row view of `total` drawn rows whose cursor is on
/// `cursor_row`, from the stored `top`: clamped to `[cursor_row + 1 − rows,
/// cursor_row]` (the cursor in view, moved as little as needed: nano's scroll by one
/// at the edges, and the nearest edge row after a jump), and never so far down that rows
/// sit empty below the text while text is hidden above (a larger area shows more).
fn fit_top(top: usize, cursor_row: usize, total: usize, rows: usize) -> usize {
    let rows = rows.max(1);
    top.clamp((cursor_row + 1).saturating_sub(rows), cursor_row)
        .min(total.saturating_sub(rows))
}

#[cfg(test)]
#[path = "text_area_editor_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "text_area_editor_view_tests.rs"]
mod view_tests;
