//! Milestone 9.6 decision 35: the document gate's note editor (`crate::doc_note`) over
//! the gate screen: its keys and paste, drawn as the last frame drew it; changes and
//! edit send their tagged `DocGate` on Ctrl-S and wait for the reply (a refusal fills
//! the editor's error row, so the text is never lost); rethink and back open their
//! confirm page. Pure: every request leaves as an `Effect`.

use super::{App, Effect, Modal, PendingAction};
use crate::doc_note::{DocNoteForm, NoteFor, NoteOutcome};
use crate::run_goal::EditorView;
use crossterm::event::KeyEvent;
use proto::DocGateAction;

impl App {
    /// The open note editor's text area as the last frame drew it: the terminal is the
    /// body plus the status bar's row (the iterate dialog's rule).
    pub(super) fn doc_note_view(&self) -> EditorView {
        let Some(Modal::DocNote(form)) = &self.modal else {
            return EditorView::default();
        };
        let area = self.body_area;
        let rows = if area.height == 0 { 0 } else { area.height + 1 };
        crate::ui::doc_note::text_view(form, area.width, rows)
    }

    /// The open editor's keys.
    pub(crate) fn on_doc_note_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let view = self.doc_note_view();
        let Some(Modal::DocNote(form)) = &mut self.modal else {
            return vec![];
        };
        let (run_id, kind, version) = (form.run_id.clone(), form.kind, form.version);
        match form.on_key_in(key, view) {
            NoteOutcome::Stay => vec![],
            NoteOutcome::Close => {
                self.modal = None;
                vec![]
            }
            NoteOutcome::Send(action) => {
                let (id, effect) = self.send_doc_gate(run_id, kind, action);
                if let Some(Modal::DocNote(form)) = &mut self.modal {
                    form.request_id = Some(id);
                }
                vec![effect]
            }
            NoteOutcome::Confirm(action) => {
                let round = super::doc_gate::round_of(self, &run_id);
                let message = super::doc_gate::confirm_text(&run_id, round, kind, version, &action);
                self.modal = Some(Modal::Confirm {
                    message,
                    action: PendingAction::DocGate {
                        run_id,
                        kind,
                        action,
                    },
                });
                vec![]
            }
        }
    }

    /// A confirm page left with `n` or Esc (final fix wave FW-79, WB-D m7): a rethink's
    /// or a back's reopens its editor with the note, while the run still waits at that
    /// gate; any other page closes.
    pub(super) fn confirm_declined(&mut self, action: PendingAction) -> Vec<Effect> {
        let PendingAction::DocGate {
            run_id,
            kind,
            action,
        } = action
        else {
            return vec![];
        };
        let (purpose, note) = match action {
            DocGateAction::Rethink { note } => (NoteFor::Rethink, note),
            DocGateAction::Back { note } => (NoteFor::Back, note),
            _ => return vec![],
        };
        let gate = (self.runs.runs.iter())
            .find(|r| r.run_id == run_id)
            .and_then(|r| r.doc_gate.as_ref())
            .filter(|g| g.kind == kind);
        if let Some(gate) = gate {
            let form = DocNoteForm::reopened(run_id, (kind, gate.version), purpose, &note);
            self.modal = Some(Modal::DocNote(Box::new(form)));
        }
        vec![]
    }

    /// A bracketed paste into the open editor, drawn as the last frame drew it.
    pub(super) fn on_doc_note_paste(&mut self, text: &str) {
        let view = self.doc_note_view();
        if let Some(Modal::DocNote(form)) = &mut self.modal {
            form.on_paste_in(text, view);
        }
    }
}
