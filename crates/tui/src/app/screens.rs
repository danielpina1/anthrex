//! Milestone 9.0.6 decision 24: the settings the daemon owns, as the client keeps them.
//! One tagged `Settings(Get)` leaves with every new connection; its `Current` reply and
//! every `Saved` become `App.settings_cache`, which the goal form's model picker, the
//! Promote form and (tasks 13-15) the screens read. None of them sends a request of its
//! own to read it. Pure: every request leaves as an `Effect`.

use super::replies::PendingWhat;
use super::runs::first_line_and_more;
use super::{App, Effect, Modal, ToastLevel};
use proto::{
    ModelEntry, Origin, RunReply, RunRequest, Runtime, SettingsDoc, SettingsReply, SettingsRequest,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The daemon's settings document, where each key came from, and the config path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsCache {
    pub doc: SettingsDoc,
    pub origin: BTreeMap<String, Origin>,
    pub path: PathBuf,
}

impl SettingsCache {
    /// The enabled roster, in order.
    pub fn models(&self) -> &[ModelEntry] {
        &self.doc.models
    }

    /// The names of `runtime`'s enabled models, in roster order. An entry with an empty
    /// model is the runtime's own default and is not a name.
    pub fn models_of(&self, runtime: Runtime) -> Vec<String> {
        models_of(self.models(), runtime)
    }
}

/// [`SettingsCache::models_of`] over any roster.
pub fn models_of(models: &[ModelEntry], runtime: Runtime) -> Vec<String> {
    models
        .iter()
        .filter(|m| m.runtime == runtime && !m.model.is_empty())
        .map(|m| m.model.clone())
        .collect()
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
    /// `Settings` reply with any other id is dropped.
    pub(super) fn route_settings_reply(&mut self, reply: &RunReply) -> Option<Vec<Effect>> {
        let RunReply::Settings { reply, request_id } = reply else {
            return None;
        };
        let Some(pending) = self.replies.take(*request_id) else {
            return Some(vec![]);
        };
        let put = match pending.what {
            PendingWhat::SettingsGet => false,
            PendingWhat::SettingsPut => true,
            _ => return None,
        };
        let mut effects = Vec::new();
        match &**reply {
            SettingsReply::Current { doc, origin, path } if !put => {
                self.set_cache(SettingsCache {
                    doc: doc.clone(),
                    origin: origin.clone(),
                    path: path.clone(),
                });
            }
            SettingsReply::Saved { doc, origin } if put => {
                match self.settings_cache.as_ref().map(|c| c.path.clone()) {
                    Some(path) => self.set_cache(SettingsCache {
                        doc: doc.clone(),
                        origin: origin.clone(),
                        path,
                    }),
                    // No `Current` yet: the path is unknown, so ask.
                    None => effects.push(self.settings_fetch()),
                }
            }
            other => {
                if let SettingsReply::Refused { problems } = other {
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
        Some(effects)
    }

    /// Replaces the cache and tells the open goal form its roster changed.
    fn set_cache(&mut self, cache: SettingsCache) {
        if let Some(Modal::StartGoal(form)) = &mut self.modal {
            form.set_roster(cache.doc.models.clone());
        }
        self.settings_cache = Some(cache);
    }
}
