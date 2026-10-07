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
use proto::{RunReply, RunRequest, SettingsReply};

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
            .into_iter()
            .chain(self.settings_tuning_ask())
            .collect()
    }

    /// Milestone 9.5 decision 48: with a project, one tagged read-only `Stats` for its
    /// tuning (no revert recorded, no file written), awaited by the open screen, which
    /// draws no note until it answers. Asked on opening and again after the screen's
    /// own save, which can make a class's budget explicit (decision 3).
    pub(in crate::app) fn settings_tuning_ask(&mut self) -> Option<Effect> {
        let dir = self.goal_project()?;
        let request = RunRequest::Stats {
            dir: dir.clone(),
            apply: Vec::new(),
            dismiss: Vec::new(),
            read_only: true,
        };
        let timeout = crate::app::replies::reply_timeout(&request);
        let (id, effect) = self.tagged_request(request);
        self.replies.insert(id, PendingWhat::Stats { dir }, timeout);
        let s = self.settings_screen_mut()?;
        s.tuning_request = Some(id);
        s.tuning = None;
        Some(effect)
    }

    /// The reply to the open screen's read-only `Stats`: its tuning, or (refused)
    /// nothing drawn. `None` when the reply is not that request's.
    pub(in crate::app) fn route_settings_tuning(
        &mut self,
        reply: &RunReply,
    ) -> Option<Vec<Effect>> {
        let (id, tuning) = match reply {
            RunReply::Stats { stats, request_id } => (*request_id, stats.tuning.clone()),
            RunReply::Refused { request_id, .. } => (*request_id, None),
            _ => return None,
        };
        let s = self
            .settings_screen_mut()
            .filter(|s| s.tuning_request == id && id.is_some())?;
        s.tuning_request = None;
        s.tuning = tuning;
        self.replies.take(id);
        Some(vec![])
    }

    /// Whether `id` is the open screen's read-only `Stats` (its send failing is quiet).
    pub(in crate::app) fn settings_tuning_is(&self, id: u64) -> bool {
        matches!(&self.screen, Some(Screen::Settings(s)) if s.tuning_request == Some(id))
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
        // A save stores its doc cleaned (`config::settings::cleaned`), so a landed save
        // is known by its cleaned form (ruling M9.2.15, carried).
        let landed = s.loaded && config::settings::cleaned(&s.built()) == cache.doc;
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
                    // Ruling (M9.2.15, carried): the screen adopts the saved doc, which
                    // the daemon cleaned; the edits made since the send stay on top,
                    // cleaned as the next save will store them.
                    s.base = doc.clone();
                    s.origin = origin.clone();
                    drop_hidden(s);
                }
                s.outcome = Some(SaveOutcome::Saved);
            }
            SettingsReply::Refused { problems } => {
                s.outcome = Some(SaveOutcome::Refused(problems.clone()));
            }
            SettingsReply::Current { .. }
            | SettingsReply::RepoModels { .. }
            | SettingsReply::RepoSaved { .. } => {}
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

    /// Whether `id` is the open screen's own `Put`, awaited or not.
    pub(in crate::app) fn settings_put_was_screens(&self, id: u64) -> bool {
        matches!(&self.screen, Some(Screen::Settings(s)) if s.put_id == Some(id))
    }

    /// `on_send_failed` (final review I1): the screen's `Put` never left. The screen
    /// says so by the link: `not sent: daemon is not responding` while connected, `not
    /// saved: link lost` otherwise.
    pub(in crate::app) fn settings_put_not_sent(&mut self) {
        let outcome = if self.connected() {
            SaveOutcome::NotSent
        } else {
            SaveOutcome::LinkLost
        };
        if let Some(s) = self.settings_screen_mut() {
            s.put_id = None;
            s.sent = None;
            s.outcome = Some(outcome);
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

/// `config::settings::cleaned`'s rule on the screen's own fields: every hidden format
/// character dropped from each model's name and note (and a custom row's label, its
/// name) and from the orchestrator default's model. Everything else, a disabled custom
/// row and the digits typed in a limit included, is kept as it is.
fn drop_hidden(s: &mut SettingsScreen) {
    use config::settings::{clean_entry, strip_hidden};
    for row in s.claude.iter_mut().chain(s.codex.iter_mut()) {
        clean_entry(&mut row.entry);
        if row.custom {
            row.label = strip_hidden(&row.label);
        }
    }
    // The orchestrator default's model, `cleaned`'s `orchestrator.model`.
    s.model = strip_hidden(&s.model);
}
