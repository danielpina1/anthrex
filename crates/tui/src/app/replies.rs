//! Milestone 9.0.6 decisions 16, 19 and 20: the replies this client waits on, by
//! `request_id`. `route_reply` is the one dispatch by id, called first by
//! `on_run_reply`; later tasks add their arms here, not in `runs.rs` (preflight F29).
//! Pure: the clock is read only to stamp and expire an entry, as the toasts do.

use super::actions::{ActionStep, MovedBasePage};
use super::profile_screen::ProfileAsk;
use super::runs::{capped, first_line_and_more};
use super::{App, Effect, Modal, ToastLevel};
use crate::actions_request::ActionTarget;
use proto::{ActionKind, BaseMovedInfo, PlanEdit, ProfileRequest, RunReply, RunRequest};
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
            ProfileRequest::Detect { .. }
            | ProfileRequest::Edit { .. }
            | ProfileRequest::Confirm { .. },
        ) => LONG_REPLY_TIMEOUT,
        _ => REPLY_TIMEOUT,
    }
}

/// Decision 16: the error toast of a request that got no reply in time.
pub const NO_REPLY: &str = "no reply from daemon";
/// What a screen or form waiting on a request shows when the link went while it waited.
pub const NOT_CONNECTED: &str = "not connected";
/// What a screen or form shows when its request could not be sent while connected.
pub const NOT_SENT: &str = "not sent: daemon is not responding";

/// How many expired Profile and stats view ids `PendingReplies` remembers.
const QUIET_KEPT: usize = 32;

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
        ask: ProfileAsk,
    },
    /// The stats screen's `Stats { dir }` (decision 38, `app/stats.rs`).
    Stats { dir: std::path::PathBuf },
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
    /// The last few expired views (a Profile `Status` or `Show`, a `Stats`), with what
    /// each was: a late reply is never toasted, since it only describes; the screen
    /// still loading on it takes it (decision 16).
    quiet: Vec<(u64, PendingWhat)>,
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

    /// Whether a request of this kind is waiting.
    pub fn waits_for(&self, what: &PendingWhat) -> bool {
        self.by_id.values().any(|p| p.what == *what)
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
        let quiet = &mut self.quiet;
        self.by_id.retain(|id, p| {
            let live = now.saturating_duration_since(p.sent_at) < p.timeout;
            if !live {
                let view = match &p.what {
                    PendingWhat::Profile { ask, .. } => {
                        matches!(ask, ProfileAsk::Status | ProfileAsk::Show { .. })
                    }
                    PendingWhat::Stats { .. } => true,
                    _ => false,
                };
                if view {
                    quiet.push((*id, p.what.clone()));
                }
                gone.push(p.what.clone());
            }
            live
        });
        let excess = self.quiet.len().saturating_sub(QUIET_KEPT);
        self.quiet.drain(..excess);
        gone
    }

    /// Whether `id` was an expired view, whose late reply is not toasted.
    pub fn expired_quietly(&self, id: u64) -> bool {
        self.expired_view(id).is_some()
    }

    /// What the expired view `id` asked, while it is remembered.
    pub fn expired_view(&self, id: u64) -> Option<&PendingWhat> {
        self.quiet
            .iter()
            .find(|(q, _)| *q == id)
            .map(|(_, what)| what)
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
        if let Some(effects) = self.route_stats_reply(reply) {
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
        let screens = self.settings_saving();
        let stats = self.stats_awaited();
        let gone = self.replies.expire(Instant::now());
        if gone.is_empty() {
            return vec![];
        }
        // The open Settings screen shows its own save's expiry (`settings_flow.rs`), the
        // stats screen its request's (`stats.rs`), the Profile screen its views' and the
        // answer form its brief's (final review): no toast for any of them.
        let mut owned = usize::from(screens && !self.settings_saving())
            + usize::from(stats.is_some() && self.stats_awaited().is_none());
        self.stats_tick();
        for what in &gone {
            owned += usize::from(self.view_failed(what, NO_REPLY));
        }
        if gone.len() > owned {
            self.toast_at(ToastLevel::Error, NO_REPLY);
        }
        // A save that went unanswered may still land: ask for the settings again, so a
        // late `Saved` does not leave the cache stale.
        if gone.contains(&PendingWhat::SettingsPut) {
            self.settings_screen_no_reply();
            return vec![self.settings_fetch()];
        }
        vec![]
    }

    /// A form or screen still waiting on `what` shows `why` instead; `true` when one did
    /// (the request was theirs, so no toast).
    fn view_failed(&mut self, what: &PendingWhat, why: &str) -> bool {
        match what {
            PendingWhat::FormBrief { run_id, task_id } => {
                self.fail_form_brief(run_id, task_id, why)
            }
            PendingWhat::Profile { dir, ask } => self.profile_view_failed(dir, *ask, why),
            _ => false,
        }
    }

    /// `on_send_failed`'s tagged share (final review I1): the request never left, so its
    /// entry goes at once and no `no reply from daemon` follows. A screen or form that
    /// owns it says so itself; `true` when the usual not-sent toast is its only feedback.
    pub(super) fn tagged_not_sent(&mut self, id: u64, request: &RunRequest) -> bool {
        let stats_own = self.stats_awaited() == Some(id);
        let put_own = self.settings_put_was_screens(id);
        let pending = self.replies.take(Some(id)).map(|p| p.what);
        let why = if self.connected() {
            NOT_SENT
        } else {
            NOT_CONNECTED
        };
        match request {
            // Milestone 9.0.5 decision 23: quiet; the panel says the detail was not
            // sent, and the task's next key asks again (never a retry per tick).
            RunRequest::TaskDetail { .. } => {
                self.task_detail_not_sent(id);
                if let Some(what) = &pending {
                    self.view_failed(what, why);
                }
                false
            }
            // Decision 34: a Profile view is the screen's; it asks again on its tick.
            RunRequest::Profile(ProfileRequest::Status { dir }) => {
                self.profile_view_failed(dir, ProfileAsk::Status, why);
                false
            }
            RunRequest::Profile(ProfileRequest::Show { dir, proposed }) => {
                let ask = ProfileAsk::Show {
                    proposed: *proposed,
                };
                self.profile_view_failed(dir, ask, why);
                false
            }
            // Decision 24: quiet; the cache stays as it was until the next connection.
            RunRequest::Settings(proto::SettingsRequest::Get) => false,
            RunRequest::Settings(proto::SettingsRequest::Put { .. }) if put_own => {
                self.settings_put_not_sent();
                false
            }
            // Decision 38: its screen says so at once.
            RunRequest::Stats { .. } => {
                self.stats_not_sent(id);
                !stats_own
            }
            // Whole-branch review M2: a refused `Edit` frees its submitting form;
            // milestone 9: the form's own tagged request, and the goal form's.
            RunRequest::Edit { .. } => {
                self.edit_not_sent(Some(id));
                true
            }
            RunRequest::StartGoal { .. } => {
                self.goal_not_sent(Some(id));
                true
            }
            _ => true,
        }
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
