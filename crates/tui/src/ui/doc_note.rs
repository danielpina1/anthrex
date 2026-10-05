//! Milestone 9.6 decision 35: the document gate's note editor (`crate::doc_note`). A
//! note (changes, rethink, back) is the 9.3 editor in a small dialog (milestone 9.0.6's
//! grammar: at most 64 wide, `NOTE_ROWS` rows of text), titled `<verb> · <kind> v<n>`,
//! with a prompt line, at the changes request the `review ‹ no ›` row (Tab toggles it,
//! decision 15's default no), the error row while set, the position row and the hints.
//! An edit is the 9.3 editor's large dialog from 60×16 (`ui::goal_editor::dialog_rect`),
//! titled `edit <kind> v<n>`, its footer `^S save  ^K cut  ^U paste  Esc cancel`; below
//! that size it is the small dialog too. The text area's width and rows ([`text_view`])
//! are the ones the keys move by. Esc on a changed text draws its discard page. Every
//! drawn row passes `safe_text`. Pure: no I/O.

use crate::doc_note::{DocNoteForm, NoteFor};
use crate::run_goal::EditorView;
use crate::safe_text::one_line;
use crate::theme::{Palette, Role, dot, dot_sep, ellipsis, role};
use crate::ui::goal_editor::{dialog_rect, footer, is_large};
use crate::ui::kit::{self, Hint};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Clear, Paragraph};

/// A note's text rows.
pub const NOTE_ROWS: u16 = 6;
/// The edit dialog's footer, dropped from the right when narrow.
pub const EDIT_FOOTER: [(&str, &str); 4] = [
    ("^S", "save"),
    ("^K", "cut"),
    ("^U", "paste"),
    ("Esc", "cancel"),
];

fn hint(key: &str, word: &str, priority: u8) -> Hint {
    Hint {
        key: key.to_string(),
        word: word.to_string(),
        priority,
    }
}

/// The prompt line over a note.
fn prompt(purpose: NoteFor) -> &'static str {
    match purpose {
        NoteFor::Changes => "what should the orchestrator change?",
        NoteFor::Rethink => "what should the brainstormers think again about?",
        NoteFor::Back => "what should change in the earlier document?",
        NoteFor::Edit => "your text becomes the next version",
    }
}

/// Whether `form` is drawn in the large dialog in a terminal `cols`×`rows`.
fn large(form: &DocNoteForm, cols: u16, rows: u16) -> bool {
    form.purpose == NoteFor::Edit && is_large(cols, rows)
}

/// The small dialog's interior width in a terminal `cols` wide.
fn small_width(cols: u16) -> u16 {
    cols.min(kit::DIALOG_MAX).saturating_sub(4).min(kit::WRAP)
}

/// The rows the large dialog keeps beside the text: the prompt, the position row and
/// the footer, and the error row while set.
fn fixed_rows(form: &DocNoteForm) -> u16 {
    3 + u16::from(form.error.is_some())
}

/// The text area in a terminal `cols`×`rows`, as the dialog draws it.
pub fn text_view(form: &DocNoteForm, cols: u16, rows: u16) -> EditorView {
    if !large(form, cols, rows) {
        return EditorView {
            width: small_width(cols),
            rows: NOTE_ROWS,
        };
    }
    let rect = dialog_rect(Rect::new(0, 0, cols, rows));
    EditorView {
        width: rect.width.saturating_sub(4),
        rows: rect
            .height
            .saturating_sub(2)
            .saturating_sub(fixed_rows(form))
            .max(1),
    }
}

/// The dialog's title: `<verb> · <kind> v<n>`, or `edit <kind> v<n>`.
pub fn title(form: &DocNoteForm, width: u16, p: Palette) -> String {
    let what = format!("{} v{}", form.kind.label(), form.version);
    let text = match form.purpose {
        NoteFor::Edit => format!("edit {what}"),
        purpose => format!("{} {} {what}", purpose.verb(), dot(p)),
    };
    kit::cut(&text, usize::from(width), ellipsis(p))
}

