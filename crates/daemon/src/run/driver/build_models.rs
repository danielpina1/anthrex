//! Milestone 9.8 decision 9, the driver side: a run's start freezes the role table
//! ([`RunService::freeze_models`]) from the live global table, the repository file
//! (`<repo_dir>/models.toml`, read on `spawn_blocking`, never under the engine lock) and
//! the catalogs in memory, validated against them (decision 21); a run recorded before
//! the role table gets the current one at restore ([`fill`]). I/O.

use std::path::Path;
use std::time::Duration;

use proto::ModelCatalog;
use proto::models::ModelTable;

use super::RunService;
use crate::run::model::{LogEntry, Run};
use crate::run::model_roles::RunModels;

/// Decision 9's run-log line for a run filled at restore.
pub const PREDATES: &str = "models: this run predates the role table; using the current one";

/// How long a start waits for the repository file's read (one small file); past it the
/// run starts on the global table and says so.
pub const MODELS_READ_BOUND: Duration = Duration::from_secs(5);

/// The run log's line for a start whose repository file read did not finish in time.
const READ_SLOW: &str =
    "models: the repository's models.toml was not read within 5 s; using the global table";

/// The run log's line for a start whose repository file read failed outright.
const READ_FAILED: &str =
    "models: the repository's models.toml could not be read; using the global table";

/// Decision 9: `global` overlaid by `project`'s repository file, every role resolved,
/// then validated against `catalogs` (decision 21). The repository file's problems (a
/// broken file falls back to the global table) and the validation's lines are returned
/// for the run log, each on one line. Blocking: it reads the repository file.
pub fn freeze(
    global: &ModelTable,
    project: &Path,
    data_dir: &Path,
    catalogs: &[ModelCatalog],
) -> (RunModels, Vec<String>) {
    let path = crate::profile::repo_dir(data_dir, project).join(config::models::REPO_FILE);
    let (repo, problems) = config::models::load_repo(&path);
    let mut models = RunModels::resolve(global, repo.as_ref());
    let mut lines: Vec<String> = (problems.iter())
        .map(|p| proto::safe_text::one_line(p))
        .collect();
    lines.extend(models.validate(catalogs));
    (models, lines)
}

/// Decision 9 at restore: a run recorded without a role table takes `frozen` (from
/// [`freeze`] on the restore's own blocking step), its lines and [`PREDATES`] logged;
/// its tasks keep the routes they have. A run that has one is left alone.
pub fn fill(run: &mut Run, frozen: (RunModels, Vec<String>), now: u64) {
    if run.limits.models.is_some() {
        return;
    }
    let (models, lines) = frozen;
    run.limits.models = Some(models);
    for text in lines.into_iter().chain([PREDATES.to_string()]) {
        run.log.push(LogEntry { at: now, text });
    }
}

impl RunService {
    /// Decision 9 for a start: [`freeze`] on `spawn_blocking`, bounded by
    /// [`MODELS_READ_BOUND`], over the catalogs in memory now; with none in memory, a
    /// refresh starts in the background and the start does not wait for it (decision 20).
    pub(super) async fn freeze_models(
        &self,
        global: &ModelTable,
        project: &Path,
    ) -> (RunModels, Vec<String>) {
        let service = self.models();
        let catalogs = service.current();
        if catalogs.is_empty() {
            service.refresh_in_background();
        }
        let (table, project) = (global.clone(), project.to_path_buf());
        let data_dir = self.ctx.data_dir.clone();
        let read =
            tokio::task::spawn_blocking(move || freeze(&table, &project, &data_dir, &catalogs));
        match tokio::time::timeout(MODELS_READ_BOUND, read).await {
            Ok(Ok(frozen)) => frozen,
            Ok(Err(error)) => {
                tracing::warn!(%error, "freezing the role table panicked");
                (
                    RunModels::resolve(global, None),
                    vec![READ_FAILED.to_string()],
                )
            }
            Err(_) => (
                RunModels::resolve(global, None),
                vec![READ_SLOW.to_string()],
            ),
        }
    }
}
