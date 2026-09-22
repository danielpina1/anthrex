//! Task M6.11: whether the client currently has a working connection to the daemon,
//! is retrying one, or has given up (decision 30) — plus the pure transitions between
//! those states (decisions 31 and 33). Split out of `app/mod.rs` per `AGENTS.md` hard
//! rule 8, the same shape `app/lifecycle.rs` and `app/windows.rs` already give their
//! own tasks. `on_send_failed` and `check_stopping_timeout` (task M6.10) moved here
//! too, alongside `on_link_lost`: all three are "how the link notices and recovers
//! from a failure" concerns, and splitting them across two files once `on_link_lost`
//! had to know about `Link` would only separate code that has to change together.
//!
//! Critical fix folded in here rather than left for a later wave (see this task's
//! report): the event loop's read arm used to be gated on `App.connected`, a flag
//! `DaemonMsg::Bye`'s handler set to `false` the moment `Bye` arrived — before the
//! socket had actually closed. That permanently stopped the read arm from ever
//! observing the real close, so `on_link_lost` (and, in the `C-b Q` case, the `Quit`
//! it produces) never ran. Decision 30 already pointed at the fix: `Bye` now only
//! toasts, `self.link` changes *only* when the channel actually closes, and
//! `crates/tui/src/lib.rs`'s read arm is gated on the connection itself
//! (`Option<Connection>::is_some()`), never on `self.link` or `self.connected()`.
//!
//! `stop_daemon_command` and `start_stopping` moved in from `app/lifecycle.rs` in a
//! follow-up closing the M6.10 review's Minor finding and its own disclosed
//! naming-mismatch: decision 39 names this file for "stopping", so the guard against
//! a second `C-b Q` re-stamping the wait belongs alongside the three ways the wait
//! already ends here, not split across two files.

use super::{App, Effect, Modal, PendingAction};
use proto::{ClientMsg, WindowInfo};
use std::time::{Duration, Instant};

/// Decision 36's timeout, moved here from `app/lifecycle.rs` alongside
/// `check_stopping_timeout`: how long `C-b Q` waits for the daemon to confirm a
/// `Shutdown` (by closing the link) before giving up and telling the user to stop it
/// by hand.
pub const STOPPING_TIMEOUT: Duration = Duration::from_secs(5);

/// Decision 30. Pure client state; the event loop in `crates/tui/src/lib.rs` owns the
/// socket, the timers and the attempt task that drive the transitions between these.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Connected,
    /// An automatic or manual reconnect attempt is scheduled or in flight.
    /// `attempts` counts failed attempts within the current 30 s window (decision 34's
    /// "reconnecting (attempt N)").
    Reconnecting {
        attempts: u32,
        reason: String,
    },
    /// The 30 s window elapsed without a working connection (decision 31); only a
    /// fresh `C-b r` (decision 32) tries again.
    Lost {
        reason: String,
    },
}

impl App {
    pub fn connected(&self) -> bool {
        matches!(self.link, Link::Connected)
    }

    /// `C-b r` (decisions 23 and 32). Connected, it is a no-op affirmation. Not
    /// connected, it starts an attempt at once and opens a fresh 30 s window —
    /// `lib.rs` is the one that actually checks "unless one is already in flight"
    /// before spawning it, since `App` has no visibility into the event loop's
    /// in-flight task.
    pub(super) fn reconnect_command(&mut self) -> Vec<Effect> {
        if self.connected() {
            self.toast("connected");
            return vec![];
        }
        let reason = match &self.link {
            Link::Reconnecting { reason, .. } | Link::Lost { reason } => reason.clone(),
            Link::Connected => String::new(),
        };
        self.link = Link::Reconnecting {
            attempts: 0,
            reason,
        };
        vec![Effect::Reconnect]
    }

    /// `on_tick`'s decision 35 share: a full outgoing queue drops a `Subscribe`
    /// silently (`Input` does the same on every keystroke, so this is the one
    /// command worth retrying instead of leaving the user stuck on a stale screen);
    /// retried every 100 ms until it drains, i.e. until `subscribed` catches up with
    /// `focused` again.
    pub(super) fn retry_dropped_subscribe(&mut self) -> Option<Effect> {
        if self.connected()
            && let Some(id) = self.focused
            && self.subscribed != Some(id)
        {
            let (cols, rows) = self.term_size;
            self.subscribed = Some(id);
            return Some(Effect::Send(ClientMsg::Subscribe {
                window_id: id,
                cols,
                rows,
            }));
        }
        None
    }

    /// `C-b Q`, before the confirm dialog even opens. Decision 39 puts all of
    /// `stopping`'s pure logic in this module, so the guard belongs here too, not as
    /// an inline check in `app/mod.rs::run`. Already waiting for a confirmed
    /// `Shutdown`, a second attempt does not reopen the dialog — reopening it would
    /// let a second `y` call `start_stopping` again and re-stamp the clock, which is
    /// exactly the M6.10 review's Minor finding. Silently doing nothing was rejected:
    /// pressing `C-b Q` twice means the user thinks the first press had no effect, and
    /// this codebase already has a precedent for saying so instead of staying quiet —
    /// `app/modal_keys.rs::on_remove_confirm_key`'s `pending_worktree_remove` guard
    /// toasts "already removing/still removing ..." rather than silently refusing.
    pub(super) fn stop_daemon_command(&mut self) -> Vec<Effect> {
        if self.stopping.is_some() {
            self.toast("already waiting for the daemon to stop");
            return vec![];
        }
        self.modal = Some(Modal::Confirm {
            message: "Stop the daemon and kill every agent?".into(),
            action: PendingAction::StopDaemon,
        });
        vec![]
    }

