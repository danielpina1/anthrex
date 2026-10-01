//! The Settings screen's requests and replies (decision 36), split from
//! `settings_screen.rs` by responsibility (`AGENTS.md` hard rule 8): opening it from the
//! cache, `w`'s tagged `Settings(Put)`, and what the screen does with its own save's
//! reply, a new cache, an expiry and a lost link. While its `Put` is awaited the screen
//! owns the save's feedback: `route_settings_reply` and `expire_replies` toast only for
//! a save that is not the open screen's. Pure: every request leaves as an `Effect`.

use super::{NO_CHANGES, SaveOutcome, SettingsScreen};
use crate::app::replies::PendingWhat;
use crate::app::screens::Screen;
use crate::app::{App, Effect, ToastLevel};
use proto::SettingsReply;

impl App {
    /// `C-b S` (decision 36): the screen from the cache, or a loading screen and one
    /// `Settings(Get)` when there is no cache yet (and none is on its way). Refused over
    /// the plan review (decision 33); already open, it stays as it is.
    pub(in crate::app) fn open_settings(&mut self) -> Vec<Effect> {
        if self.plan_review.is_some() {
            self.toast(crate::app::plan_review::LEAVE_REVIEW_FIRST);
            return vec![];
        }
        if matches!(self.screen, Some(Screen::Settings(_))) {
            return vec![];
        }
        let (screen, effects) = match &self.settings_cache {
            Some(cache) => {
                let mut s = SettingsScreen::from_doc(&cache.doc, &cache.origin);
                s.path = cache.path.display().to_string();
                (s, vec![])
            }
            None if self.replies.waits_for(&PendingWhat::SettingsGet) => {
                (SettingsScreen::loading(), vec![])
            }
            None => (SettingsScreen::loading(), vec![self.settings_fetch()]),
        };
        self.set_screen(Some(Screen::Settings(Box::new(screen))));
        effects
    }

    pub(crate) fn settings_screen_mut(&mut self) -> Option<&mut SettingsScreen> {
        match &mut self.screen {
            Some(Screen::Settings(s)) => Some(s),
            _ => None,
        }
    }

    /// The open screen's `Put` id while its reply is awaited.
    fn screen_put(&self) -> Option<u64> {
        match &self.screen {
            Some(Screen::Settings(s)) => s.put_id.filter(|id| self.replies.contains(*id)),
            _ => None,
        }
    }

    /// The screen's `Put` is still awaited: the screen shows `saving…`. Derived from
    /// `App.replies`, so a lost link, a refused send or an expiry never leaves it stale.
    pub fn settings_saving(&self) -> bool {
        self.screen_put().is_some()
    }

    /// Whether `id` is the open screen's own awaited `Put`: the screen shows its outcome.
    pub(crate) fn settings_put_is_screens(&self, id: u64) -> bool {
        self.screen_put() == Some(id)
    }

    /// `w`: the screen's doc as a tagged `Settings(Put)`, unless nothing changed
    /// (`no changes to save`), a problem blocks it (listed inline already), a save is
    /// under way, or the link is down. Editing stays allowed while it is in flight.
    pub(in crate::app) fn settings_save(&mut self) -> Vec<Effect> {
        if self.settings_saving() {
            return vec![];
        }
        let Some(s) = self.settings_screen_mut().filter(|s| s.loaded) else {
            return vec![];
        };
        if !s.dirty() {
            self.toast_at(ToastLevel::Info, NO_CHANGES);
            return vec![];
        }
        let Ok(doc) = s.doc() else {
            return vec![];
        };
        if !self.connected() {
            self.toast_at(ToastLevel::Warn, "not connected");
            return vec![];
        }
        let effect = self.settings_put(doc.clone());
        if let (Some(s), Effect::Send(proto::ClientMsg::RunTagged { id, .. })) =
            (self.settings_screen_mut(), &effect)
        {
            s.put_id = Some(*id);
            s.sent = Some(doc);
            s.outcome = None;
        }
        vec![effect]
    }

    /// `set_cache`'s share: a loading screen fills from the new cache, an unchanged one
    /// follows it, and one whose edits are exactly the cache (a save that landed while
    /// its reply was lost) takes it; other unsaved changes are never replaced.
    pub(in crate::app) fn sync_settings_screen(&mut self) {
        let saving = self.settings_saving();
        let Some(cache) = self.settings_cache.clone() else {
            return;
        };
        let Some(s) = self.settings_screen_mut() else {
            return;
        };
        let landed = s.loaded && s.built() == cache.doc;
        if !s.loaded || landed || (!s.dirty() && !saving) {
            s.load(&cache.doc, &cache.origin);
            s.path = cache.path.display().to_string();
            if s.outcome == Some(SaveOutcome::LinkLost) && landed {
                s.outcome = None;
            }
        }
    }

    /// `route_settings_reply`'s share: the outcome of the screen's own `Put`. `Saved`
    /// reloads the screen on the saved doc, or, when the user edited on while it was in
    /// flight, rebases on it and keeps those edits; anything else lists what the daemon
    /// said.
    pub(in crate::app) fn settings_screen_reply(&mut self, id: u64, reply: &SettingsReply) {
        let Some(s) = self.settings_screen_mut().filter(|s| s.put_id == Some(id)) else {
            return;
        };
        s.put_id = None;
        let sent = s.sent.take();
        match reply {
            SettingsReply::Saved { doc, origin } => {
                if sent.as_ref() == Some(&s.built()) {
                    s.load(doc, origin);
                } else {
                    s.base = doc.clone();
                    s.origin = origin.clone();
                }
                s.outcome = Some(SaveOutcome::Saved);
            }
            SettingsReply::Refused { problems } => {
                s.outcome = Some(SaveOutcome::Refused(problems.clone()));
            }
            SettingsReply::Current { .. } => {}
        }
    }

    /// `expire_replies`' share: the screen's `Put` went unanswered.
    pub(in crate::app) fn settings_screen_no_reply(&mut self) {
        let lost = match &self.screen {
            Some(Screen::Settings(s)) => s.put_id.is_some_and(|id| !self.replies.contains(id)),
            _ => false,
        };
        if lost && let Some(s) = self.settings_screen_mut() {
            s.put_id = None;
            s.sent = None;
            s.outcome = Some(SaveOutcome::Refused(vec!["no reply from daemon".into()]));
        }
    }

    /// `screens_tick`: a `Put` that is no longer awaited and got no reply (the link was
    /// lost, or its send refused) shows `not saved: link lost` instead of `saving…`.
    pub(in crate::app) fn settings_tick(&mut self) -> Vec<Effect> {
        let gone = match &self.screen {
            Some(Screen::Settings(s)) => s.put_id.is_some() && self.screen_put().is_none(),
            _ => false,
        };
        if gone && let Some(s) = self.settings_screen_mut() {
            s.put_id = None;
            s.sent = None;
            s.outcome = Some(SaveOutcome::LinkLost);
        }
        vec![]
    }
}
