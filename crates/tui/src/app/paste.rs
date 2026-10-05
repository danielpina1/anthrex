//! Decision 30's paste routing and the bracketed-paste sanitising for the PTY. Moved
//! out of `app/mod.rs` (`AGENTS.md` hard rule 8) so the module keeps room for the
//! plan-gate keys.

use super::{App, Effect, Modal};
use proto::ClientMsg;

const BRACKETED_PASTE_START: &[u8] = b"\x1b[200~";
const BRACKETED_PASTE_END: &[u8] = b"\x1b[201~";

fn push_sanitized_paste_byte(bytes: &mut Vec<u8>, byte: u8) {
    bytes.push(byte);
    while bytes.ends_with(BRACKETED_PASTE_START) || bytes.ends_with(BRACKETED_PASTE_END) {
        bytes.truncate(bytes.len() - BRACKETED_PASTE_START.len());
    }
}

fn sanitize_paste(text: &str) -> Vec<u8> {
    let source = text.as_bytes();
    let mut bytes = Vec::with_capacity(source.len());
    let mut index = 0;
    while index < source.len() {
        if source[index] == b'\r' && source.get(index + 1) == Some(&b'\n') {
            push_sanitized_paste_byte(&mut bytes, b'\r');
            index += 2;
        } else {
            let byte = if source[index] == b'\n' {
                b'\r'
            } else {
                source[index]
            };
            push_sanitized_paste_byte(&mut bytes, byte);
            index += 1;
        }
    }
    bytes
}

impl App {
    pub fn on_paste(&mut self, text: String) -> Vec<Effect> {
        // Decision 30: a paste goes to the open form's focused field, or is dropped for
        // any other modal — both checked before tree mode and the PTY (risk 10).
        // Milestone 9.3: the goal's editor takes the paste as the dialog draws it, and
        // so does the iterate dialog's (decision 32).
        if matches!(self.modal, Some(Modal::Iterate(_))) {
            self.on_iterate_paste(&text);
            return vec![];
        }
        if matches!(self.modal, Some(Modal::DocNote(_))) {
            self.on_doc_note_paste(&text);
            return vec![];
        }
        let goal_view = self.goal_view();
        if let Some(modal) = &mut self.modal {
            match modal {
                Modal::NewAgent(form) => form.on_paste(&text),
                Modal::EditTask(form) => form.on_paste(&text),
                Modal::StartGoal(form) => form.on_paste_in(&text, goal_view),
                Modal::Action(flow) => {
                    if let super::actions::ActionStep::Form(form) = &mut flow.step {
                        form.on_paste(&text);
                    }
                }
                _ => {}
            }
            return vec![];
        }
        // Milestone 9.0.6 decision 33: a screen covers the body; a paste goes to its
        // editor or nowhere.
        if self.screen.is_some() {
            self.on_profile_paste(&text);
            self.on_settings_paste(&text);
            return vec![];
        }
        // Milestone 9.0.5 review finding 1: the plan review covers the body, so a
        // paste reaches neither the PTY nor the conversation's search under it.
        if self.plan_review.is_some() {
            return vec![];
        }
        // Review of task 8: nor while the Alerts box has the keys.
        if self.alerts_focus.is_some() {
            return vec![];
        }
        // Decision 11: the conversation view is read-only, so a paste while it is open
        // goes to its search query or nowhere — never to the PTY underneath.
        if self.conversation.is_open() {
            self.conversation.on_paste(&text);
            return vec![];
        }
        if self.tree_input.is_some() {
            return self.on_tree_paste(text);
        }
        let Some(id) = self.focused_pty() else {
            return vec![];
        };
        self.scroll_to_live();
        let bracketed = self.parser.screen().bracketed_paste();
        let mut bytes = Vec::with_capacity(text.len() + 12);
        if bracketed {
            bytes.extend_from_slice(BRACKETED_PASTE_START);
        }
        bytes.extend_from_slice(&sanitize_paste(&text));
        if bracketed {
            bytes.extend_from_slice(BRACKETED_PASTE_END);
        }
        vec![Effect::Send(ClientMsg::Input {
            window_id: id,
            bytes,
        })]
    }
}
