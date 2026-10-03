//! M8b decisions 32 and 33, the driver side: `MeasureDiff` and `AppendHistory`, each on
//! `spawn_blocking`, never under a lock. A run record of an accepted run gets its
//! `accepted_commit` just before it is appended: the one field the reducer cannot know.
//!
//! M8b.17, decisions 34 and 35: revert detection at every `run start` (both kinds,
//! from `choose_profile`) and `anthrex run stats`, also on `spawn_blocking`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use proto::run_wire::request;
use proto::{HistoryStats, RunReply};

use super::super::{OpCtx, RunService, unix_now};
use crate::profile::store::Stored;
use crate::run::engine::HISTORY_FILE;
use crate::run::engine::{OpKind, OpResult};
use crate::run::history_io::{
    append_line, fill_accepted_commit, measure_diff, record_reverts, summarise,
};

fn failed(message: String) -> OpResult {
    OpResult::Failed { message }
}

impl RunService {
    /// `OpKind::MeasureDiff` or `OpKind::AppendHistory`, as their executor contracts say.
    pub(in crate::run::driver) async fn history_op(&self, ctx: &OpCtx, kind: OpKind) -> OpResult {
        let (git, timeout) = (self.ctx.git.clone(), ctx.git_timeout);
        let result = match kind {
            OpKind::MeasureDiff {
                root,
                from,
                to,
                three_dot,
            } => {
                tokio::task::spawn_blocking(move || {
                    measure_diff(&git, &root, &from, &to, three_dot, timeout)
                        .map(OpResult::DiffMeasured)
                })
                .await
            }
            OpKind::AppendHistory { path, line, .. } => {
                let root = crate::lock(&self.state)
                    .runs
                    .get(&ctx.run_id)
                    .map(|run| run.root.clone());
                tokio::task::spawn_blocking(move || {
                    let line = match root {
                        Some(root) => fill_accepted_commit(&git, &root, *line, timeout),
                        None => *line,
                    };
                    append_line(&path, &line)
                        .map(|()| OpResult::HistoryAppended)
                        .map_err(|error| format!("could not append to {}: {error}", path.display()))
                })
                .await
            }
            other => return failed(format!("{} is not a history op", other.name())),
        };
        match result {
            Ok(Ok(result)) => result,
            Ok(Err(message)) => failed(message),
            Err(error) => failed(format!("a blocking step did not finish: {error}")),
        }
    }
}

/// The checkout `dir` is in and its project (main checkout), without preflight's
/// clean-tree rules: `run stats` works in a dirty checkout. Blocking.
fn checkout_of(
    git: &std::ffi::OsStr,
    dir: &Path,
    timeout: Duration,
) -> Result<(PathBuf, PathBuf), String> {
    let roots = crate::project::detect_roots_with(git, dir, timeout);
    match roots.worktree {
        Some(root) => Ok((root, roots.project)),
        None if roots.detection_failed => Err(format!(
            "could not tell whether {} is a git repository: git did not answer; try again",
            dir.display()
        )),
        None => Err(format!("not a git repository: {}", dir.display())),
    }
}

/// `RunRequest::Stats`' blocking core for `dir` at `now`: its repository's history
/// summarised, with the stored profile's `slow_tests` (milestone 9.5 decision 35).
fn stats_of(
    git: &std::ffi::OsStr,
    dir: &Path,
    data_dir: &Path,
    testing: &config::Testing,
    now: u64,
    timeout: Duration,
) -> Result<HistoryStats, String> {
    let (root, project) = checkout_of(git, dir, timeout)?;
    let repo_dir = crate::profile::repo_dir(data_dir, &project);
    let slow = match crate::profile::store::load(&repo_dir) {
        Stored::Found { profile, .. } => profile.slow_tests,
        _ => None,
    };
    let path = repo_dir.join(HISTORY_FILE);
    let slow = slow.as_deref();
    Ok(summarise(git, &root, &path, now, timeout, testing, slow))
}

impl RunService {
    fn git_timeout_secs(&self) -> Duration {
        Duration::from_secs(self.ctx.settings.current().orchestrator.git_timeout_secs)
    }

    /// `RunRequest::Stats` (decision 35): reverts recorded first (decision 34), then the
    /// history of `dir`'s repository summarised, all on `spawn_blocking`. A repository
    /// with no history answers empty rows.
    pub(in crate::run::driver) async fn stats(&self, dir: PathBuf) -> RunReply {
        let refused = |message: String| RunReply::refused(request::STATS, message);
        let (git, timeout) = (self.ctx.git.clone(), self.git_timeout_secs());
        let data_dir = self.ctx.data_dir.clone();
        let testing = self.ctx.testing.clone();
        let result = tokio::task::spawn_blocking(move || {
            stats_of(&git, &dir, &data_dir, &testing, unix_now(), timeout)
        })
        .await;
        match result {
            Ok(Ok(stats)) => RunReply::stats(stats),
            Ok(Err(message)) => refused(message),
            Err(error) => refused(format!("a blocking step did not finish: {error}")),
        }
    }

    /// Decision 34 at `run start`: reverts of `repo_dir`'s history looked for in `root`,
    /// in the background (the start does not wait for it); each failure is a warning.
    pub(in crate::run::driver) fn detect_reverts_later(&self, root: PathBuf, repo_dir: &Path) {
        let (git, timeout) = (self.ctx.git.clone(), self.git_timeout_secs());
        let path = repo_dir.join(HISTORY_FILE);
        drop(tokio::task::spawn_blocking(move || {
            for warning in record_reverts(&git, &root, &path, unix_now(), timeout) {
                tracing::warn!(path = %path.display(), %warning, "revert detection");
            }
        }));
    }
}

#[cfg(test)]
#[path = "adapt_history_tests.rs"]
mod tests;
