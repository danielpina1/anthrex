//! Milestone 9.0.6 decisions 16, 19 and 20: the replies this client waits on, by
//! `request_id`. `route_reply` is the one dispatch by id, called first by
//! `on_run_reply`; later tasks add their arms here, not in `runs.rs` (preflight F29).
//! Pure: the clock is read only to stamp and expire an entry, as the toasts do.

use super::actions::{ActionStep, MovedBasePage};
use super::runs::{capped, first_line_and_more};
use super::{App, Effect, Modal, ToastLevel};
use crate::actions_request::ActionTarget;
use proto::{ActionKind, BaseMovedInfo, PlanEdit, RunReply, RunRequest};
use std::collections::BTreeMap;
use std::time::{Duration, Instant};

/// Decision 19: requests answered in one engine step or one bounded file write.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(30);
/// Decision 19: requests that wait on git: the largest `git_timeout_secs` the config
/// allows (600) plus 60 s.
pub const LONG_REPLY_TIMEOUT: Duration = Duration::from_secs(660);

/// Decision 19's split: how long the user waits for `request`'s reply.
pub fn reply_timeout(request: &RunRequest) -> Duration {
    match request {
        RunRequest::Finish { .. }
        | RunRequest::Resume { .. }
        | RunRequest::Override { .. }
        | RunRequest::Promote { .. } => LONG_REPLY_TIMEOUT,
        RunRequest::Edit { edits, .. }
            if edits.iter().any(|e| matches!(e, PlanEdit::Refresh { .. })) =>
        {
            LONG_REPLY_TIMEOUT
        }
        RunRequest::Profile(
            proto::ProfileRequest::Detect { .. }
            | proto::ProfileRequest::Edit { .. }
            | proto::ProfileRequest::Confirm { .. },
        ) => LONG_REPLY_TIMEOUT,
        _ => REPLY_TIMEOUT,
    }
}

/// What a pending request was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingWhat {
    /// An action menu entry's request on `target` of `run_id`.
    Action {
        run_id: String,
        target: ActionTarget,
        kind: ActionKind,
    },
    /// The answer form's `TaskDetail` (decision 15): fills its brief rows.
    FormBrief { run_id: String, task_id: String },
    /// The connection's `Settings(Get)` (decision 24, `app/screens.rs`).
    SettingsGet,
    /// A `Settings(Put)`; its `Saved` replaces the cache, any other reply re-syncs it.
    SettingsPut,
    /// A Profile screen request on `dir` (decision 34, `app/profile_screen.rs`).
    Profile {
        dir: std::path::PathBuf,
        ask: super::profile_screen::ProfileAsk,
    },
}

/// One request waiting for its reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pending {
    pub what: PendingWhat,
    pub sent_at: Instant,
    pub timeout: Duration,
}

/// Every request this client waits on, by its `request_id`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PendingReplies {
    by_id: BTreeMap<u64, Pending>,
}

impl PendingReplies {
    pub fn insert(&mut self, id: u64, what: PendingWhat, timeout: Duration) {
        let sent_at = Instant::now();
        self.by_id.insert(
            id,
            Pending {
                what,
                sent_at,
                timeout,
            },
        );
    }

    pub fn peek(&self, id: u64) -> Option<&PendingWhat> {
        self.by_id.get(&id).map(|p| &p.what)
    }

    pub fn contains(&self, id: u64) -> bool {
        self.by_id.contains_key(&id)
    }

    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// The entry for `id`, removed: its reply came.
    pub fn take(&mut self, id: Option<u64>) -> Option<Pending> {
        self.by_id.remove(&id?)
    }

    /// Decision 16: a lost link drops every entry.
    pub fn clear(&mut self) {
        self.by_id.clear();
    }

    /// Drops every entry past its timeout at `now`; what each one was.
    pub fn expire(&mut self, now: Instant) -> Vec<PendingWhat> {
        let mut gone = Vec::new();
        self.by_id.retain(|_, p| {
            let live = now.saturating_duration_since(p.sent_at) < p.timeout;
            if !live {
                gone.push(p.what.clone());
            }
            live
        });
        gone
    }
}

