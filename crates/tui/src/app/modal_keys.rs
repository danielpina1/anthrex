//! Key handling for every `Modal` variant, split out of `app.rs` per the 600-line rule
//! (`AGENTS.md` hard rule 8) — the same shape `tree.rs` gives `tree/rows.rs`.
//!
//! `App::on_key` takes the modal out with `self.modal.take()` before calling in here
//! (decision: the previous `self.modal.clone()` dropped in-place edits, which a text
//! field the user is typing into cannot afford), so every arm below is responsible for
//! putting a modal back — or not, when the key closes it.
//!
//! `Modal::Remove` and `Modal::ForceRemove` implement decisions 35 and 36 (task M5.10):
//! the remove-confirm dialog's checkbox and its dirty-tree force-or-keep follow-up.
//! Both describe what will happen to the worktree's *files* — deleted, or kept — and
//! nothing about the agent's process, because `remove_with_worktree`'s dirty check
//! (`crates/daemon/src/manager/remove.rs`) runs before the agent is signalled: on the
//! common dirty-refusal path the agent is still running when this dialog is on screen,
//! so a prompt that claimed otherwise would be wrong exactly when it matters most.

use crate::app::{App, Effect, Modal, PendingAction};
use crate::dialog::{FormContext, FormDefaults, FormOutcome, NewAgentForm, RemoveConfirm};
use crossterm::event::{KeyCode, KeyEvent};
use proto::ClientMsg;

impl App {
    /// Opens the generic yes/no `Confirm` modal for the focused window, naming it in
    /// `message` and running `action` on `y`/`Enter` (`app.rs`'s `perform`). `Modal::Remove`
    /// (decision 35) no longer goes through this — only `Command::KillWindow` still does.
    pub(super) fn confirm_focused(
        &mut self,
        verb: &str,
        make: fn(u32) -> PendingAction,
    ) -> Vec<Effect> {
        if let Some((id, name)) = self.focused_window().map(|w| (w.id, w.name.clone())) {
            self.modal = Some(Modal::Confirm {
                message: format!("{verb} '{name}'?"),
                action: make(id),
            });
        }
        vec![]
    }

    /// What the next new-agent form should open with (decision 31). Once a form has
    /// been accepted this session, `form_defaults` holds its raw values verbatim. Until
    /// then `form_defaults.dir` is still the empty sentinel `App::new` set it to, so the
    /// directory shown is computed fresh from `default_dir` and `home_dir` instead —
    /// freshly, rather than once at startup, because `home_dir` is not known yet when
    /// `App::new` runs (`tui::run` sets it only after construction).
    pub(super) fn new_agent_defaults(&self) -> FormDefaults {
        if self.form_defaults.dir.is_empty() {
            FormDefaults {
                dir: crate::ui::terminal::shorten_home_with(
                    &self.default_dir,
                    self.home_dir.as_deref(),
                ),
                ..self.form_defaults.clone()
            }
        } else {
            self.form_defaults.clone()
        }
    }

    /// Decision 35: `C-b X` opens the remove-confirm dialog directly, rather than the
    /// generic yes/no `Confirm` modal, because it carries its own checkbox. Does
    /// nothing without a focused window, the same guard `confirm_focused` uses.
    pub(super) fn open_remove_confirm(&mut self) -> Vec<Effect> {
        if let Some(w) = self.focused_window() {
            self.modal = Some(Modal::Remove(RemoveConfirm {
                window_id: w.id,
                name: w.name.clone(),
                branch: w.branch.clone(),
                remove_worktree: false,
            }));
        }
        vec![]
    }

    /// Decision 36: a `RemoveDirty` reply opens the force-or-keep follow-up in place of a
    /// toast — but only when the id the daemon sent back is the one this client has a
    /// worktree removal outstanding for. `App::on_daemon` falls back to a toast when this
    /// returns `false`, which is what a stray refusal with nothing pending gets, and what
    /// a refusal for some *other* window gets.
    ///
    /// The `==` is the whole safety property of this dialog. `f` sends a `--force`
    /// deletion of a checkout, so the id it sends must be the id the refusal is about;
    /// both come from `window_id` here, and a reply that disagrees with the pending
    /// removal is refused rather than reconciled, because at that point the client cannot
    /// tell which of the two is the truth and one of the two answers destroys work.
    ///
    /// `message` is the daemon's own text (decision 24), which names the worktree's path
    /// and what removing it would destroy — never anything about the agent's process — so
    /// it is shown exactly as given.
    pub(super) fn open_force_remove(&mut self, window_id: u32, message: String) -> bool {
        if self.pending_worktree_remove != Some(window_id) {
            return false;
        }
        let name = self
            .windows
            .iter()
            .find(|w| w.id == window_id)
            .map(|w| w.name.clone())
            .unwrap_or_default();
        self.modal = Some(Modal::ForceRemove {
            window_id,
            name,
            message,
        });
        true
    }

    /// Decision 36's last sentence: an `Ack` or an `Error` for a plain `"remove"`
    /// request is the end of whichever worktree removal `pending_worktree_remove` was
    /// tracking, successful or not. A `"remove-dirty"` reply is deliberately not one of
    /// these — it keeps the pending state alive so `f` can still find the window.
    pub(super) fn clear_pending_worktree_remove_on(&mut self, request: &str) {
        if request == proto::messages::request::REMOVE {
            self.pending_worktree_remove = None;
        }
    }

