//! Milestone 9.3 decision 32: the iterate dialog's opening (the action menu's
//! `iterate`), its keys and paste, its tagged `RunRequest::Iterate` (milestone 9
//! decision 2) and its replies: `Done` closes it with a toast of the daemon's message,
//! `Refused` fills its error row with the daemon's text. The dialog itself is
//! `crate::run_iterate`. Pure: every request leaves as an `Effect`.

use super::{App, Effect, Modal};
use crate::run_goal::EditorView;
use crate::run_iterate::{IterateForm, IterateOutcome, NOT_SENT};
use crossterm::event::KeyEvent;

impl App {
    /// The menu's `iterate` on `run_id`: the dialog for the run's next round, as the
    /// snapshot numbers it. Nothing for a run the snapshot no longer lists.
    pub(super) fn open_iterate(&mut self, run_id: &str) -> Vec<Effect> {
        let Some(run) = self.runs.runs.iter().find(|run| run.run_id == run_id) else {
            return vec![];
        };
        let form = IterateForm::new(run.run_id.clone(), run.round.max(1) + 1);
        self.modal = Some(Modal::Iterate(form));
        vec![]
    }

    /// The open iterate dialog's text area as the last frame drew it (the goal
    /// dialog's rule, decision 7): the terminal is the body plus the status bar's row.
    pub(super) fn iterate_view(&self) -> EditorView {
        let Some(Modal::Iterate(form)) = &self.modal else {
            return EditorView::default();
        };
        let area = self.body_area;
        let rows = if area.height == 0 { 0 } else { area.height + 1 };
        crate::ui::run_iterate::text_view(form, area.width, rows)
    }

    /// The open dialog's keys. Ctrl-S sends one tagged `Iterate` and leaves the dialog
    /// open, submitting, until its reply.
    pub(crate) fn on_iterate_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let view = self.iterate_view();
        let Some(Modal::Iterate(form)) = &mut self.modal else {
            return vec![];
        };
        match form.on_key_in(key, view) {
            IterateOutcome::Stay => vec![],
            IterateOutcome::Close => {
                self.modal = None;
                vec![]
            }
            IterateOutcome::Submit(request) => {
                let (id, effect) = self.tagged_request(request);
                if let Some(Modal::Iterate(form)) = &mut self.modal {
                    form.request_id = Some(id);
                }
                vec![effect]
            }
        }
    }

    /// A bracketed paste into the open dialog's editor, drawn as the last frame drew it.
    pub(super) fn on_iterate_paste(&mut self, text: &str) {
        let view = self.iterate_view();
        if let Some(Modal::Iterate(form)) = &mut self.modal {
            form.on_paste_in(text, view);
        }
    }

    /// The iterate dialog while it waits on the tagged request `id`.
    pub(super) fn iterate_waiting_on(&mut self, id: Option<u64>) -> Option<&mut IterateForm> {
        match &mut self.modal {
            Some(Modal::Iterate(form)) if form.submitting && id.is_some() => {
                (form.request_id == id).then_some(form)
            }
            _ => None,
        }
    }

    /// The `Iterate` the dialog waits on was refused by the connection, or its reply
    /// went with a lost link (`id` `None`): the dialog stops submitting and says so.
    pub(super) fn iterate_not_sent(&mut self, id: Option<u64>) {
        if let Some(Modal::Iterate(form)) = &mut self.modal
            && form.submitting
            && (id.is_none() || form.request_id == id)
        {
            form.submitting = false;
            form.request_id = None;
            form.error = Some(NOT_SENT.into());
        }
    }
}
