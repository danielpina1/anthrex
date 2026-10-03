//! Milestone 9.3 decisions 7, 8 and 25 (KG §1.1, §1.2, §1.4, §3.3): the large goal
//! dialog. From 60 columns and 16 rows it is `min(cols − 4, 120)` wide and `rows − 2`
//! high, centred above the status bar and one margin row: the editor, a blank row, the
//! option rows (`ui::run_goal::option_lines`), the error row when set, the position row
//! and the footer. Below that size `ui/run_goal.rs` draws the compact dialog. The text
//! area's width and rows ([`text_view`]) are the ones the keys move by, so the cursor is
//! drawn where they put it. Pure: no I/O, no clock (`AGENTS.md` hard rule 5).

use crate::run_goal::{DISCARD_ASK, EditorView, GoalForm};
use crate::safe_text::one_line;
use crate::theme::{Palette, Role, ellipsis, role};
use crate::ui::kit::{self, Hint};
use crate::ui::run_goal::{GOAL_ROWS, goal_width, option_lines, placeholder, title};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use unicode_width::UnicodeWidthStr;

/// The smallest terminal the large dialog is drawn in (KG §1.1).
pub const MIN_COLS: u16 = 60;
pub const MIN_ROWS: u16 = 16;
/// The large dialog is at most this wide.
pub const MAX_WIDTH: u16 = 120;
/// The status bar's text under the compact dialog (KG §1.1, exact).
pub const WIDEN: &str = "widen the terminal for the editor";
/// The footer's entries (KG §1.2, exact), dropped from the right when narrow.
pub const FOOTER: [(&str, &str); 5] = [
    ("^S", "start"),
    ("Tab", "options"),
    ("^K", "cut"),
    ("^U", "paste"),
    ("Esc", "cancel"),
];
/// The rows decision 7 keeps beside the text and the options: the blank row above the
/// options, the position row and the footer.
const FIXED_ROWS: u16 = 3;

/// Whether a terminal `cols`×`rows` draws the large dialog.
pub fn is_large(cols: u16, rows: u16) -> bool {
    cols >= MIN_COLS && rows >= MIN_ROWS
}

/// The large dialog in the terminal `area`: `min(cols − 4, 120)` wide, `rows − 2` high,
/// centred in the rows above the status bar (so one margin row stays under it).
pub fn dialog_rect(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).min(MAX_WIDTH);
    let height = area.height.saturating_sub(2);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height.saturating_sub(1) - height) / 2,
        width,
        height,
    }
}

/// The rows under the text that are not the fixed three: the seven options, the
/// custom model's text row while it shows, and the error row while one is set.
fn option_rows(form: &GoalForm) -> u16 {
    7 + u16::from(form.custom_shown()) + u16::from(form.error.is_some())
}

/// The goal's text area in a terminal `cols`×`rows`, as the dialog draws it: in the
/// large dialog its interior width and `interior − options − 3` rows (10 at 80×24, 26
/// at 120×40); in the compact one `goal_width` and `GOAL_ROWS`. The app hands this one
/// view to the keys and the paste; the renderers compute the same.
pub fn text_view(form: &GoalForm, cols: u16, rows: u16) -> EditorView {
    if !is_large(cols, rows) {
        return EditorView {
            width: goal_width(cols),
            rows: GOAL_ROWS,
        };
    }
    let rect = dialog_rect(Rect::new(0, 0, cols, rows));
    let interior = rect.height.saturating_sub(2);
    EditorView {
        width: rect.width.saturating_sub(4),
        rows: interior.saturating_sub(option_rows(form) + FIXED_ROWS),
    }
}

/// The footer for `width` columns: `entries` as `key word`, two spaces apart, the key
/// in the accent and the word muted, dropping entries from the right until it fits.
pub fn footer(entries: &[(&str, &str)], width: u16, p: Palette) -> Line<'static> {
    let mut kept = entries.len();
    let total = |n: usize| -> usize {
        let words: usize = entries[..n]
            .iter()
            .map(|(k, w)| k.width() + 1 + w.width())
            .sum();
        words + 2 * n.saturating_sub(1)
    };
    while kept > 0 && total(kept) > usize::from(width) {
        kept -= 1;
    }
    let mut spans = Vec::new();
    for (i, (key, word)) in entries[..kept].iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw("  "));
        }
        spans.push(Span::styled(key.to_string(), role(Role::Accent, p)));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(word.to_string(), role(Role::Muted, p)));
    }
    Line::from(spans)
}

/// The large dialog's interior rows for the goal drawn as `view`, `view.width` wide.
pub fn body(form: &GoalForm, view: EditorView, p: Palette) -> Vec<Line<'static>> {
    let focused = form.focus == crate::run_goal::GoalField::Goal && !form.submitting;
    let mut lines = kit::editor(&form.goal, view.rows, view.width, focused);
    if form.goal.is_empty()
        && let Some(first) = lines.first_mut()
    {
        *first = placeholder(usize::from(view.width), focused, p);
    }
    lines.push(Line::raw(""));
    lines.extend(option_lines(form, view.width, p));
    if let Some(error) = &form.error {
        lines.push(Line::styled(
            kit::cut(&one_line(error), usize::from(view.width), ellipsis(p)),
            role(Role::Failed, p),
        ));
    }
    if form.submitting {
        // A continued goal is not triaged (decision 22).
        let waiting = if form.continues().is_some() {
            format!("starting{}", ellipsis(p))
        } else {
            format!("starting{} triage can take minutes", ellipsis(p))
        };
        lines.push(Line::styled(waiting, role(Role::Muted, p)));
        lines.push(footer(&[("Esc", "close")], view.width, p));
    } else {
        lines.push(kit::editor_position(&form.goal, view.width, p));
        lines.push(footer(&FOOTER, view.width, p));
    }
    lines
}

/// Decision 8's confirm page in the dialog's `rect`: a destructive kit page titled
/// `discard`, the question, a blank row and `y discard · any other key back`, `y` and
/// `discard` in `Failed`.
pub fn render_discard(frame: &mut Frame, rect: Rect, p: Palette) {
    let width = rect.width.saturating_sub(4);
    let hints = [
        Hint {
            key: "y".into(),
            word: "discard".into(),
            priority: 9,
        },
        Hint {
            key: "any other key".into(),
            word: "back".into(),
            priority: 1,
        },
    ];
    let keys = kit::hints_joined(width, &hints, " · ", p);
    let body = vec![
        Line::raw(DISCARD_ASK),
        Line::raw(""),
        kit::destructive(keys, "y", "discard", p),
    ];
    frame.render_widget(
        Paragraph::new(body).block(kit::dialog_frame("discard", true, p)),
        rect,
    );
}

/// The large dialog over the whole terminal `area` (at least [`MIN_COLS`]×[`MIN_ROWS`]).
pub fn render(frame: &mut Frame, form: &GoalForm, area: Rect, p: Palette) {
    let rect = dialog_rect(area);
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Clear, rect);
    if form.discarding {
        return render_discard(frame, rect, p);
    }
    let view = text_view(form, area.width, area.height);
    let title = title(form, view.width.saturating_sub(2), p);
    frame.render_widget(
        Paragraph::new(body(form, view, p)).block(kit::dialog_frame(&title, false, p)),
        rect,
    );
}

#[cfg(test)]
#[path = "goal_editor_tests.rs"]
pub(crate) mod tests;
