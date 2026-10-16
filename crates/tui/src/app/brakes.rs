//! Milestone 9.16 decisions 31–33: the dispatch hold from the TUI. `C-b H` opens a
//! confirm whose `y` sends `RunRequest::Hold` with the opposite of the snapshot's hold;
//! the reply's message is toasted; Enter on a `DispatchHeld` alert opens that confirm
//! (the hold) or says what releases it (a quota trip); and the Settings screen's
//! read-only hold row. Pure: every request leaves as an `Effect`.

use super::alerts::DispatchHeldKind;
use super::{App, Effect, Modal, PendingAction};
use crate::safe_text::one_line;
use proto::brakes::BrakesInfo;
use proto::{ClientMsg, RunRequest};

/// The confirm's question while dispatch runs.
pub const HOLD_ASK: &str =
    "Hold dispatch daemon-wide? No new session starts; running sessions continue.";
/// The confirm's question while dispatch is held.
pub const RELEASE_ASK: &str = "Release dispatch? New sessions may start again.";
/// Decision 31: Enter on a quota alert.
pub const QUOTA_WAITS: &str = "waits for the window to reset or a lower reading";

/// Decision 33: the Settings screen's read-only row, never saved:
/// `dispatch hold   on since <HH:MM> UTC (<by>) · <prefix> H`, or `… off · <prefix> H`;
/// `dot` is the separator (`theme::dot`, `-` in ASCII).
pub fn hold_row(brakes: &BrakesInfo, prefix: &str, dot: &str) -> String {
    let state = match &brakes.hold {
        Some(hold) => format!(
            "on since {} UTC ({})",
            common::text::hh_mm(hold.since),
            one_line(&hold.by)
        ),
        None => "off".to_owned(),
    };
    format!("dispatch hold   {state} {dot} {} H", one_line(prefix))
}

impl App {
    /// `C-b H`: the confirm, holding when the snapshot says dispatch runs and releasing
    /// when it says it is held.
    pub(super) fn hold_command(&mut self) -> Vec<Effect> {
        let on = self.runs.brakes.hold.is_none();
        let message = if on { HOLD_ASK } else { RELEASE_ASK };
        self.modal = Some(Modal::Confirm {
            message: message.to_owned(),
            action: PendingAction::DispatchHold(on),
        });
        vec![]
    }

    /// The confirm's `y`.
    pub(super) fn send_hold(&mut self, on: bool) -> Vec<Effect> {
        vec![Effect::Send(ClientMsg::Run(RunRequest::Hold { on }))]
    }

    /// `RunReply::Held`: the daemon's message (a refusal is the generic one's toast).
    pub(super) fn on_held(&mut self, message: &str) {
        self.toast(super::runs::capped(message));
    }

    /// Decision 31's Enter on a `DispatchHeld` alert.
    pub(super) fn enter_dispatch_held(&mut self, kind: DispatchHeldKind) -> Vec<Effect> {
        match kind {
            DispatchHeldKind::Hold => self.hold_command(),
            DispatchHeldKind::Quota(_) => {
                self.toast(QUOTA_WAITS);
                vec![]
            }
        }
    }
}

#[cfg(test)]
#[path = "../app_tests/brakes.rs"]
mod tests;
