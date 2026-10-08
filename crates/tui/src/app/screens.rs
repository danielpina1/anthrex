//! Milestone 9.0.6 decision 33: the one slot the full-body screens share (`Screen`),
//! its keys and its tick; and decision 24: the settings the daemon owns, as the client
//! keeps them.
//! One tagged `Settings(Get)` leaves with every new connection; its `Current` reply and
//! every `Saved` become `App.settings_cache`, which the goal form's role table entry
//! (milestone 9.8) and (tasks 13-15) the screens read. None of them
//! sends a request of its own to read it. Pure: every request leaves as an `Effect`.

use super::replies::PendingWhat;
use super::runs::first_line_and_more;
use super::{App, Effect, Modal, ToastLevel};
use proto::{Origin, RunReply, RunRequest, SettingsDoc, SettingsReply, SettingsRequest};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Instant;

/// Decision 33: the full-body screen drawn over `Layout.body`, the status bar staying.
/// Opening one replaces another.
#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    Profile(Box<super::profile_screen::ProfileScreen>),
    Settings(Box<super::settings_screen::SettingsScreen>),
    Stats(Box<super::stats::StatsScreen>),
    /// Milestone 9.6 decision 34: a run's brainstorm or spec gate (`app/doc_gate.rs`).
    DocGate(Box<super::doc_gate::DocGateScreen>),
}

impl App {
    /// Opens `screen` over the body; the keymap's screen mode follows.
    pub(super) fn set_screen(&mut self, screen: Option<Screen>) {
        self.screen = screen;
        self.keymap.set_screen_mode(self.screen.is_some());
    }

    /// Progress ruling: `C-b a`, `C-b m` and `C-b t` would act under a full screen, so
    /// they are refused with a toast while one is open; and the Settings screen's
    /// unsaved changes are not dropped by opening another screen over them.
    pub(super) fn screen_refuses(&mut self, cmd: crate::keymap::Command) -> bool {
        use super::settings_screen::{LEAVE_SETTINGS_FIRST, UNSAVED_FIRST};
        use crate::keymap::Command;
        let leave = match &self.screen {
            Some(Screen::Profile(_)) => super::profile_screen::LEAVE_SCREEN_FIRST,
            Some(Screen::Settings(_)) => LEAVE_SETTINGS_FIRST,
            Some(Screen::Stats(_)) => super::stats::LEAVE_STATS_FIRST,
            Some(Screen::DocGate(_)) => super::doc_gate::LEAVE_DOC_FIRST,
            None => return false,
        };
        let text = match (&self.screen, cmd) {
            (_, Command::FocusAlerts | Command::ToggleConversation | Command::ToggleTree) => leave,
            (Some(Screen::Settings(s)), Command::OpenProfile) if s.dirty() => UNSAVED_FIRST,
            _ => return false,
        };
        self.toast(text);
        true
    }

    /// A bare key while a screen is open (`KeyAction::Screen`).
    pub(super) fn on_screen_key(&mut self, key: crossterm::event::KeyEvent) -> Vec<Effect> {
        match &self.screen {
            Some(Screen::Profile(_)) => self.on_profile_key(key),
            Some(Screen::Settings(_)) => self.on_settings_key(key),
            Some(Screen::Stats(_)) => self.on_stats_key(key),
            Some(Screen::DocGate(_)) => self.on_doc_gate_key(key),
            None => vec![],
        }
    }

    /// `on_tick`: the open screen's timed work at `now` (the Profile screen's poll; the
    /// Settings and stats screens settle a request no longer awaited).
    /// Tests call it with a moved clock instead of sleeping.
    pub(crate) fn screens_tick(&mut self, now: Instant) -> Vec<Effect> {
        match &self.screen {
            Some(Screen::Profile(_)) => self.profile_tick(now),
            Some(Screen::Settings(_)) => self.settings_tick(),
            Some(Screen::Stats(_)) => self.stats_tick(),
            Some(Screen::DocGate(_)) => self.doc_gate_tick(),
            // Milestone 9.6: the plan review's Plan doc tab, over no screen.
            None => self.plan_doc_tick(),
        }
    }
}

/// The daemon's settings document, where each key came from, and the config path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsCache {
    pub doc: SettingsDoc,
    pub origin: BTreeMap<String, Origin>,
    pub path: PathBuf,
    /// The request id of the reply that set this cache; an older reply never replaces it.
    pub id: u64,
}

