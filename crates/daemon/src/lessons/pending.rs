//! The followups' "a timed-out `append_once` loses a paid distill result" (#88 review,
//! minor 2): a distilled answer is kept on disk until the store holds it.
//!
//! When a distill call answers, its `attempt` and `distilled` lines become a
//! [`PendingRecord`] in `<repo_dir>/lessons.pending.json` ([`PENDING_FILE`]) before
//! either is appended (written ahead, so a stop or a crash during a stalled append
//! keeps them), and are appended from there with `store::append_once`, under the
//! repository's `LessonLocks` entry, each line once: a line the store already holds
//! (by `record_id`, as a line that parses) is not appended again, so a retry after a
//! crash between the append and the record's removal writes nothing twice. A record
//! goes when every one of its lines is in the store.
//!
//! A failed append is retried after a backoff that doubles from the worker's
//! `write_backoff`, on the monotonic clock (`Instant`); a restart retries at once.
//! After `write_tries` failed tries the record gives up: its `distilled` line is
//! replaced by a `gave_up` line saying why (`the distilled result could not be
//! written to the store after <n> tries: <error>`), which is retried on the longest
//! backoff until it is written, so the entry is closed with its reason and never
//! asked for again. Only the distill worker reads or writes the file, one repository
//! at a time, so it takes no lock of its own; every read and write of it runs on
//! `spawn_blocking` under [`LESSONS_READ_BOUND`] (AGENTS.md rule 2), and the read
//! refuses anything but a regular file (`history_io::read_regular`).

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use common::bounds::LESSONS_READ_BOUND;
use proto::LessonLine;
use serde::{Deserialize, Serialize};

use super::store::{LessonLocks, append_once};

/// The pending results' file in a repository's data directory.
pub const PENDING_FILE: &str = "lessons.pending.json";
/// The file's format.
pub const PENDING_VERSION: u32 = 1;

/// One distill call's lines not yet all in the store.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PendingRecord {
    /// The entry's `queued` record id.
    pub queued: String,
    /// The lines to append, in order.
    pub lines: Vec<LessonLine>,
    /// Failed tries so far, up to the cap.
    pub tries: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Whether the record gave up: its `distilled` line was replaced by `gave_up`.
    #[serde(default)]
    pub gave_up: bool,
}

#[derive(Serialize, Deserialize)]
struct PendingFile {
    v: u32,
    records: Vec<PendingRecord>,
}

/// What a flush left: nothing pending, or a retry due at the given time.
pub enum Flushed {
    Clear,
    Wait(Instant),
}

/// The worker's pending results, per repository, loaded from disk on first use.
pub struct PendingWrites {
    backoff: Duration,
    tries: u32,
    repos: HashMap<PathBuf, Repo>,
}

#[derive(Default)]
struct Repo {
    records: Vec<PendingRecord>,
    retry_at: Option<Instant>,
}

impl PendingWrites {
    /// Retries waiting `backoff`, doubled per failed try, giving up after `tries`.
    pub fn new(backoff: Duration, tries: u32) -> Self {
        PendingWrites {
            backoff,
            tries: tries.max(1),
            repos: HashMap::new(),
        }
    }

    /// Keeps `record` for `repo`, on disk before anything is appended. The caller
    /// flushed `repo` first, so its file is loaded. A failed save is a warning: the
    /// record is still retried while the daemon runs.
    pub async fn hold(&mut self, repo: &Path, record: PendingRecord) {
        let state = self.repos.entry(repo.to_path_buf()).or_default();
        state.records.push(record);
        state.retry_at = None;
        save(repo, state.records.clone()).await;
    }

    /// Appends `repo`'s pending records, oldest first, until one fails or none is
    /// left. `now` (Unix seconds) stamps a `gave_up` line.
    pub async fn flush(&mut self, locks: &LessonLocks, repo: &Path, now: u64) -> Flushed {
        let (backoff, cap) = (self.backoff, self.tries);
        if !self.repos.contains_key(repo) {
            match load(repo).await {
                Ok(records) => {
                    let state = Repo {
                        records,
                        retry_at: None,
                    };
                    self.repos.insert(repo.to_path_buf(), state);
                }
                Err(error) => {
                    let shown = repo.join(PENDING_FILE);
                    tracing::warn!("lessons: {} was not read: {error}", shown.display());
                    return Flushed::Wait(Instant::now() + backoff);
                }
            }
        }
        let Some(state) = self.repos.get_mut(repo) else {
            return Flushed::Clear;
        };
        if let Some(at) = state.retry_at.filter(|at| *at > Instant::now()) {
            return Flushed::Wait(at);
        }
        while let Some(record) = state.records.first_mut() {
            match append_lines(locks, repo, &record.lines).await {
                Ok(()) => {
                    tracing::info!("lessons: wrote the pending result of {}", record.queued);
                    state.records.remove(0);
                    state.retry_at = None;
                    save(repo, state.records.clone()).await;
                }
                Err(error) => {
                    let changed = failed(record, &error, (cap, now));
                    let at = Instant::now() + delay(backoff, record.tries.min(cap));
                    state.retry_at = Some(at);
                    if changed {
                        save(repo, state.records.clone()).await;
                    }
                    return Flushed::Wait(at);
                }
            }
        }
        Flushed::Clear
    }
}

