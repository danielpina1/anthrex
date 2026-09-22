//! Task M6.10: the restart confirmation (decision 23) and the three ways `C-b Q`'s wait
//! to stop the daemon can end (decision 36). Split out of `app/mod.rs`
//! (`AGENTS.md` hard rule 8) — the same shape `modal_keys.rs` and `windows.rs` already
//! give their own tasks' `impl App` blocks.

use super::{App, Effect, Modal, PendingAction};
use proto::{ClientMsg, Status};
use std::time::{Duration, Instant};

/// Decision 36: how long `C-b Q` waits for the daemon to confirm a `Shutdown` (by
/// closing the link) before giving up and telling the user to stop it by hand.
pub const STOPPING_TIMEOUT: Duration = Duration::from_secs(5);

impl App {
    pub(super) fn perform(&mut self, action: PendingAction) -> Vec<Effect> {
        match action {
            PendingAction::Kill(id) => vec![Effect::Send(ClientMsg::Kill { window_id: id })],
            PendingAction::Restart(id) => {
                vec![Effect::Send(ClientMsg::Restart { window_id: id })]
            }
            // Decision 36: only the send. Quitting happens once the daemon actually
            // confirms — see `on_link_lost`, `on_send_failed` and
            // `check_stopping_timeout` for the three ways that wait can end.
            PendingAction::StopDaemon => {
                self.stopping = Some(Instant::now());
                vec![Effect::Send(ClientMsg::Shutdown)]
            }
        }
    }

    /// Decision 23: `C-b R`. An exited window restarts at once; a live one asks first,
    /// through the same generic `Confirm` modal `confirm_focused` uses for `Kill`, with
    /// `PendingAction::Restart(id)` carrying the window id so the eventual `y` cannot
    /// act on whatever happens to be focused by the time it is pressed.
    pub(super) fn restart_focused(&mut self) -> Vec<Effect> {
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

    /// Called by the event loop when the daemon closes the connection or the socket
    /// otherwise drops. Decision 36: if `C-b Q` was waiting for exactly this (`stopping`
    /// is set), the drop *is* the confirmation, so the client quits; `DaemonMsg::Bye`
    /// alone never does this (see its own doc comment in `app/mod.rs`'s `on_daemon`)
    /// because it can arrive before the socket is actually gone. Otherwise this is an
    /// unplanned disconnect; M6.11 (decisions 30-35) adds the reconnect attempt and its
    /// own status text, so for now this only reports the drop.
    pub fn on_link_lost(&mut self) -> Vec<Effect> {
        self.connected = false;
        if self.stopping.take().is_some() {
            return vec![Effect::Quit];
        }
        self.toast(format!(
            "connection to daemon lost; {} d to exit",
            self.settings.prefix_label
        ));
        vec![]
    }

    /// Called by the event loop when `Connection::send` reports the outgoing queue is
    /// full or gone. Decision 36's half of this: a refused `Shutdown` means `C-b Q`
    /// cannot be confirmed by the daemon at all, so the wait ends right here rather
    /// than sitting until `STOPPING_TIMEOUT`. Every other refused message still gets
    /// `lib.rs`'s own generic handling (decision 35, M6.11, moves the rest of it here).
    pub fn on_send_failed(&mut self, msg: &ClientMsg) -> Vec<Effect> {
        if matches!(msg, ClientMsg::Shutdown) && self.stopping.take().is_some() {
            self.toast("could not reach the daemon; run anthrex daemon stop");
        }
        vec![]
    }

    /// `on_tick`'s share of decision 36: the third of the three ways the wait can end —
    /// nothing arrived at all within `STOPPING_TIMEOUT`.
    pub(super) fn check_stopping_timeout(&mut self) {
        if self
            .stopping
            .is_some_and(|at| at.elapsed() >= STOPPING_TIMEOUT)
        {
            self.stopping = None;
            self.toast("the daemon did not confirm the shutdown; run anthrex daemon stop");
        }
    }
}