impl App {
    /// The connection's one `Settings(Get)` (decision 24): `lib.rs` sends it beside
    /// `run_subscription` after `App::new`, and `on_reconnected` for every new
    /// connection. Its reply fills `settings_cache`.
    pub fn settings_fetch(&mut self) -> Effect {
        let (id, effect) = self.tagged_request(RunRequest::Settings(SettingsRequest::Get));
        self.replies
            .insert(id, PendingWhat::SettingsGet, super::replies::REPLY_TIMEOUT);
        effect
    }

    /// A save of `doc`: a tagged `Settings(Put)` whose `Saved` reply replaces the cache.
    /// Any other reply re-syncs the cache with a `Get` (a timeout's outcome is unknown).
    pub fn settings_put(&mut self, doc: SettingsDoc) -> Effect {
        let request = RunRequest::Settings(SettingsRequest::Put { settings: doc });
        let timeout = super::replies::reply_timeout(&request);
        let (id, effect) = self.tagged_request(request);
        self.replies.insert(id, PendingWhat::SettingsPut, timeout);
        effect
    }

    /// `route_reply`'s settings arm: a reply to a `Get` or a `Put` of ours, by id. A
    /// `Settings` reply with any other id is dropped, and one whose id names another
    /// kind of pending request is left for its owner. The daemon spawns each request, so
    /// replies can arrive out of order: a reply applies only when its id is newer than
    /// the id that last set the cache.
    pub(super) fn route_settings_reply(&mut self, reply: &RunReply) -> Option<Vec<Effect>> {
        let RunReply::Settings { reply, request_id } = reply else {
            return None;
        };
        let Some(id) = *request_id else {
            return Some(vec![]);
        };
        let put = match self.replies.peek(id) {
            None => return Some(vec![]),
            Some(PendingWhat::SettingsGet) => false,
            Some(PendingWhat::SettingsPut) => true,
            Some(PendingWhat::RepoModels { project, put }) => {
                let (project, put) = (project.clone(), *put);
                self.replies.take(Some(id));
                self.settings_repo_reply(id, project, put, reply);
                return Some(vec![]);
            }
            Some(_) => return None,
        };
        // The open Settings screen shows its own save's outcome; no toast for it.
        let screens = put && self.settings_put_is_screens(id);
        self.replies.take(Some(id));
        let newer = self.settings_cache.as_ref().is_none_or(|c| id > c.id);
        let mut effects = Vec::new();
        match &**reply {
            SettingsReply::Current { doc, origin, path } if !put => {
                if newer {
                    self.set_cache(SettingsCache {
                        doc: doc.clone(),
                        origin: origin.clone(),
                        path: path.clone(),
                        id,
                    });
                }
            }
            SettingsReply::Saved { doc, origin } if put => {
                match self.settings_cache.as_ref().map(|c| c.path.clone()) {
                    Some(path) if newer => self.set_cache(SettingsCache {
                        doc: doc.clone(),
                        origin: origin.clone(),
                        path,
                        id,
                    }),
                    Some(_) => {}
                    // No `Current` yet: the path is unknown, so ask.
                    None => effects.push(self.settings_fetch()),
                }
            }
            other => {
                if let SettingsReply::Refused { problems } = other
                    && !screens
                {
                    let text = first_line_and_more(&problems.join("\n"))
                        .unwrap_or_else(|| "settings refused".into());
                    self.toast_at(ToastLevel::Error, text);
                }
                // Progress ruling: after a `Put` that was not `Saved` the cache may be
                // stale (a timeout's outcome is unknown), so ask again.
                if put {
                    effects.push(self.settings_fetch());
                }
            }
        }
        // Decision 36: the Settings screen shows its own save's outcome; milestone 9.5
        // decision 48: a save can make a budget explicit, so its notes are asked again.
        self.settings_screen_reply(id, reply);
        if screens && matches!(**reply, SettingsReply::Saved { .. }) {
            effects.extend(self.settings_tuning_ask());
        }
        Some(effects)
    }

    /// Replaces the cache and tells the open goal form (its design default, its role
    /// table entry).
    fn set_cache(&mut self, cache: SettingsCache) {
        if let Some(Modal::StartGoal(form)) = &mut self.modal {
            form.design_default = cache.doc.design_default;
        }
        self.settings_cache = Some(cache);
        self.refresh_goal_models();
        self.sync_settings_screen();
    }
}
