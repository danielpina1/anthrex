//! Milestone 9.6 decision 35 (DF §6.1): the document gate's note editor, pure
//! (`AGENTS.md` hard rule 5). One form serves four keys: `c` (changes: a note and
//! whether the revision is reviewed again, decision 15's default `no`), `r` (rethink:
//! a note), `b` (back: a note) and `e` (edit: the document's own text, the 9.3 editor's
//! Ctrl-S saving it as the next version). The text is the 9.3 editor's
//! (`TextArea::on_editor_key`). Ctrl-S hands the gate's action back: changes and edit
//! are sent at once (their save is their confirmation), rethink and back go on to their
//! confirm page. Esc on an unchanged text closes; on a changed one it asks first, as the
//! goal dialog does (decision 8). Opening, sending and the replies are
//! `app/doc_gate_note.rs`'s; rendering is `ui/doc_note.rs`.

use crate::run_goal::EditorView;
use crate::text_area::TextArea;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{DocGateAction, DocGateKind};

/// The most characters a note holds (the daemon's goal cap, as the iterate dialog's).
pub const NOTE_MAX_CHARS: usize = proto::GOAL_MAX_CHARS;
/// The most characters the edited document holds: `run edit-doc`'s 64 KiB, the most
/// the daemon shows of any version (task M9.6.16).
pub const EDIT_MAX_CHARS: usize = 64 * 1024;
/// Ctrl-S on an empty edit.
pub const EMPTY_EDIT: &str = "the document is empty; write it first";
/// The inline error after the connection refused the request or the link was lost.
pub const NOT_SENT: &str = "the request was not sent; press Ctrl-S to retry";

/// Which gate key opened the form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NoteFor {
    Changes,
    Rethink,
    Back,
    Edit,
}

