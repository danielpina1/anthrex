//! The Profile screen's view requests over time (decision 34, final review minor 2):
//! the 1 s tick that polls the status while a detection runs and asks again for a view
//! no reply will come for, the failure a view shows when its request expired or was not
//! sent, and whether a late view still fills the screen; and the replies that fill the
//! views (moved here from `app/profile_screen.rs` in milestone 9.10.8, rule 8). Split from
//! `app/profile_screen.rs` by responsibility (`AGENTS.md` hard rule 8). Pure: the clock
//! is the tick's `now`.

use super::{POLL_EVERY, ProfileAsk, Shown, Side};
use crate::app::replies::PendingWhat;
use crate::app::runs::{capped, first_line_and_more};
use crate::app::screens::Screen;
use crate::app::{App, Effect, ToastLevel};
use proto::{ProfileReply, RunReply};
use std::time::Instant;

impl App {
    /// Decision 34's poll: while a detection runs, or a row edit is checked (milestone
    /// 9.10 decision 31), one `Status` a second, never two in flight. Nothing once the screen is closed or the proposal is past verifying.
    /// Minor 2: a view no reply will come for (`status_failed`, a `Failed` side) is asked
    /// again the same way, once a second and one at a time.
    pub(in crate::app) fn profile_tick(&mut self, now: Instant) -> Vec<Effect> {
        let Some(Screen::Profile(s)) = &self.screen else {
            return vec![];
        };
        if !self.connected() {
            return vec![];
        }
        let due = |at: Option<Instant>| {
            at.is_none_or(|at| now.saturating_duration_since(at) >= POLL_EVERY)
        };
        let status = !s.status_id.is_some_and(|id| self.replies.contains(id))
            && due(s.status_sent_at)
            && (s.in_progress()
                || s.row_checking()
                || (s.status.is_none() && s.status_failed.is_some()));
        let sides: Vec<bool> = [false, true]
            .into_iter()
            .filter(|&proposed| {
                let side = if proposed { &s.proposal } else { &s.stored };
                let what = PendingWhat::Profile {
                    dir: s.dir.clone(),
                    ask: ProfileAsk::Show { proposed },
                };
                matches!(side, Side::Failed(_))
                    && due(s.show_sent_at[usize::from(proposed)])
                    && !self.replies.waits_for(&what)
            })
            .collect();
        let mut effects = if status {
            self.profile_status(now)
        } else {
            vec![]
        };
        for proposed in sides {
            effects.extend(self.profile_show(proposed, now));
        }
        effects
    }

    /// No reply will come for the screen's view `ask` on `dir` (request `id`, expired or
    /// not sent): a side or a status still loading says `why`; a side only for its
    /// latest `Show` (9.0.7 decision 37: an older one's expiry leaves a re-fetch
    /// loading). `true` when the screen is open on `dir` (the view was the screen's, so
    /// no toast).
    pub(in crate::app) fn profile_view_failed(
        &mut self,
        id: u64,
        dir: &std::path::Path,
        ask: ProfileAsk,
        why: &str,
    ) -> bool {
        let Some(s) = self.profile_screen_mut().filter(|s| s.dir == dir) else {
            return false;
        };
        match ask {
            ProfileAsk::Show { proposed } => {
                let latest = s.show_id[usize::from(proposed)] == Some(id);
                let side = s.side_mut(proposed);
                if latest && !matches!(side, Side::Ready(_)) {
                    *side = Side::Failed(why.into());
                }
            }
            ProfileAsk::Status if s.status.is_none() => s.status_failed = Some(why.into()),
            ProfileAsk::Status => {}
            _ => return false,
        }
        true
    }

    /// Decision 16: whether a late `ask` (request `id`) on `dir` still fills the open
    /// screen (its side, or its status, never came). A `Show` fills its side only when
    /// it is the side's latest request (9.0.7 decision 37): one sent before a detection
    /// is dropped while the re-fetch is out.
    pub(super) fn profile_awaits(&self, dir: &std::path::Path, ask: ProfileAsk, id: u64) -> bool {
        match &self.screen {
            Some(Screen::Profile(s)) if s.dir == dir => match ask {
                ProfileAsk::Show { proposed } => {
                    let side = if proposed { &s.proposal } else { &s.stored };
                    !matches!(side, Side::Ready(_)) && s.show_id[usize::from(proposed)] == Some(id)
                }
                ProfileAsk::Status => s.status.is_none(),
                _ => false,
            },
            _ => false,
        }
    }

    /// Whether `id` is the open screen's latest `Show` of the side `proposed`; a closed
    /// screen has none, and its reply is dropped by the caller's directory check.
    pub(super) fn profile_latest_show(&self, proposed: bool, id: Option<u64>) -> bool {
        match &self.screen {
            Some(Screen::Profile(s)) => id.is_some() && s.show_id[usize::from(proposed)] == id,
            _ => true,
        }
    }

