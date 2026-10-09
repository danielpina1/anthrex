//! Milestone 9.0.6 decisions 25, 28, 29 and 37: `RunRequest::Settings` and the runs a
//! profile edit waits for; milestone 9.8 decision 36: the repository's role table
//! (`RepoModels`, `PutRepoModels`), read and written on blocking threads.
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

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use proto::models::ModelTable;
use proto::{RunReply, RunState, SettingsDoc, SettingsReply, SettingsRequest};

use super::RunService;
use crate::live_config::LiveSettings;

/// Decision 28: a save that did not start renaming within the timeout.
pub const WRITE_TIMEOUT_TEXT: &str = "config.toml was not written within 5 s; nothing changed";

/// A save that had begun renaming when the timeout fired (Task 9 notes).
pub const WRITE_UNKNOWN_TEXT: &str = "config.toml was still being written after 5 s; if the write completes, new runs use the new settings";

/// Decision 36: a repository table not written within the timeout. Its outcome is not
/// known: the write goes on in the background (an atomic rename, so the file is either
/// the old table or the new one).
pub const REPO_WRITE_TIMEOUT_TEXT: &str =
    "the repository's models.toml was not written within 5 s; it may still be written";

/// Fix round 1 (M6): another settings save held the write lock for the whole timeout.
pub const REPO_BUSY_TEXT: &str = "the repository's models.toml was not written: another settings save is still running; nothing changed";

/// Fix round 1 (I2): a save over rows the daemon cannot read.
fn unreadable_text(problems: &[String]) -> String {
    format!(
        "models.toml has rows anthrex can't read: {}; fix or remove them first",
        problems.join("; ")
    )
}

/// `<repo_dir>/models.toml` (MR §3.3).
fn repo_file(data_dir: &Path, project: &Path) -> PathBuf {
    crate::profile::repo_dir(data_dir, project).join(config::models::REPO_FILE)
}

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
            SettingsRequest::RepoModels { project } => self.repo_models(project).await,
            SettingsRequest::PutRepoModels { project, table } => {
                self.put_repo_models(project, table).await
            }
        }
    }

    /// Milestone 9.8 decision 36: `<repo_dir>/models.toml` as a table, read on a blocking
    /// thread within `SettingsIo.timeout`. A missing file is the empty table; an
    /// unreadable or broken one refuses with its problems; rows it cannot read come back
    /// as `problems` beside the rows it can (fix round 1, I2).
    async fn repo_models(&self, project: PathBuf) -> RunReply {
        let path = repo_file(&self.ctx.data_dir, &project);
        let file = path.clone();
        let read = tokio::task::spawn_blocking(move || config::models::load_repo(&file));
        match tokio::time::timeout(self.ctx.settings_io.timeout, read).await {
            Ok(Ok((table, problems))) if problems.is_empty() || table.is_some() => {
                reply(SettingsReply::RepoModels {
                    project,
                    table: table.unwrap_or_default(),
                    path,
                    // Fix round 1 (I2): the rows it could not read, for the screen to show.
                    problems,
                })
            }
            Ok(Ok((_, problems))) => refused(problems),
            Ok(Err(error)) => refused(vec![format!("the read did not finish: {error}")]),
            Err(_) => refused(vec![format!(
                "{} was not read within {} s",
                path.display(),
                self.ctx.settings_io.timeout.as_secs()
            )]),
        }
    }

    /// Decision 36: `table` written to the repository's `models.toml` (an empty table
    /// removes it), validated first, refused while the file holds rows the daemon cannot
    /// read (fix round 1, I2), on a blocking thread within `SettingsIo.timeout`
    /// and under `settings_write`, so it never races a `Put`. Runs read the file when
    /// they start, so nothing live changes.
    async fn put_repo_models(&self, project: PathBuf, table: ModelTable) -> RunReply {
        let problems = config::settings::validate(&SettingsDoc {
            roles: table.clone(),
            ..config::settings::doc_of(&config::Orchestrator::default())
        });
        if !problems.is_empty() {
            return refused(problems);
        }
        let timeout = self.ctx.settings_io.timeout;
        let write = self.settings_write.clone().lock_owned();
        let Ok(guard) = tokio::time::timeout(timeout, write).await else {
            // Fix round 1 (M6): nothing started.
            return refused(vec![REPO_BUSY_TEXT.into()]);
        };
        let path = repo_file(&self.ctx.data_dir, &project);
        let wanted = table.clone();
        let task = tokio::task::spawn_blocking(move || {
            // Fix round 1 (I2): never erase what the screen could not show.
            let (_, problems) = config::models::load_repo(&path);
            let saved = if problems.is_empty() {
                config::models::save_repo(&path, &wanted)
            } else {
                Err(unreadable_text(&problems))
            };
            drop(guard);
            saved
        });
        match tokio::time::timeout(timeout, task).await {
            Ok(Ok(Ok(()))) => reply(SettingsReply::RepoSaved { project, table }),
            Ok(Ok(Err(problem))) => refused(vec![problem]),
            Ok(Err(error)) => refused(vec![format!("the save did not finish: {error}")]),
            Err(_) => refused(vec![REPO_WRITE_TIMEOUT_TEXT.into()]),
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
        let live = self.ctx.settings.current();
        let mut task = tokio::task::spawn_blocking(move || {
            // Final review I1: the save writes the live table, so a file edited since
            // the daemon read it is refused rather than reverted.
            if let Some(problem) = config::settings::changed_since_loaded(&path, &live.orchestrator)
            {
                return Err(vec![problem]);
            }
            save(&path, &wanted, &flag)
        });
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
