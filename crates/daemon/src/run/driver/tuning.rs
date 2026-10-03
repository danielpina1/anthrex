//! Milestone 9.5 decisions 10 and 12, the driver side: a run's start refits
//! `tuning.toml` from the repository's history and freezes the result into the run
//! ([`tune_for_start`], called by `build_delivered` only, ruling RH-8).
//!
//! **Locks.** Each repository's tuning is read, refitted and written under its own
//! `tokio::sync::Mutex` ([`TuningLocks`]), never the engine or manager lock, and never
//! across a git call: the work is file I/O on `spawn_blocking`, which holds the guard
//! until it ends, so a start that gave up never lets a second writer in. One bound,
//! [`TUNING_START_BOUND`], covers both the wait for the lock and the work (ruling
//! T9-3): a start never waits longer for its tuning, whatever another start does.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use proto::TuningFile;
use tokio::sync::OwnedMutexGuard;

use crate::run::engine::HISTORY_FILE;
use crate::run::history_io::read_history;
use crate::run::refit::{self, Tuned};
use crate::run::refit_render::moved_bad_line;
use crate::run::tuning_io::{self, Loaded};

/// Ruling T9-3: how long a start waits for its tuning, the repository's tuning lock and
/// the file work together (one history read and one small write); past it the run
/// starts untuned and says so ([`TUNING_BUSY`]). Every start bound that counts the
/// build counts it (`docs/timing-budgets.md`).
pub const TUNING_START_BOUND: Duration = Duration::from_secs(10);

/// The run log's line for a start that gave up at [`TUNING_START_BOUND`].
pub const TUNING_BUSY: &str =
    "tuning: none (the tuning file was busy; started without what history taught)";

/// One `tokio::sync::Mutex` per repository data directory (decision 10), and how many
/// tunings were asked of it (each start asks once).
#[derive(Default)]
pub struct TuningLocks {
    repos: Mutex<HashMap<PathBuf, Arc<tokio::sync::Mutex<()>>>>,
    tunings: AtomicU64,
}

impl TuningLocks {
    /// `repo_dir`'s tuning lock, held until the guard drops.
    pub async fn lock(&self, repo_dir: &Path) -> OwnedMutexGuard<()> {
        let lock = crate::lock(&self.repos)
            .entry(repo_dir.to_path_buf())
            .or_default()
            .clone();
        lock.lock_owned().await
    }

    /// How many starts have tuned through these locks.
    pub fn tunings(&self) -> u64 {
        self.tunings.load(Ordering::Relaxed)
    }
}

/// What a run uses when nothing is learned or read: the config's lists and explicit
/// budgets and no refit, with `line` (why) as its one log line, or none.
fn untuned(config: &config::Orchestrator, line: Option<String>) -> Tuned {
    Tuned {
        log: line.into_iter().collect(),
        ..refit::tuned(&TuningFile::default(), config)
    }
}

/// `tuning: none (<why>; started without what history taught)`.
fn none_because(why: impl std::fmt::Display) -> Option<String> {
    Some(format!(
        "tuning: none ({why}; started without what history taught)"
    ))
}

/// Decision 12 for a start in the repository whose data directory is `repo_dir`: the
/// file loaded (a bad one moved aside), refitted from `history.jsonl` and written when
/// the refit changed it, then frozen. `Tuned.log` is the run log's tuning lines in
/// order: the moved file's line, the refit-write lines (also to the daemon log), then
/// the start lines, each text once. A run with history off (an empty `repo_dir`)
/// reads and writes no tuning. `config` is the start's one read of the settings.
pub async fn tune_for_start(
    config: &config::Orchestrator,
    locks: &TuningLocks,
    repo_dir: &Path,
    now: u64,
) -> Tuned {
    tune_within(config, locks, repo_dir, now, TUNING_START_BOUND).await
}

/// [`tune_for_start`] with its bound given (a test's): the lock's wait and the work
/// together end by `bound`.
pub async fn tune_within(
    config: &config::Orchestrator,
    locks: &TuningLocks,
    repo_dir: &Path,
    now: u64,
    bound: Duration,
) -> Tuned {
    locks.tunings.fetch_add(1, Ordering::Relaxed);
    if repo_dir.as_os_str().is_empty() {
        return untuned(config, None);
    }
    let work = async {
        let guard = locks.lock(repo_dir).await;
        let (dir, cfg) = (repo_dir.to_path_buf(), config.clone());
        tokio::task::spawn_blocking(move || {
            let tuned = tune_blocking(&dir, &cfg, now);
            drop(guard);
            tuned
        })
        .await
    };
    match tokio::time::timeout(bound, work).await {
        Ok(Ok(tuned)) => tuned,
        Ok(Err(error)) => {
            tracing::warn!(%error, "tuning did not finish; the run starts untuned");
            untuned(
                config,
                none_because(format!("tuning did not finish: {error}")),
            )
        }
        Err(_) => {
            let secs = bound.as_secs_f64();
            tracing::warn!("tuning took over {secs} s; the run starts untuned");
            untuned(config, Some(TUNING_BUSY.to_string()))
        }
    }
}

/// [`tune_for_start`]'s blocking core, under the repository's tuning lock.
fn tune_blocking(repo_dir: &Path, cfg: &config::Orchestrator, now: u64) -> Tuned {
    let mut log = Vec::new();
    let file = match tuning_io::load(repo_dir, now) {
        Ok(Loaded::File(file)) => file,
        Ok(Loaded::Absent) => TuningFile::default(),
        Ok(Loaded::MovedBad { error, moved_to }) => {
            let line = moved_bad_line(Some(&error), &moved_to);
            tracing::warn!(repo = %repo_dir.display(), "{line}");
            log.push(line);
            TuningFile::default()
        }
        Err(error) => {
            // Unreadable, not unparseable: nothing is moved or overwritten.
            tracing::warn!(repo = %repo_dir.display(), %error, "tuning.toml unreadable");
            return untuned(
                cfg,
                none_because(format!("tuning.toml could not be read: {error}")),
            );
        }
    };
    let (lines, _) = read_history(&repo_dir.join(HISTORY_FILE));
    let (refitted, written) = refit::refit(&lines, &file, cfg, now);
    let used = if refitted == file {
        file
    } else {
        match tuning_io::save(repo_dir, &refitted) {
            Ok(()) => {
                for line in &written {
                    tracing::info!(repo = %repo_dir.display(), "{line}");
                }
                log.extend(written);
                refitted
            }
            Err(error) => {
                tracing::warn!(repo = %repo_dir.display(), %error, "tuning.toml not written");
                file
            }
        }
    };
    let mut tuned = refit::tuned_with(&lines, &used, cfg);
    // Ruling T9-5: a start line the refit already wrote (the weights' two lines are one
    // text) is written once.
    for line in tuned.log.drain(..) {
        if !log.contains(&line) {
            log.push(line);
        }
    }
    tuned.log = log;
    tuned
}