    pub(crate) fn on_modal_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        let Some(modal) = self.modal.take() else {
            return vec![];
        };
        match modal {
            // Any key closes the help overlay or the config notice; nothing to restore.
            Modal::Help | Modal::Notice { .. } => vec![],
            Modal::Confirm { message, action } => match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => self.perform(action),
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => vec![],
                _ => {
                    self.modal = Some(Modal::Confirm { message, action });
                    vec![]
                }
            },
            Modal::NewAgent(form) => self.on_new_agent_key(form, key),
            Modal::Remove(confirm) => self.on_remove_confirm_key(confirm, key),
            Modal::ForceRemove {
                window_id,
                name,
                message,
            } => self.on_force_remove_key(window_id, name, message, key),
        }
    }

    /// Decision 35. The checkbox (`Space` or `w`) only does anything when the window
    /// has a worktree to offer removing (`confirm.branch.is_some()`); a plain window's
    /// dialog has no checkbox line for it to toggle. Confirming always sends `Remove`;
    /// ticking the box is what tells the daemon to also delete the worktree directory
    /// (`remove_worktree: true`), which is the only reason this records
    /// `pending_worktree_remove` — so a later `remove-dirty` refusal knows this is the
    /// window whose files it is talking about.
    fn on_remove_confirm_key(&mut self, mut confirm: RemoveConfirm, key: KeyEvent) -> Vec<Effect> {
        match key.code {
            KeyCode::Char(' ') | KeyCode::Char('w') | KeyCode::Char('W')
                if confirm.branch.is_some() =>
            {
                confirm.remove_worktree = !confirm.remove_worktree;
                self.modal = Some(Modal::Remove(confirm));
                vec![]
            }
            KeyCode::Char('y') | KeyCode::Char('Y') | KeyCode::Enter => {
                let remove_worktree = confirm.remove_worktree;
                if remove_worktree {
                    // One worktree removal at a time (see `App::pending_worktree_remove`),
                    // for *any* window, including this same one. A second one started now
                    // — same window or different — could not be told apart from the first
                    // by the id-less `Ack`/`Error` that ends it: the daemon's answer to the
                    // second request clears the slot, and the first request's own reply
                    // (an outstanding `RemoveDirty`, if the tree turns out dirty) then lands
                    // as a toast instead of the force-or-keep dialog it should open — a
                    // second route to the same lost-prompt residual the cross-window guard
                    // below exists for, reachable with a single window. The cost of
                    // refusing is a few seconds' wait; the alternative can silently drop
                    // the one dialog standing between the user and losing uncommitted work.
                    if let Some(pending) = self.pending_worktree_remove {
                        let message = if pending == confirm.window_id {
                            format!(
                                "already removing {}'s worktree; wait for it to finish",
                                confirm.name
                            )
                        } else {
                            let name = self
                                .windows
                                .iter()
                                .find(|w| w.id == pending)
                                .map(|w| w.name.as_str())
                                .unwrap_or("another window");
                            format!("still removing {name}'s worktree; try again once it finishes")
                        };
                        self.toast(message);
                        return vec![];
                    }
                    self.pending_worktree_remove = Some(confirm.window_id);
                }
                vec![Effect::Send(ClientMsg::Remove {
                    window_id: confirm.window_id,
                    remove_worktree,
                    force: false,
                })]
            }
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => vec![],
            _ => {
                self.modal = Some(Modal::Remove(confirm));
                vec![]
            }
        }
    }

    /// Decision 36. `f` forces the removal through, deleting the worktree directory and
    /// whatever uncommitted work is in it; `k` removes only the window and leaves the
    /// directory on disk; `n`/`Esc` cancels and leaves both the window and the files
    /// untouched. `Enter` is deliberately not a synonym for either choice, so a
    /// reflexive keystroke can never discard files.
    fn on_force_remove_key(
        &mut self,
        window_id: u32,
        name: String,
        message: String,
        key: KeyEvent,
    ) -> Vec<Effect> {
        match key.code {
            KeyCode::Char('f') | KeyCode::Char('F') => vec![Effect::Send(ClientMsg::Remove {
                window_id,
                remove_worktree: true,
                force: true,
            })],
            KeyCode::Char('k') | KeyCode::Char('K') => vec![Effect::Send(ClientMsg::Remove {
                window_id,
                remove_worktree: false,
                force: false,
            })],
            // Cancelling ends the removal this client was tracking: the refusal was its
            // final reply, so nothing is outstanding any more. Leaving the slot set would
            // make the *next* worktree removal refuse itself for ever.
            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                self.pending_worktree_remove = None;
                vec![]
            }
            _ => {
                self.modal = Some(Modal::ForceRemove {
                    window_id,
                    name,
                    message,
                });
                vec![]
            }
        }
    }

    fn on_new_agent_key(&mut self, mut form: NewAgentForm, key: KeyEvent) -> Vec<Effect> {
        let existing_names: Vec<&str> = self.windows.iter().map(|w| w.name.as_str()).collect();
        let ctx = FormContext {
            default_dir: &self.default_dir,
            home: self.home_dir.as_deref(),
            existing_names: &existing_names,
        };
        match form.on_key(key, &ctx) {
            FormOutcome::Cancel => vec![],
            FormOutcome::Stay => {
                self.modal = Some(Modal::NewAgent(form));
                vec![]
            }
            FormOutcome::Submit(spec) => {
                let (cols, rows) = self.term_size;
                self.modal = Some(Modal::NewAgent(form));
                vec![Effect::Send(ClientMsg::CreateWindow {
                    spec,
                    cols: cols.max(1),
                    rows: rows.max(1),
                })]
            }
        }
    }
}