impl App {
    /// Decision 16: the reply to a pending request, by its id. `Some` when it was ours
    /// and is handled; `None` leaves it to `on_run_reply`'s own arms (a late reply
    /// among them, which is still shown).
    pub(super) fn route_reply(&mut self, reply: &RunReply) -> Option<Vec<Effect>> {
        if let Some(effects) = self.route_form_reply(reply) {
            return Some(effects);
        }
        if let Some(effects) = self.route_settings_reply(reply) {
            return Some(effects);
        }
        if let Some(effects) = self.route_profile_reply(reply) {
            return Some(effects);
        }
        match reply {
            RunReply::Done {
                message,
                request_id,
                ..
            } => {
                self.replies.take(*request_id)?;
                self.toast_at(ToastLevel::Info, capped(message));
            }
            RunReply::Refused {
                request,
                message,
                request_id,
            } => {
                self.replies.take(*request_id)?;
                let text =
                    first_line_and_more(message).unwrap_or_else(|| format!("{request} refused"));
                self.toast_at(ToastLevel::Error, text);
            }
            RunReply::ConfirmNeeded {
                run_id,
                prompt,
                base_moved,
                request_id,
            } => {
                let pending = self.replies.take(*request_id);
                let accept = matches!(
                    pending.as_ref().map(|p| &p.what),
                    Some(PendingWhat::Action {
                        kind: ActionKind::Accept,
                        ..
                    })
                );
                match base_moved {
                    Some(moved) if accept && self.open_moved_base(run_id, moved) => {}
                    // Decision 16: any other `ConfirmNeeded` of ours, or a late one,
                    // is a warning of its prompt.
                    _ if pending.is_some() || request_id.is_some() => {
                        self.toast_at(ToastLevel::Warn, capped(prompt));
                    }
                    _ => return None,
                }
            }
            _ => return None,
        }
        Some(vec![])
    }

    /// Decision 18: the moved-base page of `run_id`'s accept, over an open menu of the
    /// run or on a fresh one. `false` (a toast instead) when another dialog is open or
    /// the run no longer lists `Accept`.
    fn open_moved_base(&mut self, run_id: &str, moved: &BaseMovedInfo) -> bool {
        let opened = match &self.modal {
            None => {
                self.open_actions(
                    (run_id.to_string(), ActionTarget::Run),
                    Some(ActionKind::Accept),
                );
                true
            }
            Some(Modal::Action(flow))
                if flow.run_id == run_id && flow.target == ActionTarget::Run =>
            {
                false
            }
            Some(_) => return false,
        };
        let Some(Modal::Action(flow)) = self.modal.as_mut() else {
            return false;
        };
        let Some(info) = flow
            .items
            .iter()
            .find(|a| a.kind == ActionKind::Accept)
            .cloned()
        else {
            if opened {
                self.modal = None;
            }
            return false;
        };
        flow.step = ActionStep::MovedBase(MovedBasePage {
            info,
            moved: moved.clone(),
            typed: String::new(),
            wrong: false,
        });
        true
    }

    /// `on_tick` (decision 19): a pending request with no reply within its timeout is
    /// dropped with an error toast, and changes no state.
    pub(super) fn expire_replies(&mut self) -> Vec<Effect> {
        let gone = self.replies.expire(Instant::now());
        if gone.is_empty() {
            return vec![];
        }
        self.toast_at(ToastLevel::Error, "no reply from daemon");
        // A save that went unanswered may still land: ask for the settings again, so a
        // late `Saved` does not leave the cache stale.
        if gone.contains(&PendingWhat::SettingsPut) {
            return vec![self.settings_fetch()];
        }
        vec![]
    }

    /// Tests move a request's sending into the past instead of sleeping (Global
    /// Constraint 11).
    #[cfg(test)]
    pub(crate) fn set_reply_sent_at(&mut self, id: u64, at: Instant) {
        if let Some(pending) = self.replies.by_id.get_mut(&id) {
            pending.sent_at = at;
        }
    }
}
