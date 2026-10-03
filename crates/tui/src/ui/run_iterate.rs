//! Milestone 9.3 decision 32 (KG §2.2, §6): rendering for the iterate dialog
//! (`crate::run_iterate`, the pure model). It has the goal dialog's size and fallback
//! (decision 7): from 60 columns and 16 rows the large dialog (`ui::goal_editor`'s
//! rect), titled `iterate run <h4> · round <n>`, with the prompt line `what should
//! change or be added?`, the editor, the error row when set, the position row and the
//! footer `^S start  ^K cut  ^U paste  Esc cancel` (KG's footer without `Tab options`:
//! the dialog has no options); below that size a compact 64-column dialog with the
//! editor in `GOAL_ROWS` rows. The text area's width and rows ([`text_view`]) are the
//! ones the keys move by. Esc on a text draws the goal dialog's confirm page. The run
//! id and every drawn row pass `safe_text` (decision 33). Pure: no I/O.

use crate::run_goal::EditorView;
use crate::run_iterate::{IterateForm, PROMPT};
use crate::safe_text::one_line;
use crate::theme::{Palette, Role, dot_sep, ellipsis, role};
use crate::ui::goal_editor::{dialog_rect, footer, is_large, render_discard};
use crate::ui::kit::{self, Hint};
use crate::ui::run_goal::GOAL_ROWS;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};

/// The large dialog's footer (KG §1.2's, without `Tab options`), dropped from the right
/// when narrow.
pub const FOOTER: [(&str, &str); 4] = [
    ("^S", "start"),
    ("^K", "cut"),
    ("^U", "paste"),
    ("Esc", "cancel"),
];
/// The rows beside the text in the large dialog: the prompt, the position row and the
/// footer (the error row is counted while set).
const FIXED_ROWS: u16 = 3;

/// The compact dialog's interior width in a terminal `cols` wide.
fn compact_width(cols: u16) -> u16 {
    cols.min(kit::DIALOG_MAX).saturating_sub(4).min(kit::WRAP)
}

/// The request's text area in a terminal `cols`×`rows`, as the dialog draws it: in the
/// large dialog its interior width and the interior less the prompt, the error row
/// while set, the position row and the footer (17 rows at 80×24, 33 at 120×40), at
/// least one; in the compact one its interior width and `GOAL_ROWS`. The app hands this
/// one view to the keys and the paste; the renderer computes the same.
pub fn text_view(form: &IterateForm, cols: u16, rows: u16) -> EditorView {
    if !is_large(cols, rows) {
        return EditorView {
            width: compact_width(cols),
            rows: GOAL_ROWS,
        };
    }
    let rect = dialog_rect(Rect::new(0, 0, cols, rows));
    let fixed = FIXED_ROWS + u16::from(form.error.is_some());
    EditorView {
        width: rect.width.saturating_sub(4),
        rows: rect.height.saturating_sub(2).saturating_sub(fixed).max(1),
    }
}

/// The dialog's title, `iterate run <h4> · round <n>`, cut to `width`: the short id is
/// the cleaned run id's last four characters (decision 33).
pub fn title(form: &IterateForm, width: u16, p: Palette) -> String {
    let id: Vec<char> = one_line(&form.run_id).chars().collect();
    let h4: String = id[id.len().saturating_sub(4)..].iter().collect();
    let text = format!("iterate run {h4} · round {}", form.round);
    kit::cut(&text, usize::from(width), ellipsis(p))
}

/// The rows every layout shares: the prompt (muted), the editor's `view.rows` rows and
/// the error row while set.
fn top_rows(form: &IterateForm, view: EditorView, p: Palette) -> Vec<Line<'static>> {
    let width = usize::from(view.width);
    let mut lines = vec![Line::styled(
        kit::cut(PROMPT, width, ellipsis(p)),
        role(Role::Muted, p),
    )];
    let focused = !form.submitting;
    lines.extend(kit::editor(&form.text, view.rows, view.width, focused));
    if let Some(error) = &form.error {
        lines.push(Line::styled(
            kit::cut(&one_line(error), width, ellipsis(p)),
            role(Role::Failed, p),
        ));
    }
    lines
}

/// `starting…` while the request waits on its reply, else the position row.
fn position(form: &IterateForm, width: u16, p: Palette) -> Line<'static> {
    if form.submitting {
        return Line::styled(format!("starting{}", ellipsis(p)), role(Role::Muted, p));
    }
    kit::editor_position(&form.text, width, p)
}

/// The large dialog's interior rows for the text drawn as `view`.
pub fn body(form: &IterateForm, view: EditorView, p: Palette) -> Vec<Line<'static>> {
    let mut lines = top_rows(form, view, p);
    lines.push(position(form, view.width, p));
    lines.push(match form.submitting {
        true => footer(&[("Esc", "close")], view.width, p),
        false => footer(&FOOTER, view.width, p),
    });
    lines
}

fn hint(key: &str, word: &str, priority: u8) -> Hint {
    Hint {
        key: key.to_string(),
        word: word.to_string(),
        priority,
    }
}

/// The compact dialog's rows for `width` interior columns: the large one's, with the
/// kit's hints (`^S start · esc cancel`) for the footer.
pub fn compact_body(form: &IterateForm, width: u16, p: Palette) -> Vec<Line<'static>> {
    let view = EditorView {
        width,
        rows: GOAL_ROWS,
    };
    let mut lines = top_rows(form, view, p);
    lines.push(position(form, width, p));
    let keys = match form.submitting {
        true => vec![hint("esc", "close", 1)],
        false => vec![hint("^S", "start", 9), hint("esc", "cancel", 1)],
    };
    lines.push(kit::hints_joined(width, &keys, dot_sep(p), p));
    lines
}

/// The iterate dialog over the whole terminal `area`: large from 60×16, else compact;
/// the confirm page in the dialog's place while it is open.
pub fn render(frame: &mut Frame, form: &IterateForm, area: Rect, p: Palette) {
    let (rect, body, width) = if is_large(area.width, area.height) {
        let view = text_view(form, area.width, area.height);
        (dialog_rect(area), body(form, view, p), view.width)
    } else {
        let width = compact_width(area.width);
        let body = compact_body(form, width, p);
        (kit::dialog_area(area, body.len() as u16), body, width)
    };
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Clear, rect);
    if form.discarding {
        return render_discard(frame, rect, p);
    }
    let title = title(form, width.saturating_sub(2), p);
    frame.render_widget(
        Paragraph::new(body).block(kit::dialog_frame(&title, false, p)),
        rect,
    );
}

#[cfg(test)]
#[path = "run_iterate_tests.rs"]
pub(crate) mod tests;
