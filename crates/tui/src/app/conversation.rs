//! `C-b m` and the conversation view's key routing (task M6.5.12). The view's own state
//! is `crate::conversation`; this file only opens and closes it and keeps
//! `Keymap::conversation_mode` in step with whether it is open.

use super::{App, Effect};
use crossterm::event::KeyEvent;
use proto::conversation::GONE_WINDOW_REMOVED;

impl App {
    /// `C-b m`: opens the view on the focused window, or closes it (with every
    /// unsubscribe on its trail) when it is already open.
    pub(super) fn toggle_conversation(&mut self) -> Vec<Effect> {
        self.conversation_follow = None;
        let effects = if self.conversation.is_open() {
            self.conversation.close()
        } else if let Some(id) = self.focused_window().map(|w| w.id) {
            self.conversation.open(id)
        } else {
            self.toast("no window focused");
            vec![]
        };
        self.sync_conversation_mode();
        effects
    }

    pub(super) fn on_conversation_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let effects = self.conversation.on_key(key);
        self.sync_conversation_mode();
        effects
    }

    /// The daemon ended one of the view's subscriptions. A level on the view's trail is
    /// popped (or, for the root, the view closes), the reason is toasted — including
    /// `GONE_TOO_LARGE`, "conversation too large to send" — and the keymap leaves
    /// conversation mode if the view closed. A `Gone` for a key the view does not hold
    /// changes nothing and shows nothing.
    pub(super) fn on_conversation_gone(
        &mut self,
        window_id: u32,
        agent_id: Option<String>,
        reason: String,
    ) -> Vec<Effect> {
        if !self.conversation.holds(window_id, &agent_id) {
            return vec![];
        }
        // Review M3: the focused window's removal. The window list that follows moves
        // focus, and the view goes with it (`follow_focus`).
        let removed =
            agent_id.is_none() && reason == GONE_WINDOW_REMOVED && self.focused == Some(window_id);
        let effects = self.conversation.on_gone(window_id, agent_id, reason);
        if removed && !self.conversation.is_open() {
            self.conversation_follow = Some(window_id);
        }
        if let Some(reason) = self.conversation.gone_reason() {
            let reason = reason.to_owned();
            self.toast(reason);
        }
        self.sync_conversation_mode();
        effects
    }

    /// Spec §6 (review M3): the view shows the focused window. `App::focus` calls this
    /// once focus has moved to `id`: an open view on another window unsubscribes its
    /// whole trail and its root and opens on `id`'s root. So does a view its window's
    /// removal just closed (`conversation_follow`).
    pub(super) fn follow_focus(&mut self, id: u32) -> Vec<Effect> {
        let follows = self.conversation_follow.take().is_some();
        let elsewhere = self
            .conversation
            .window_id()
            .is_some_and(|shown| shown != id);
        if !follows && !elsewhere {
            return vec![];
        }
        // Final re-review m1: the window list can beat the daemon's `ConversationGone`
        // for a removed window, so say why the view moved whichever arrives first.
        let shown_removed = self
            .conversation
            .window_id()
            .is_some_and(|shown| !self.windows.iter().any(|w| w.id == shown));
        if shown_removed {
            self.toast(GONE_WINDOW_REMOVED);
        }
        let effects = self.conversation.open(id);
        self.sync_conversation_mode();
        effects
    }

    /// Review M3: focus moved to no window at all. An open view closes, with the toast
    /// `C-b m` shows when no window is focused; so does a view already closed by its
    /// window's removal, which says so too.
    pub(super) fn follow_no_focus(&mut self) -> Vec<Effect> {
        let follows = self.conversation_follow.take().is_some();
        if !follows && !self.conversation.is_open() {
            return vec![];
        }
        let effects = self.conversation.close();
        self.toast("no window focused");
        self.sync_conversation_mode();
        effects
    }

    /// `q`, a last `Esc`, or a `ConversationGone` for the root can each close the view;
    /// bare keys go back to the PTY the moment it is.
    pub(crate) fn sync_conversation_mode(&mut self) {
        self.keymap
            .set_conversation_mode(self.conversation.is_open());
    }
}