    /// `route_reply`'s profile arm. A reply to a profile request of ours applies to the
    /// screen still open on its project; one whose screen closed, or a late one, is
    /// shown as a toast when it reports an outcome (decision 16), and dropped when it is
    /// a view.
    pub(in crate::app) fn route_profile_reply(&mut self, reply: &RunReply) -> Option<Vec<Effect>> {
        let RunReply::Profile { reply, request_id } = reply else {
            return None;
        };
        let pending = request_id.and_then(|id| self.replies.peek(id).cloned());
        let (dir, ask) = match pending {
            Some(PendingWhat::Profile { dir, ask }) => (dir, ask),
            Some(_) => return None,
            None => {
                // A late view (its entry expired) fills the open screen still loading
                // on it, else is dropped (decision 16, minor 2); a late outcome shown.
                let late = request_id.and_then(|id| self.replies.expired_view(id).cloned());
                match late.zip(*request_id) {
                    Some((PendingWhat::Profile { dir, ask }, id))
                        if self.profile_awaits(&dir, ask, id) =>
                    {
                        return Some(self.apply_profile_reply(ask, reply));
                    }
                    Some(_) => {}
                    None if request_id.is_some() => self.toast_profile_outcome(reply),
                    None => {}
                }
                return Some(vec![]);
            }
        };
        self.replies.take(*request_id);
        // Decision 37: a `Show` that is no longer its side's latest (a re-fetch went
        // out after it) describes an older state; the re-fetch's reply fills the side.
        if let ProfileAsk::Show { proposed } = ask
            && !self.profile_latest_show(proposed, *request_id)
        {
            return Some(vec![]);
        }
        if self.profile_dir().as_ref() != Some(&dir) {
            if !matches!(ask, ProfileAsk::Status | ProfileAsk::Show { .. }) {
                self.toast_profile_outcome(reply);
            }
            return Some(vec![]);
        }
        Some(self.apply_profile_reply(ask, reply))
    }

    fn toast_profile_outcome(&mut self, reply: &ProfileReply) {
        match reply {
            ProfileReply::Done { message } => self.toast_at(ToastLevel::Info, capped(message)),
            ProfileReply::Refused { message } => {
                let text = first_line_and_more(message).unwrap_or_else(|| "profile refused".into());
                self.toast_at(ToastLevel::Error, text);
            }
            _ => {}
        }
    }

    fn apply_profile_reply(&mut self, ask: ProfileAsk, reply: &ProfileReply) -> Vec<Effect> {
        let Some(s) = self.profile_screen_mut() else {
            return vec![];
        };
        match (ask, reply) {
            (ProfileAsk::Status, ProfileReply::Status(status)) => {
                let was_running = s.in_progress();
                // A row edit whose check ends changed its side (decision 15): the stored
                // profile for an `Edit`-origin record, else the review proposal.
                let checked = s.row_checking().then(|| !s.review_proposal());
                s.status = Some(status.clone());
                s.status_failed = None;
                if status.proposal.is_none() {
                    s.proposal = Side::Absent("no proposal".into());
                }
                let mut sides = Vec::new();
                // Decision 34: leaving the running states (for `Ready`, or `Failed`:
                // minor 1) fetches the proposal once; the old one is not shown meanwhile.
                if was_running && !s.in_progress() && status.proposal.is_some() {
                    s.proposal = Side::Loading;
                    sides.push(true);
                }
                match checked.filter(|_| !s.row_checking()) {
                    Some(true) => sides.push(false),
                    Some(false) if !sides.contains(&true) && status.proposal.is_some() => {
                        sides.push(true)
                    }
                    _ => {}
                }
                s.note_saved();
                let now = Instant::now();
                sides
                    .into_iter()
                    .flat_map(|proposed| self.profile_show(proposed, now))
                    .collect()
            }
            (
                ProfileAsk::Show { proposed },
                ProfileReply::Shown {
                    toml,
                    verification,
                    dropped,
                    ..
                },
            ) => {
                let shown = Shown {
                    toml: toml.clone(),
                    profile: toml::from_str(toml).ok(),
                    verification: verification.clone(),
                    dropped: dropped.clone(),
                };
                *s.side_mut(proposed) = Side::Ready(Box::new(shown));
                s.note_saved();
                vec![]
            }
            (ProfileAsk::Show { proposed }, ProfileReply::Refused { message }) => {
                *s.side_mut(proposed) = Side::Absent(message.clone());
                vec![]
            }
            (_, ProfileReply::Refused { message }) => {
                // A refused `Status` is not asked again (minor 2's retry is for silence).
                s.status_failed = None;
                s.error = Some(message.clone());
                s.message = None;
                s.saving = None;
                vec![]
            }
            (ProfileAsk::Detect, ProfileReply::Done { message }) => {
                s.message = Some(message.clone());
                s.error = None;
                self.profile_status(Instant::now())
            }
            (_, ProfileReply::Done { message }) => {
                s.message = Some(message.clone());
                s.error = None;
                if ask != ProfileAsk::Edit {
                    s.saving = None;
                }
                self.profile_fetch_all(Instant::now())
            }
            _ => vec![],
        }
    }
}
