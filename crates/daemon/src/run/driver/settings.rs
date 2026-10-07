//! Milestone 9.0.6 decisions 25, 28, 29 and 37: `RunRequest::Settings` and the runs a
//! profile edit waits for.
//!
//! `Get` answers from memory (the live settings, decision 29); no file is read. `Put`
//! validates, takes `settings_write` (never the manager or engine lock), runs the
//! blocking `config::settings::save` on `spawn_blocking` under `SettingsIo.timeout`, and
//! swaps the live settings only once the file holds the new text.
//!
//! **The timeout's window.** `save` claims the rename by swapping its `cancel` flag to
//! `true` just before it renames; the daemon, on a timeout, swaps the same flag. Exactly
//! one of the two sees `false`: when the daemon does, the save had not claimed the rename
//! and never will, so "nothing changed" is true; when the save did, the rename is under
//! way and the reply says the outcome is not known yet. Either way the save goes on in
//! the background holding `settings_write`, and a save that does land is swapped in then,
//! so the live settings never disagree with the file.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use proto::{RunReply, RunState, SettingsDoc, SettingsReply, SettingsRequest};

use super::RunService;
use crate::live_config::LiveSettings;

/// Decision 28: a save that did not start renaming within the timeout.
pub const WRITE_TIMEOUT_TEXT: &str = "config.toml was not written within 5 s; nothing changed";

/// A save that had begun renaming when the timeout fired (Task 9 notes).
pub const WRITE_UNKNOWN_TEXT: &str = "config.toml was still being written after 5 s; if the write completes, new runs use the new settings";

fn reply(reply: SettingsReply) -> RunReply {
    RunReply::Settings {
        reply: Box::new(reply),
        request_id: None,
    }
}

fn refused(problems: Vec<String>) -> RunReply {
    reply(SettingsReply::Refused { problems })
}

impl RunService {
    /// Decision 25: `Settings(Get | Put)`.
    pub(super) async fn settings(&self, request: SettingsRequest) -> RunReply {
        match request {
            SettingsRequest::Get => {
                let live = self.ctx.settings.current();
                reply(SettingsReply::Current {
                    doc: config::settings::doc_of(&live.orchestrator),
                    origin: live.origin.clone(),
                    path: self.ctx.config_path.clone(),
                })
            }
            SettingsRequest::Put { settings } => self.put_settings(settings).await,
            // M9.8.12 serves these; until then every request still gets an answer.
            SettingsRequest::RepoModels { .. } | SettingsRequest::PutRepoModels { .. } => {
                refused(vec!["repository models are not available yet".into()])
            }
        }
    }

    /// Decision 28, steps 1 to 4. See the module doc for the timeout.
    async fn put_settings(&self, doc: SettingsDoc) -> RunReply {
        let problems = config::settings::validate(&doc);
        if !problems.is_empty() {
            return refused(problems);
        }
        let io = self.ctx.settings_io.clone();
        let write = self.settings_write.clone().lock_owned();
        // A save still running in the background holds the lock: nothing starts here.
        let Ok(guard) = tokio::time::timeout(io.timeout, write).await else {
            return refused(vec![WRITE_TIMEOUT_TEXT.into()]);
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let (path, flag, wanted) = (self.ctx.config_path.clone(), cancel.clone(), doc);
        let save = io.save.clone();
        let mut task = tokio::task::spawn_blocking(move || save(&path, &wanted, &flag));
        match tokio::time::timeout(io.timeout, &mut task).await {
            Ok(Ok(Ok(saved))) => {
                self.ctx
                    .settings
                    .swap_owned(&saved.orchestrator, saved.origin.clone());
                drop(guard);
                reply(SettingsReply::Saved {
                    doc: config::settings::doc_of(&saved.orchestrator),
                    origin: saved.origin,
                })
            }
            Ok(Ok(Err(problems))) => refused(problems),
            Ok(Err(error)) => refused(vec![format!("the settings save did not finish: {error}")]),
            Err(_) => {
                let claimed = cancel.swap(true, Ordering::SeqCst);
                let live = self.ctx.settings.clone();
                tokio::spawn(finish_late(task, live, guard));
                let text = if claimed {
                    WRITE_UNKNOWN_TEXT
                } else {
                    WRITE_TIMEOUT_TEXT
                };
                refused(vec![text.into()])
            }
        }
    }

    /// Decision 37: the ids of the runs in `project` that are neither terminal nor
    /// `complete`, sorted. The engine lock is held only to read run states.
    pub fn live_runs_in(&self, project: &Path) -> Vec<String> {
        let mut ids: Vec<String> = crate::lock(&self.state)
            .runs
            .values()
            .filter(|r| r.project == project)
            .filter(|r| !r.state.is_terminal() && r.state != RunState::Complete)
            .map(|r| r.id.clone())
            .collect();
        ids.sort();
        ids
    }

    /// The live settings this service's runs are built from (the scouts read them too).
    pub fn live_settings(&self) -> &Arc<LiveSettings> {
        &self.ctx.settings
    }
}

/// A save that outlived the timeout: awaited in the background with `settings_write`
/// held, and swapped in if it landed.
async fn finish_late(
    task: tokio::task::JoinHandle<Result<config::settings::Saved, Vec<String>>>,
    live: Arc<LiveSettings>,
    guard: tokio::sync::OwnedMutexGuard<()>,
) {
    if let Ok(Ok(saved)) = task.await {
        tracing::warn!("a settings save finished after its timeout; applying it");
        live.swap_owned(&saved.orchestrator, saved.origin);
    }
    drop(guard);
}

#[cfg(test)]
#[path = "settings_tests.rs"]
mod tests;