/// The rows every layout shares: the prompt, the editor, the review row (changes) and
/// the error row while set.
fn top_rows(form: &DocNoteForm, view: EditorView, p: Palette) -> Vec<Line<'static>> {
    let width = usize::from(view.width);
    let mut lines = vec![Line::styled(
        kit::cut(prompt(form.purpose), width, ellipsis(p)),
        role(Role::Muted, p),
    )];
    lines.extend(kit::editor(
        &form.text,
        view.rows,
        view.width,
        !form.submitting,
    ));
    if form.purpose == NoteFor::Changes {
        let value = if form.review { "yes" } else { "no" };
        let row = format!("review again  {}", kit::choice_in(value, p));
        lines.push(Line::raw(kit::cut(&row, width, ellipsis(p))));
    }
    if let Some(error) = &form.error {
        lines.push(Line::styled(
            kit::cut(&one_line(error), width, ellipsis(p)),
            role(Role::Failed, p),
        ));
    }
    lines
}

/// `sending…` while the request waits on its reply, else the position row.
fn position(form: &DocNoteForm, width: u16, p: Palette) -> Line<'static> {
    if form.submitting {
        return Line::styled(format!("sending{}", ellipsis(p)), role(Role::Muted, p));
    }
    kit::editor_position(&form.text, width, p)
}

/// The small dialog's rows at `width` interior columns.
pub fn small_body(form: &DocNoteForm, width: u16, p: Palette) -> Vec<Line<'static>> {
    let view = EditorView {
        width,
        rows: NOTE_ROWS,
    };
    let mut lines = top_rows(form, view, p);
    lines.push(position(form, width, p));
    let keys = match (form.submitting, form.purpose) {
        (true, _) => vec![hint("esc", "close", 1)],
        (false, NoteFor::Changes) => vec![
            hint("^S", form.purpose.verb(), 9),
            hint("tab", "review", 5),
            hint("esc", "cancel", 1),
        ],
        (false, purpose) => vec![hint("^S", purpose.verb(), 9), hint("esc", "cancel", 1)],
    };
    lines.push(kit::hints_joined(width, &keys, dot_sep(p), p));
    lines
}

/// The large (edit) dialog's rows for the text drawn as `view`.
pub fn large_body(form: &DocNoteForm, view: EditorView, p: Palette) -> Vec<Line<'static>> {
    let mut lines = top_rows(form, view, p);
    lines.push(position(form, view.width, p));
    lines.push(match form.submitting {
        true => footer(&[("Esc", "close")], view.width, p),
        false => footer(&EDIT_FOOTER, view.width, p),
    });
    lines
}

/// The discard page (Esc on a changed text).
fn render_discard(frame: &mut Frame, form: &DocNoteForm, rect: Rect, p: Palette) {
    let ask = match form.purpose {
        NoteFor::Edit => "discard your edit?",
        _ => "discard this note?",
    };
    let width = rect.width.saturating_sub(4);
    let hints = [hint("y", "discard", 9), hint("any other key", "back", 1)];
    let keys = kit::hints_joined(width, &hints, dot_sep(p), p);
    let body = vec![
        Line::raw(ask),
        Line::raw(""),
        kit::destructive(keys, "y", "discard", p),
    ];
    frame.render_widget(
        Paragraph::new(body).block(kit::dialog_frame("discard", true, p)),
        rect,
    );
}

/// The editor over the whole terminal `area`.
pub fn render(frame: &mut Frame, form: &DocNoteForm, area: Rect, p: Palette) {
    let (rect, body, width) = if large(form, area.width, area.height) {
        let view = text_view(form, area.width, area.height);
        (dialog_rect(area), large_body(form, view, p), view.width)
    } else {
        let width = small_width(area.width);
        let body = small_body(form, width, p);
        (kit::dialog_area(area, body.len() as u16), body, width)
    };
    if rect.width == 0 || rect.height == 0 {
        return;
    }
    frame.render_widget(Clear, rect);
    if form.discarding {
        return render_discard(frame, form, rect, p);
    }
    let title = title(form, width.saturating_sub(2), p);
    frame.render_widget(
        Paragraph::new(body).block(kit::dialog_frame(&title, false, p)),
        rect,
    );
}
