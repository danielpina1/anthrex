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
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use proto::{ProposalValue, TuningFile, TuningReport};
use tokio::sync::OwnedMutexGuard;

use crate::run::engine::HISTORY_FILE;
use crate::run::history_io::read_history;
use crate::run::refit::{self, Tuned};
use crate::run::refit_render::moved_bad_line;
use crate::run::tuning_io::{self, Loaded, TUNING_FILE};

/// Ruling T9-3: how long a start waits for its tuning, the repository's tuning lock and
/// the file work together (one history read and one small write); past it the run
/// starts untuned and says so ([`TUNING_BUSY`]). Every start bound that counts the
/// build counts it (`docs/timing-budgets.md`).
pub const TUNING_START_BOUND: Duration = Duration::from_secs(10);

/// The run log's line for a start that gave up at [`TUNING_START_BOUND`].
pub const TUNING_BUSY: &str =
    "tuning: none (the tuning file was busy; started without what history taught)";

/// `run stats`' refusal when the repository's tuning stayed busy past
/// [`TUNING_START_BOUND`].
pub const TUNING_STATS_BUSY: &str =
    "the tuning file stayed busy (a run is starting); nothing was applied or dismissed; try again";

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

    /// How many starts have tuned through these locks (tests only).
    #[cfg(test)]
    pub(crate) fn tunings(&self) -> u64 {
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

/// What `run stats` asks of the tuning (decisions 11 and 48): the proposals to apply,
/// each with the value the user confirmed (whole-branch review C, m-2), the ids to
/// dismiss, and whether nothing may be recorded or written.
#[derive(Debug, Clone, Copy, Default)]
pub struct StatsAsk<'a> {
    pub apply: &'a [ProposalValue],
    pub dismiss: &'a [String],
    pub read_only: bool,
}

/// `run stats --apply`'s refusal of a proposal whose value is no longer the one the
/// user confirmed (whole-branch review C, m-2; exact).
pub fn proposal_changed(id: &str) -> String {
    format!("proposal {id} changed since you saw it; run anthrex run stats again; nothing applied")
}

/// Where the blocking work stands, so a request that gave up at its bound is never
/// answered "busy" while its write lands (task M9.5.11's fix round, review m3).
const RUNNING: u8 = 0;
const COMMITTING: u8 = 1;
const ABANDONED: u8 = 2;

/// Decisions 11 and 48: the tuning half of `run stats` for the repository whose data
/// directory is `repo_dir` and whose main checkout is `project`, under its tuning lock
/// and within [`TUNING_START_BOUND`] (the lock's wait and the file work together, as a
/// start's). The file is loaded (a bad one moved aside) and refitted from
/// `history.jsonl`; `ask`'s ids are taken against the proposals still current, an
/// unknown id refusing the whole request with nothing written; then the file is saved
/// when it changed. With `read_only` nothing is moved or written: the report shows the
/// file as it is, and each class's `refit_budget` is the refit a plain `run stats`
/// would write. `config` is the request's one read of the settings.
pub async fn stats_with_tuning(
    config: &config::Orchestrator,
    locks: &TuningLocks,
    (repo_dir, project): (&Path, &Path),
    ask: StatsAsk<'_>,
    now: u64,
) -> Result<TuningReport, String> {
    stats_within(
        config,
        locks,
        (repo_dir, project),
        ask,
        now,
        TUNING_START_BOUND,
    )
    .await
}

