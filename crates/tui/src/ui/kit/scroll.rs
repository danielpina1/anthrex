//! The kit's scrolled views (milestone 9.0.6 decision 33, 9.0.7 decision 34): a window
//! that keeps a line in view, and a view from a top line; what is cut is marked. Moved
//! out of `screen.rs` by the final fix wave to keep it within its bound.

use super::rows::scroll_marks;
use crate::theme::{Palette, Role, role};
use ratatui::text::Line;

/// `lines` cut to `rows`, keeping line `at` in view; what is cut is marked.
pub fn window(lines: Vec<Line<'static>>, at: usize, rows: usize, p: Palette) -> Vec<Line<'static>> {
    let len = lines.len();
    let (top, room) = window_span(len, at, rows);
    if len <= rows || rows < 3 {
        return lines.into_iter().skip(top).take(room).collect();
    }
    let below = len - top - room;
    let (up, down) = scroll_marks(top, below, p.ascii);
    let muted = role(Role::Muted, p);
    let mut out: Vec<Line<'static>> = up.map(|m| Line::styled(m, muted)).into_iter().collect();
    out.extend(lines.into_iter().skip(top).take(room));
    out.extend(down.map(|m| Line::styled(m, muted)));
    out
}

/// What [`window`] shows of `len` lines: its first line and how many follow. Line `i`
/// of them is drawn at row `i - top`, one lower when `top > 0` and `rows >= 3` (the
/// `↑` mark).
pub(crate) fn window_span(len: usize, at: usize, rows: usize) -> (usize, usize) {
    if len <= rows {
        return (0, len);
    }
    if rows < 3 {
        return (at.min(len), rows);
    }
    // From the top while `at` fits above the one `↓` mark; at the bottom under the one
    // `↑` mark (final fix wave: no row kept for a `↓` with nothing below); else both.
    let end = len - (rows - 1);
    if at + 1 < rows {
        (0, rows - 1)
    } else if at >= end {
        (end, rows - 1)
    } else {
        let room = rows - 2;
        ((at + 1).saturating_sub(room), room)
    }
}

/// The last first line `len` lines can show from in `rows` rows: at the bottom only
/// the `↑` mark shows, so it leaves `rows - 1` lines (all `rows` under 3 rows). The
/// stats screen and the help scroll by their top line and stop here (milestone 9.0.7
/// decision 34).
pub(crate) fn last_top(len: usize, rows: usize) -> usize {
    if rows == 0 || len <= rows {
        0
    } else if rows < 3 {
        len - rows
    } else {
        len - (rows - 1)
    }
}

/// `lines` from line `top` in `rows` rows; a cut above or below is marked with the
/// kit's marks, and the view never scrolls past its last line ([`last_top`]).
pub(crate) fn from_top(
    lines: Vec<Line<'static>>,
    top: usize,
    rows: usize,
    p: Palette,
) -> Vec<Line<'static>> {
    let limit = last_top(lines.len(), rows);
    from_top_until(lines, top, limit, rows, p)
}

/// [`from_top`] with the top stopped at `limit` rather than at [`last_top`]: a `limit`
/// past it leaves blank rows under the last line (the help's opened group, milestone
/// 9.0.7 decision 34), never a negative count.
pub(crate) fn from_top_until(
    lines: Vec<Line<'static>>,
    top: usize,
    limit: usize,
    rows: usize,
    p: Palette,
) -> Vec<Line<'static>> {
    let len = lines.len();
    let end = last_top(len, rows);
    let top = top.min(limit.max(end)).min(len);
    let muted = role(Role::Muted, p);
    if len <= rows && top > 0 && rows >= 3 {
        // Everything fits, but the view is held below its first line (the help opened
        // on a later group): mark what it skipped (final fix wave, task 12's minor).
        let (up, _) = scroll_marks(top, 0, p.ascii);
        let mut out: Vec<Line<'static>> = up.map(|m| Line::styled(m, muted)).into_iter().collect();
        out.extend(lines.into_iter().skip(top).take(rows - 1));
        return out;
    }
    if len <= rows || rows < 3 {
        return lines.into_iter().skip(top).take(rows).collect();
    }
    let room = if top == 0 || top >= end {
        rows - 1
    } else {
        rows - 2
    };
    let below = len.saturating_sub(top + room);
    let (up, down) = scroll_marks(top, below, p.ascii);
    let mut out: Vec<Line<'static>> = up.map(|m| Line::styled(m, muted)).into_iter().collect();
    out.extend(lines.into_iter().skip(top).take(room));
    out.extend(down.map(|m| Line::styled(m, muted)));
    out
}
