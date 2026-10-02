//! `TextArea`'s Up and Down (milestone 9.0.7 decision 35): the text's rows as
//! `ui::kit::text_area` draws them, and the cursor moved a row at the same column.

use super::TextArea;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

impl TextArea {
    /// The drawn rows as `(first grapheme index, graphemes, last row of its line)`,
    /// wrapped as `ui::kit::text_area_focus` wraps them: one column short of `width`.
    pub(super) fn rows(&self, width: u16) -> Vec<(usize, Vec<&str>, bool)> {
        let wrap_at = match width {
            0 => usize::MAX,
            w => usize::from(w).saturating_sub(1).max(1),
        };
        let mut rows = Vec::new();
        let mut index = 0;
        for logical in self.text.split('\n') {
            let mut start = index;
            let mut row: Vec<&str> = Vec::new();
            let mut used: usize = 0;
            for g in logical.graphemes(true) {
                let w = g.width();
                if used.saturating_add(w) > wrap_at && !row.is_empty() {
                    rows.push((start, std::mem::take(&mut row), false));
                    start = index;
                    used = 0;
                }
                row.push(g);
                used += w;
                index += 1;
            }
            rows.push((start, row, true));
            index += 1; // the newline
        }
        rows
    }

    pub(super) fn move_row(&mut self, down: bool, width: u16) -> bool {
        let rows = self.rows(width);
        // The cursor's row: the last whose start is at or before it (a cursor on a
        // wrap boundary is drawn at the start of the next row, as the kit draws it).
        let Some(at) = rows.iter().rposition(|(start, _, _)| *start <= self.cursor) else {
            return false;
        };
        let target = match (down, at) {
            (false, 0) => return false,
            (false, at) => at - 1,
            (true, at) if at + 1 >= rows.len() => return false,
            (true, at) => at + 1,
        };
        let (start, row, _) = &rows[at];
        let column: usize = row[..self.cursor - start].iter().map(|g| g.width()).sum();
        let (start, row, last) = &rows[target];
        // A wrapped row's end is the next row's start: stop one grapheme short of it.
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
        true
    }
}