    /// `PendingAction::StopDaemon`'s confirm (`app/lifecycle.rs::perform`): arms the
    /// wait and sends the one `Shutdown`. Moved here, alongside the other three ways
    /// the wait ends (`on_link_lost`, `on_send_failed`, `check_stopping_timeout`), so
    /// decision 39's "stopping" pure logic lives in one file instead of being split
    /// between where the wait starts and where it can end.
    pub(super) fn start_stopping(&mut self) -> Vec<Effect> {
        self.stopping = Some(Instant::now());
        vec![Effect::Send(ClientMsg::Shutdown)]
    }

    /// Called by the event loop when the daemon closes the connection or the socket
    /// otherwise drops — never when `DaemonMsg::Bye` merely *arrives* (see this
    /// module's doc comment on why those are different events). `reason` is the last
    /// `Bye` reason the event loop saw, or "connection closed" when none arrived.
    ///
    /// Decision 36: if `C-b Q` was waiting for exactly this (`stopping` is set), the
    /// drop *is* the confirmation, so the client quits instead of starting a
    /// reconnect.
    pub fn on_link_lost(&mut self, reason: impl Into<String>) -> Vec<Effect> {
        if self.stopping.take().is_some() {
            return vec![Effect::Quit];
        }
        self.link = Link::Reconnecting {
            attempts: 0,
            reason: reason.into(),
        };
        self.toast("connection to the daemon lost");
        vec![]
    }

    /// Called by the event loop when a reconnect attempt (automatic or `C-b r`)
    /// fails. `gave_up` is the event loop's own `reconnect::RetrySchedule::after_failure`
    /// result: once true, the 30 s window has elapsed and only a fresh `C-b r` opens a
    /// new one.
    pub fn on_reconnect_failed(&mut self, reason: impl Into<String>, gave_up: bool) -> Vec<Effect> {
        let reason = reason.into();
        self.link = if gave_up {
            Link::Lost { reason }
        } else {
            let attempts = match &self.link {
                Link::Reconnecting { attempts, .. } => attempts + 1,
                _ => 1,
            };
            Link::Reconnecting { attempts, reason }
        };
        vec![]
    }

    /// Decision 33: on success, resubscribes the focused window with a fresh parser,
    /// even though it is already `focused` — the daemon's forwarder for the old
    /// subscription is gone, so `App::focus`'s early return (which exists for the
    /// ordinary "nothing changed" case) must not suppress this one. When the focused
    /// window itself vanished while disconnected, `replace_windows`'s own neighbour
    /// fallback (task M5's `removed_focused_window_moves_focus_to_a_neighbour`) has
    /// already sent exactly one `Subscribe` for the window that took over focus, so
    /// this only adds its own `Subscribe` when that fallback did not fire — never
    /// both, which is what keeps this to "exactly one `Subscribe` per visible window."
    pub fn on_reconnected(&mut self, windows: Vec<WindowInfo>) -> Vec<Effect> {
        self.link = Link::Connected;
        self.toast("reconnected");
        let mut effects = self.replace_windows(windows);
        let already_resubscribed = effects.iter().any(|effect| {
            matches!(
                effect,
                Effect::Send(ClientMsg::Subscribe { window_id, .. })
                    if Some(*window_id) == self.focused
            )
        });
        if !already_resubscribed && let Some(id) = self.focused {
            let (cols, rows) = self.term_size;
            self.parser =
                vt100::Parser::new(rows.max(1), cols.max(1), self.settings.scrollback_lines);
            self.subscribed = Some(id);
            effects.push(Effect::Send(ClientMsg::Subscribe {
                window_id: id,
                cols,
                rows,
            }));
        }
        effects
    }

    /// Called by the event loop when `Connection::send` reports the outgoing queue is
    /// full or the connection is gone (decision 35 generalizes this to every
    /// `ClientMsg`, not just `Shutdown`).
    pub fn on_send_failed(&mut self, msg: &ClientMsg) -> Vec<Effect> {
        match msg {
            // Decision 36's half of this: a refused `Shutdown` means `C-b Q` cannot be
            // confirmed by the daemon at all, so the wait ends right here rather than
            // sitting until `STOPPING_TIMEOUT`.
            ClientMsg::Shutdown => {
                if self.stopping.take().is_some() {
                    self.toast("could not reach the daemon; run anthrex daemon stop");
                }
            }
            // Decision 35: clears the optimistic "handed to the connection" mark so
            // `on_tick` (and a later `App::focus` call, whose early return checks the
            // same field) knows to try again.
            ClientMsg::Subscribe { window_id, .. } => {
                if self.subscribed == Some(*window_id) {
                    self.subscribed = None;
                }
            }
            // A dropped keystroke is not worth a toast; the next one will try again.
            ClientMsg::Input { .. } => {}
            _ => {
                if self.connected() {
                    self.toast("daemon is not responding");
                } else {
                    self.toast(format!(
                        "not connected; {} r to reconnect",
                        self.settings.prefix_label
                    ));
                }
            }
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
