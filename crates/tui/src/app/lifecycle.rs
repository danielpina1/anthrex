//! Task M6.10: the restart confirmation (decision 23) and dispatching `PendingAction`.
//! Split out of `app/mod.rs` (`AGENTS.md` hard rule 8) — the same shape
//! `modal_keys.rs` and `windows.rs` already give their own tasks' `impl App` blocks.
//! `C-b Q`'s wait to stop the daemon (decision 36) used to be armed here too, but
//! decision 39 names `app/link.rs` as where all of `stopping`'s pure logic belongs;
//! a follow-up (closing the M6.10 review's naming-mismatch finding) moved
//! `start_stopping` and the pre-confirm guard there alongside `on_link_lost`,
//! `on_send_failed` and `check_stopping_timeout`, the three ways the wait ends. This
//! file's `perform` arm for `PendingAction::StopDaemon` only dispatches to it.

use super::{App, Effect, Modal, PendingAction};
use proto::{ClientMsg, Status, WindowKind};

impl App {
    pub(super) fn perform(&mut self, action: PendingAction) -> Vec<Effect> {
        match action {
            PendingAction::Kill(id) => vec![Effect::Send(ClientMsg::Kill { window_id: id })],
            PendingAction::Restart(id) => {
                vec![Effect::Send(ClientMsg::Restart { window_id: id })]
            }
            // Decision 36: only the send. Quitting happens once the daemon actually
            // confirms — see `app/link.rs`'s `on_link_lost`, `on_send_failed` and
            // `check_stopping_timeout` for the three ways that wait can end.
            // `start_stopping` lives there too (decision 39): this arm only dispatches.
            PendingAction::StopDaemon => self.start_stopping(),
        }
    }

    /// Milestone 8c decision 26: the daemon's own refusal text when the focused window is
    /// headless, so `C-b x`, `C-b X` and `C-b R` toast it and open nothing.
    fn headless_control_refusal(&self) -> Option<String> {
        let w = self.focused_window()?;
        (w.kind == WindowKind::Headless)
            .then(|| daemon::manager::control_refusal(w.id, w.run.as_ref()))
    }

    /// `true`, with the refusal toasted, when a control command must stop here.
    pub(super) fn refuse_headless_control(&mut self) -> bool {
        let Some(refusal) = self.headless_control_refusal() else {
            return false;
        };
        self.toast(refusal);
        true
    }

    /// Decision 23: `C-b R`. An exited window restarts at once; a live one asks first,
    /// through the same generic `Confirm` modal `confirm_focused` uses for `Kill`, with
    /// `PendingAction::Restart(id)` carrying the window id so the eventual `y` cannot
    /// act on whatever happens to be focused by the time it is pressed.
    pub(super) fn restart_focused(&mut self) -> Vec<Effect> {
        if self.refuse_headless_control() {
            return vec![];
        }
        let Some(w) = self.focused_window() else {
            return vec![];
        };
        let (id, name) = (w.id, w.name.clone());
        if w.status == Status::Exited {
            self.toast(format!("restarting {name}"));
            vec![Effect::Send(ClientMsg::Restart { window_id: id })]
        } else {
            self.modal = Some(Modal::Confirm {
                message: format!("Restart '{name}'? It is running and will be stopped first."),
                action: PendingAction::Restart(id),
            });
            vec![]
        }
    }
}