/// `backoff` doubled `tries - 1` times.
fn delay(backoff: Duration, tries: u32) -> Duration {
    let doublings = tries.saturating_sub(1).min(16);
    backoff.saturating_mul(1 << doublings)
}

/// Counts a failed try of `record` (up to `cap`), and at the cap gives it up. True
/// when the record changed, so its file is saved.
fn failed(record: &mut PendingRecord, error: &io::Error, (cap, now): (u32, u64)) -> bool {
    let error = proto::safe_text::one_line(&error.to_string());
    tracing::warn!(
        "lessons: the pending result of {} was not written: {error}",
        record.queued
    );
    if record.tries >= cap {
        return false;
    }
    record.tries += 1;
    record.last_error = Some(error.clone());
    if record.tries >= cap && !record.gave_up {
        let tries = record.tries;
        let reason = format!(
            "the distilled result could not be written to the store after {tries} tries: {error}"
        );
        let dropped = (record.lines.iter())
            .map(|l| match l {
                LessonLine::Distilled(d) => d.items.len(),
                _ => 0,
            })
            .sum::<usize>();
        tracing::warn!(
            "lessons: gave up on the pending result of {}: {dropped} distilled lessons dropped",
            record.queued
        );
        record
            .lines
            .retain(|l| !matches!(l, LessonLine::Distilled(_)));
        record
            .lines
            .push(super::service::gave_up(&record.queued, &reason, now));
        record.gave_up = true;
    }
    true
}

/// Each of `lines` appended once, in order; the first failure stops.
async fn append_lines(locks: &LessonLocks, repo: &Path, lines: &[LessonLine]) -> io::Result<()> {
    for line in lines {
        append_once(locks, repo, line.clone()).await?;
    }
    Ok(())
}

/// `work` on `spawn_blocking` under [`LESSONS_READ_BOUND`]; past it, an error of kind
/// `TimedOut` (its thread is left to end when the file system answers).
async fn bounded<T: Send + 'static>(
    what: &str,
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    let task = tokio::task::spawn_blocking(work);
    match tokio::time::timeout(LESSONS_READ_BOUND, task).await {
        Ok(Ok(done)) => done,
        Ok(Err(error)) => Err(io::Error::other(error)),
        Err(_) => {
            let secs = LESSONS_READ_BOUND.as_secs();
            let message = format!("{what} did not finish within {secs} s");
            Err(io::Error::new(io::ErrorKind::TimedOut, message))
        }
    }
}

/// `repo`'s pending records; none without a file. A file that does not parse is
/// moved aside (`.corrupt`), with a warning, and holds none.
async fn load(repo: &Path) -> io::Result<Vec<PendingRecord>> {
    let path = repo.join(PENDING_FILE);
    bounded("the pending results' read", move || load_blocking(&path)).await
}

/// [`load`]'s blocking core.
fn load_blocking(path: &Path) -> io::Result<Vec<PendingRecord>> {
    let Some(bytes) = crate::run::history_io::read_regular(path)? else {
        return Ok(Vec::new());
    };
    match serde_json::from_slice::<PendingFile>(&bytes) {
        Ok(file) => Ok(file.records),
        Err(error) => {
            let aside = path.with_extension("json.corrupt");
            tracing::warn!(
                "lessons: {} does not parse ({error}); moved to {}",
                path.display(),
                aside.display()
            );
            std::fs::rename(path, &aside)?;
            Ok(Vec::new())
        }
    }
}

/// `records` written to `repo`'s file (atomically), or the file removed when there
/// are none. A failure is a warning.
async fn save(repo: &Path, records: Vec<PendingRecord>) {
    let path = repo.join(PENDING_FILE);
    let shown = path.display().to_string();
    let saved = bounded("the pending results' write", move || {
        if records.is_empty() {
            return match std::fs::remove_file(&path) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            };
        }
        let file = PendingFile {
            v: PENDING_VERSION,
            records,
        };
        let bytes = serde_json::to_vec(&file).map_err(io::Error::other)?;
        crate::profile::store::write_atomic(&path, &bytes)
    })
    .await;
    if let Err(error) = saved {
        tracing::warn!("lessons: {shown} was not saved: {error}");
    }
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;
