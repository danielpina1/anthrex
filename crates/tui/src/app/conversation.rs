//! `C-b m` and the conversation view's key routing (task M6.5.12). The view's own state
//! is `crate::conversation`; this file only opens and closes it and keeps
//! `Keymap::conversation_mode` in step with whether it is open.

use super::{App, Effect};
use crossterm::event::KeyEvent;

impl App {
    /// `C-b m`: opens the view on the focused window, or closes it (with every
    /// unsubscribe on its trail) when it is already open.
    pub(super) fn toggle_conversation(&mut self) -> Vec<Effect> {
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
        let effects = self.conversation.on_gone(window_id, agent_id, reason);
        if let Some(reason) = self.conversation.gone_reason() {
            let reason = reason.to_owned();
            self.toast(reason);
        }
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