/// [`stats_with_tuning`] with its bound given (a test's). Past the bound, the answer is
/// [`TUNING_STATS_BUSY`] only when the file work is stopped before its write; once the
/// write has begun, its own result is awaited and returned.
pub async fn stats_within(
    config: &config::Orchestrator,
    locks: &TuningLocks,
    (repo_dir, project): (&Path, &Path),
    ask: StatsAsk<'_>,
    now: u64,
    bound: Duration,
) -> Result<TuningReport, String> {
    let busy = || Err(TUNING_STATS_BUSY.to_string());
    let deadline = tokio::time::Instant::now() + bound;
    let Ok(guard) = tokio::time::timeout_at(deadline, locks.lock(repo_dir)).await else {
        return busy();
    };
    let state = Arc::new(AtomicU8::new(RUNNING));
    let (dir, cfg, shared) = (repo_dir.to_path_buf(), config.clone(), state.clone());
    let (apply, dismiss, read_only) = (ask.apply.to_vec(), ask.dismiss.to_vec(), ask.read_only);
    let commit = move || {
        (shared.compare_exchange(RUNNING, COMMITTING, Ordering::SeqCst, Ordering::SeqCst)).is_ok()
    };
    let mut work = tokio::task::spawn_blocking(move || {
        let report = stats_blocking(&dir, &cfg, (&apply, &dismiss), read_only, now, commit);
        drop(guard);
        report
    });
    let done = match tokio::time::timeout_at(deadline, &mut work).await {
        Ok(done) => done,
        Err(_)
            if state
                .compare_exchange(RUNNING, ABANDONED, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok() =>
        {
            return busy();
        }
        // The write had begun: its result is the answer.
        Err(_) => work.await,
    };
    match done {
        Ok(report) => report.map(|mut r| {
            r.project = Some(project.to_path_buf());
            r
        }),
        Err(error) => Err(format!("tuning did not finish: {error}")),
    }
}

/// [`stats_with_tuning`]'s blocking core, under the repository's tuning lock. `commit`
/// is asked once, just before the only write; `false` means the request gave up, so
/// nothing is written and the answer is [`TUNING_STATS_BUSY`].
fn stats_blocking(
    repo_dir: &Path,
    cfg: &config::Orchestrator,
    (apply, dismiss): (&[ProposalValue], &[String]),
    read_only: bool,
    now: u64,
    commit: impl FnOnce() -> bool,
) -> Result<TuningReport, String> {
    let unreadable = |e: std::io::Error| format!("tuning.toml could not be read: {e}");
    let (file, moved, parse_error) = if read_only {
        match tuning_io::peek(repo_dir).map_err(unreadable)? {
            Ok(file) => (file.unwrap_or_default(), None, None),
            Err(error) => (TuningFile::default(), None, Some(error)),
        }
    } else {
        match tuning_io::load(repo_dir, now).map_err(unreadable)? {
            Loaded::File(file) => (file, None, None),
            Loaded::Absent => (TuningFile::default(), None, None),
            Loaded::MovedBad { error, moved_to } => {
                let line = moved_bad_line(Some(&error), &moved_to);
                tracing::warn!(repo = %repo_dir.display(), "{line}");
                (TuningFile::default(), Some(moved_to), Some(error))
            }
        }
    };
    let (lines, _) = read_history(&repo_dir.join(HISTORY_FILE));
    let (refitted, written) = refit::refit(&lines, &file, cfg, now);
    let current = refit::proposals(&lines, &refitted, cfg);
    // Whole-branch review C, m-2: only the value the user confirmed is applied.
    let changed = apply
        .iter()
        .find(|a| (current.iter()).any(|p| p.id == a.id && p.proposed != a.value));
    if let Some(a) = changed {
        return Err(proposal_changed(&a.id));
    }
    let ids: Vec<String> = apply.iter().map(|a| a.id.clone()).collect();
    let decided = refit::apply(&refitted, &current, &ids)?;
    let decided = refit::dismiss(&decided, &current, dismiss)?;
    let path = repo_dir.join(TUNING_FILE);
    let mut report = if read_only {
        let mut shown = refit::report(&lines, &file, cfg, &path);
        let would = refit::report(&lines, &refitted, cfg, &path);
        for (class, would) in shown.classes.iter_mut().zip(would.classes) {
            class.refit_budget = would.refit_budget;
        }
        shown
    } else {
        if decided != file {
            if !commit() {
                return Err(TUNING_STATS_BUSY.to_string());
            }
            tuning_io::save(repo_dir, &decided)
                .map_err(|e| format!("tuning.toml could not be written: {e}"))?;
            for line in &written {
                tracing::info!(repo = %repo_dir.display(), "{line}");
            }
        }
        refit::report(&lines, &decided, cfg, &path)
    };
    report.moved_bad_file = moved;
    report.parse_error = parse_error;
    report.applied = ids;
    report.dismissed = (dismiss.iter())
        .map(|id| ProposalValue {
            id: id.clone(),
            value: decided.dismissed.get(id).cloned().unwrap_or_default(),
        })
        .collect();
    Ok(report)
}
