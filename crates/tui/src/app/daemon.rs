//! `App::on_daemon`: how each daemon message changes the client's state. Split out of
//! `app/mod.rs` (task M6.5.12 fix round 1) to keep that file under 600 lines; task
//! M6.5.13 routes the conversation messages here.

use super::{App, Effect, Modal};
use proto::DaemonMsg;

impl App {
    pub fn on_daemon(&mut self, msg: DaemonMsg) -> Vec<Effect> {
        match msg {
            DaemonMsg::Welcome { windows, .. } | DaemonMsg::WindowsChanged { windows } => {
                self.replace_windows(windows)
            }
            DaemonMsg::Created { window_id } => {
                // Decision 34: a still-open form closes and hands its values on.
                match self.modal.take() {
                    Some(Modal::NewAgent(form)) => self.form_defaults = form.defaults(),
                    other => self.modal = other,
                }
                if self.windows.iter().any(|w| w.id == window_id) {
                    self.focus(window_id)
                } else {
                    self.pending_focus = Some(window_id);
                    vec![]
                }
            }
            DaemonMsg::Snapshot {
                window_id,
                cols,
                rows,
                bytes,
            } => {
                if Some(window_id) == self.focused {
                    self.parser = vt100::Parser::new(
                        rows.max(1),
                        cols.max(1),
                        self.settings.scrollback_lines,
                    );
                    self.parser.process(&bytes);
                    self.scroll_offset = 0;
                }
                vec![]
            }
            DaemonMsg::Output { window_id, bytes } => {
                if Some(window_id) == self.focused {
                    self.parser.process(&bytes);
                }
                vec![]
            }
            DaemonMsg::Error { request, message } => {
                // Decision 33: a submitting `create` failure goes inline, not a toast.
                if request == proto::messages::request::CREATE
                    && let Some(Modal::NewAgent(form)) = &mut self.modal
                    && form.submitting
                {
                    form.error = Some(message);
                    form.submitting = false;
                } else {
                    self.clear_pending_worktree_remove_on(&request);
                    self.toast(message);
                }
                vec![]
            }
            // Decision 36: the force-or-keep follow-up, but only when this client asked
            // for *this* window's worktree to be removed. A refusal that cannot be
            // matched to an outstanding removal is shown as a toast and nothing is
            // offered to force — see `open_force_remove`.
            DaemonMsg::RemoveDirty { window_id, message } => {
                if !self.open_force_remove(window_id, message.clone()) {
                    self.toast(message);
                }
                vec![]
            }
            DaemonMsg::Bye { reason } => {
                // Nothing is outstanding on a connection that is gone. Leaving the slot
                // set would block every later worktree removal behind a reply that can
                // never arrive.
                self.pending_worktree_remove = None;
                self.toast(format!("daemon: {reason}"));
                // Task M6.10's decision 36, sharpened by M6.11's decision 30: `Bye` is
                // the daemon's own confirmation that it is stopping, but it is not the
                // same event as the link actually closing, and `self.link` must not
                // change here. The event loop keeps reading after `Bye` and only calls
                // `on_link_lost` once the channel actually closes — the read arm used
                // to be gated on a flag this handler set `false` right here, which
                // meant the real close (and the `Quit` a pending `C-b Q` produces from
                // it) was never observed at all. Quitting *here*, before the socket has
                // actually gone, would race the daemon's own exit the same way.
                vec![]
            }
            DaemonMsg::Ack { request } => {
                self.clear_pending_worktree_remove_on(&request);
                vec![]
            }
            DaemonMsg::Git { root, state } => {
                match state {
                    Some(state) => {
                        self.git.insert(root, state);
                    }
                    None => {
                        self.git.remove(&root);
                    }
                }
                vec![]
            }
            DaemonMsg::ConversationSnapshot {
                window_id,
                agent_id,
                conversation,
            } => self
                .conversation
                .on_snapshot(window_id, agent_id, conversation),
            DaemonMsg::ConversationDelta {
                window_id,
                agent_id,
                from_rev,
                to_rev,
                turns,
                session_id,
                degraded,
                dropped_turns,
                dropped_by,
            } => self.conversation.on_delta(
                window_id,
                agent_id,
                from_rev,
                to_rev,
                turns,
                session_id,
                degraded,
                dropped_turns,
                dropped_by,
            ),
            DaemonMsg::ConversationGone {
                window_id,
                agent_id,
                reason,
            } => self.on_conversation_gone(window_id, agent_id, reason),
        }
    }
}
