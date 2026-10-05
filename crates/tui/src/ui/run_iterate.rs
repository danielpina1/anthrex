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
//!
//! Milestone 9.6: a design run's dialog has the `design` row under the text (the goal
//! dialog's grammar: the selection mark while focused, a lower-case label, `‹ value ›`),
//! then the run's own round mode muted, `this round: <mode>`; the footer gains
//! `Tab options` and the compact hints `tab next`. Without the flow nothing changes.

use crate::run_goal::EditorView;
use crate::run_iterate::{IterateForm, PROMPT};
use crate::safe_text::one_line;
use crate::theme::{Glyph, Palette, Role, dot_sep, ellipsis, glyph, role};
use crate::ui::goal_editor::{dialog_rect, footer, is_large, render_discard};
use crate::ui::kit::{self, Hint};
use crate::ui::run_goal::GOAL_ROWS;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

/// The large dialog's footer (KG §1.2's, without `Tab options`), dropped from the right
/// when narrow.
pub const FOOTER: [(&str, &str); 4] = [
    ("^S", "start"),
    ("^K", "cut"),
    ("^U", "paste"),
    ("Esc", "cancel"),
];
/// [`FOOTER`] with the design row's `Tab options` (KG §1.2's own entry).
pub const DESIGN_FOOTER: [(&str, &str); 5] = [
    ("^S", "start"),
    ("Tab", "options"),
    ("^K", "cut"),
    ("^U", "paste"),
    ("Esc", "cancel"),
];
/// The rows beside the text in the large dialog: the prompt, the position row and the
/// footer (the error row and the design row are counted while drawn).
const FIXED_ROWS: u16 = 3;
/// The design row's selection mark and label columns, the goal dialog's.
const MARK_W: usize = 2;
const LABEL_W: usize = 18;

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
    let fixed = FIXED_ROWS + u16::from(form.error.is_some()) + u16::from(form.design.is_some());
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

/// Milestone 9.6: the `design` row, cut to `width`: the mark and the label (accented
/// and bold while focused), `‹ <mode> ›`, then `this round: <mode>` muted.
fn design_row(form: &IterateForm, width: usize, p: Palette) -> Option<Line<'static>> {
    let label = |mode: proto::RoundDesign| match mode {
        proto::RoundDesign::Amend => "amend",
        proto::RoundDesign::Full => "full",
        proto::RoundDesign::Off => "off",
    };
    let value = label(form.design?);
    let focused = form.on_design && !form.submitting;
    let (mark, style) = match focused {
        true => (
            glyph(Glyph::Selection, p.ascii),
            role(Role::Accent, p).add_modifier(Modifier::BOLD),
        ),
        false => (" ", role(Role::Muted, p)),
    };
    let choice_style = match focused {
        true => style,
        false => ratatui::style::Style::default(),
    };
    let mut spans = vec![
        Span::styled(format!("{mark:<MARK_W$}{:<LABEL_W$}", "design"), style),
        Span::styled(kit::choice_in(value, p), choice_style),
    ];
    if let Some(current) = form.current {
        let text = format!("  this round: {}", label(current));
        spans.push(Span::styled(text, role(Role::Muted, p)));
    }
    Some(crate::ui::plan_review::clip(Line::from(spans), width))
}

/// The rows every layout shares: the prompt (muted), the editor's `view.rows` rows, the
/// design row (milestone 9.6) and the error row while set.
fn top_rows(form: &IterateForm, view: EditorView, p: Palette) -> Vec<Line<'static>> {
    let width = usize::from(view.width);
    let mut lines = vec![Line::styled(
        kit::cut(PROMPT, width, ellipsis(p)),
        role(Role::Muted, p),
    )];
    let focused = !form.submitting && !form.on_design;
    lines.extend(kit::editor(&form.text, view.rows, view.width, focused));
    lines.extend(design_row(form, width, p));
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
    lines.push(match (form.submitting, form.design.is_some()) {
        (true, _) => footer(&[("Esc", "close")], view.width, p),
        (false, true) => footer(&DESIGN_FOOTER, view.width, p),
        (false, false) => footer(&FOOTER, view.width, p),
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
    let keys = match (form.submitting, form.design.is_some()) {
        (true, _) => vec![hint("esc", "close", 1)],
        (false, true) => vec![
            hint("^S", "start", 9),
            hint("tab", "next", 6),
            hint("esc", "cancel", 1),
        ],
        (false, false) => vec![hint("^S", "start", 9), hint("esc", "cancel", 1)],
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
