//! The Profile screen's view requests over time (decision 34, final review minor 2):
//! the 1 s tick that polls the status while a detection runs and asks again for a view
//! no reply will come for, the failure a view shows when its request expired or was not
//! sent, and whether a late view still fills the screen. Split from
//! `app/profile_screen.rs` by responsibility (`AGENTS.md` hard rule 8). Pure: the clock
//! is the tick's `now`.

use super::{POLL_EVERY, ProfileAsk, Side};
use crate::app::replies::PendingWhat;
use crate::app::screens::Screen;
use crate::app::{App, Effect};
use std::time::Instant;

impl App {
    /// Decision 34's poll: while a detection runs, one `Status` a second, never two in
    /// flight. Nothing once the screen is closed or the proposal is past verifying.
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
            && (s.in_progress() || (s.status.is_none() && s.status_failed.is_some()));
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

    /// No reply will come for the screen's view `ask` on `dir` (expired, or not sent):
    /// a side or a status still loading says `why`. `true` when the screen is open on
    /// `dir` (the view was the screen's, so no toast).
    pub(in crate::app) fn profile_view_failed(
        &mut self,
        dir: &std::path::Path,
        ask: ProfileAsk,
        why: &str,
    ) -> bool {
        let Some(s) = self.profile_screen_mut().filter(|s| s.dir == dir) else {
            return false;
        };
        match ask {
            ProfileAsk::Show { proposed } => {
                let side = s.side_mut(proposed);
                if !matches!(side, Side::Ready(_)) {
                    *side = Side::Failed(why.into());
                }
            }
            ProfileAsk::Status if s.status.is_none() => s.status_failed = Some(why.into()),
            ProfileAsk::Status => {}
            _ => return false,
        }
        true
    }

    /// Decision 16: whether a late `ask` on `dir` still fills the open screen (its side,
    /// or its status, never came).
    pub(super) fn profile_awaits(&self, dir: &std::path::Path, ask: ProfileAsk) -> bool {
        match &self.screen {
            Some(Screen::Profile(s)) if s.dir == dir => match ask {
                ProfileAsk::Show { proposed } => {
                    let side = if proposed { &s.proposal } else { &s.stored };
                    !matches!(side, Side::Ready(_))
                }
                ProfileAsk::Status => s.status.is_none(),
                _ => false,
            },
            _ => false,
        }
    }
}