impl NoteFor {
    /// The dialog's verb, in its title and its Ctrl-S hint.
    pub fn verb(self) -> &'static str {
        match self {
            NoteFor::Changes => "ask for changes",
            NoteFor::Rethink => "rethink",
            NoteFor::Back => "go back",
            NoteFor::Edit => "save",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocNoteForm {
    pub run_id: String,
    pub kind: DocGateKind,
    /// The gate version the form was opened at.
    pub version: u32,
    pub purpose: NoteFor,
    pub text: TextArea,
    /// Changes only: whether the revision goes to the document reviewer again
    /// (`Changes { review }`); decision 15's default is no.
    pub review: bool,
    pub error: Option<String>,
    pub submitting: bool,
    /// The id of the tagged `DocGate` the form waits on (changes and edit).
    pub request_id: Option<u64>,
    /// Esc on a changed text asks before discarding it.
    pub discarding: bool,
    /// The text the form opened with: Esc closes at once while it is unchanged.
    start: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoteOutcome {
    Stay,
    Close,
    /// Send this action now (changes, edit).
    Send(DocGateAction),
    /// Ask this action's confirm page first (rethink, back).
    Confirm(DocGateAction),
}

fn is_ctrl(key: &KeyEvent, c: char) -> bool {
    key.modifiers.contains(KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char(k) if k.eq_ignore_ascii_case(&c))
}

impl DocNoteForm {
    /// A form for `purpose` at `kind`'s gate, version `version`, holding `text`: the
    /// prefilled note (the report's questions for changes at the brainstorm gate) or the
    /// document for an edit.
    pub fn new(
        run_id: String,
        (kind, version): (DocGateKind, u32),
        purpose: NoteFor,
        text: &str,
    ) -> Self {
        let cap = match purpose {
            NoteFor::Edit => EDIT_MAX_CHARS,
            _ => NOTE_MAX_CHARS,
        };
        let text = TextArea::with_cap(text, cap);
        Self {
            run_id,
            kind,
            version,
            purpose,
            start: text.text().to_owned(),
            text,
            review: false,
            error: None,
            submitting: false,
            request_id: None,
            discarding: false,
        }
    }

    /// Final fix wave FW-79: a rethink's or a back's form reopened with `note` after its
    /// confirm page was left, as it opened (an empty start), so Esc still asks before
    /// discarding the note.
    pub fn reopened(
        run_id: String,
        (kind, version): (DocGateKind, u32),
        purpose: NoteFor,
        note: &str,
    ) -> Self {
        let mut form = Self::new(run_id, (kind, version), purpose, note);
        form.start = String::new();
        form
    }

    /// The form's keys, the text drawn as `view` says. The discard page takes the next
    /// key: a plain `y` closes, any other goes back. Esc and Ctrl-C close at once while
    /// submitting or on an unchanged text. While submitting nothing else acts. Ctrl-S
    /// hands the action back; Tab toggles the review of a changes request; every other
    /// key is the editor's.
    pub fn on_key_in(&mut self, key: KeyEvent, view: EditorView) -> NoteOutcome {
        if self.discarding {
            self.discarding = false;
            let plain = !key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT);
            if key.code == KeyCode::Char('y') && plain {
                return NoteOutcome::Close;
            }
            return NoteOutcome::Stay;
        }
        if key.code == KeyCode::Esc || is_ctrl(&key, 'c') {
            if self.submitting || self.text.text() == self.start {
                return NoteOutcome::Close;
            }
            self.discarding = true;
            return NoteOutcome::Stay;
        }
        if self.submitting {
            return NoteOutcome::Stay;
        }
        if is_ctrl(&key, 's') {
            return self.save();
        }
        if key.code == KeyCode::Tab && self.purpose == NoteFor::Changes {
            self.review = !self.review;
            return NoteOutcome::Stay;
        }
        self.text.on_editor_key(key, view.width, view.rows);
        NoteOutcome::Stay
    }

    /// A bracketed paste into the text, through the editor. Nothing while submitting or
    /// on the discard page.
    pub fn on_paste_in(&mut self, text: &str, view: EditorView) {
        if self.submitting || self.discarding {
            return;
        }
        self.text.on_editor_paste(text, view.width, view.rows);
    }

    /// The gate's action this form saves: a note trimmed (it may be empty), the edited
    /// document as written.
    pub fn action(&self) -> DocGateAction {
        let note = self.text.text().trim().to_owned();
        match self.purpose {
            NoteFor::Changes => DocGateAction::Changes {
                note,
                review: self.review,
            },
            NoteFor::Rethink => DocGateAction::Rethink { note },
            NoteFor::Back => DocGateAction::Back { note },
            NoteFor::Edit => DocGateAction::Edit {
                text: self.text.text().to_owned(),
            },
        }
    }

    fn save(&mut self) -> NoteOutcome {
        if self.purpose == NoteFor::Edit && self.text.text().trim().is_empty() {
            self.error = Some(EMPTY_EDIT.into());
            return NoteOutcome::Stay;
        }
        self.error = None;
        match self.purpose {
            NoteFor::Changes | NoteFor::Edit => {
                self.submitting = true;
                NoteOutcome::Send(self.action())
            }
            NoteFor::Rethink | NoteFor::Back => NoteOutcome::Confirm(self.action()),
        }
    }
}

/// The `## Questions for you` section of a merged report (DF §3.4), without its
/// heading, trimmed: what `c` at the brainstorm gate opens with. Empty when the report
/// has none. The section ends at the next `#` or `##` heading.
pub fn report_questions(report: &str) -> String {
    let mut lines = report.lines();
    if !lines.any(|l| l.trim_end() == "## Questions for you") {
        return String::new();
    }
    let section: Vec<&str> = lines
        .take_while(|l| !(l.starts_with("# ") || l.starts_with("## ")))
        .collect();
    section.join("\n").trim().to_owned()
}

#[cfg(test)]
#[path = "doc_note_tests.rs"]
mod tests;
