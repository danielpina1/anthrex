//! Milestone 9.15 decisions 2 to 4: the lessons store on disk. Writes go through
//! `history_io`'s append (one whole line per write, a torn tail healed first) on
//! `spawn_blocking`, under the repository's [`LessonLocks`] entry, never the manager
//! lock or the engine lock (AGENTS.md rule 2). A read takes no lock: a line being
//! appended reads as a torn last line, which the parse skips as a problem.

use std::collections::HashMap;
use std::io;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::bounds::LESSONS_READ_BOUND;
use proto::LessonLine;
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::LESSONS_FILE;
use super::fold;
use super::service::DAY_SECS;
use crate::run::history_io::{self, UNREADABLE};

/// One `tokio::sync::Mutex` per repository data directory, as `TuningLocks`. The map's
/// own lock is a `tokio` one too, so it never poisons and nothing here takes a
/// `std` mutex.
#[derive(Default)]
pub struct LessonLocks {
    repos: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
}

impl LessonLocks {
    /// `repo_dir`'s store lock, held until the guard drops.
    pub async fn lock(&self, repo_dir: &Path) -> OwnedMutexGuard<()> {
        let lock = self
            .repos
            .lock()
            .await
            .entry(repo_dir.to_path_buf())
            .or_default()
            .clone();
        lock.lock_owned().await
    }

    /// How many hold or wait for `repo_dir`'s lock, the map's own reference included
    /// (0 for a repository never locked): the service test's view of a waiting pass.
    #[cfg(test)]
    pub(crate) async fn holders(&self, repo_dir: &Path) -> usize {
        (self.repos.lock().await.get(repo_dir)).map_or(0, Arc::strong_count)
    }
}

/// Decision 4: every line of the store that parses (the last of each `record_id`) and
/// one problem per line that does not. No file is an empty store. Blocking: callers
/// run it on `spawn_blocking`.
pub fn read_store(path: &Path) -> (Vec<LessonLine>, Vec<String>) {
    match std::fs::read(path) {
        Ok(bytes) => fold::parse(
            &String::from_utf8_lossy(&bytes),
            &path.display().to_string(),
        ),
        Err(error) if error.kind() == io::ErrorKind::NotFound => (Vec::new(), Vec::new()),
        Err(error) => (
            Vec::new(),
            vec![format!("{UNREADABLE}{}: {error}", path.display())],
        ),
    }
}

/// [`read_store`] through `history_io::read_regular`: anything but a regular file is
/// refused (`Err`, `history_io::not_regular`'s words) and never read; any other error
/// reads as [`read_store`]'s. The lessons requests' read. Blocking.
pub fn read_store_regular(path: &Path) -> Result<(Vec<LessonLine>, Vec<String>), String> {
    match history_io::read_regular(path) {
        Ok(Some(bytes)) => Ok(fold::parse(
            &String::from_utf8_lossy(&bytes),
            &path.display().to_string(),
        )),
        Ok(None) => Ok((Vec::new(), Vec::new())),
        Err(error) if error.kind() == io::ErrorKind::InvalidInput => Err(error.to_string()),
        Err(error) => Ok((
            Vec::new(),
            vec![format!("{UNREADABLE}{}: {error}", path.display())],
        )),
    }
}

/// Appends `line` to `<repo_dir>/lessons.jsonl`.
pub async fn append(locks: &LessonLocks, repo_dir: &Path, line: LessonLine) -> io::Result<()> {
    let path = repo_dir.join(LESSONS_FILE);
    let _guard = locks.lock(repo_dir).await;
    blocking(move || history_io::append_json(&path, &line)).await
}

/// Appends `line` unless the store holds its `record_id` already; true when appended.
///
/// Task 3's review, I1 (the controller's ruling): "already" means a line that parses,
/// by the fold's own parse. A torn line (a crash or a full disk mid-append) carries
/// the `record_id` in its first bytes but is skipped by the fold, so it must not stop
/// the record from being written whole. An unreadable store is an error.
///
/// The read never runs unbounded under the lock: it is awaited for at most
/// [`LESSONS_READ_BOUND`], and past it the guard drops (the lock is released) and the
/// call is an error of kind `TimedOut`, writing nothing; its blocking thread is left
/// to end when the file system answers, holding no lock. The append then runs with
/// the guard moved into it, so it is never unlocked, untimed as [`append`]'s.
pub async fn append_once(
    locks: &LessonLocks,
    repo_dir: &Path,
    line: LessonLine,
) -> io::Result<bool> {
    append_once_reading(locks, repo_dir, line, LESSONS_READ_BOUND, holds_record).await
}

/// [`append_once`] with its read and that read's bound given (the tests' seam).
pub(crate) async fn append_once_reading(
    locks: &LessonLocks,
    repo_dir: &Path,
    line: LessonLine,
    bound: Duration,
    holds: impl FnOnce(&Path, &str) -> io::Result<bool> + Send + 'static,
) -> io::Result<bool> {
    let path = repo_dir.join(LESSONS_FILE);
    let guard = locks.lock(repo_dir).await;
    let (read_path, id) = (path.clone(), line.record_id().to_string());
    match tokio::time::timeout(bound, blocking(move || holds(&read_path, &id))).await {
        Ok(held) => {
            if held? {
                return Ok(false);
            }
        }
        Err(_) => {
            let (shown, secs) = (path.display(), bound.as_secs_f64());
            let what = format!("{shown} could not be read within {secs} s");
            return Err(io::Error::new(io::ErrorKind::TimedOut, what));
        }
    }
    blocking(move || {
        let _guard = guard;
        history_io::append_json(&path, &line).map(|()| true)
    })
    .await
}

/// Whether the store at `path` holds a line of `record_id` that parses, by the fold's
/// own parse: [`append_once`]'s test, and the `QueueDistill` reconcile row's (a torn
/// line is no record). No file holds nothing; an unreadable one is an error, and so is
/// anything but a regular file, refused before any read (`history_io::read_regular`).
/// Blocking.
pub fn holds_record(path: &Path, record_id: &str) -> io::Result<bool> {
    let text = match history_io::read_regular(path)? {
        Some(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        None => return Ok(false),
    };
    let (lines, _) = fold::parse(&text, "");
    Ok(lines.iter().any(|l| l.record_id() == record_id))
}

/// `work` on `spawn_blocking`; a panicked task is an error.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> io::Result<T> + Send + 'static,
) -> io::Result<T> {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|error| Err(io::Error::other(error)))
}

/// Decision 4: the size below which [`compact`] leaves the store alone.
pub const LESSONS_COMPACT_BYTES: u64 = 1024 * 1024;

/// What [`compact`] did: nothing, or a rewrite from `before` lines to `after`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compacted {
    Skipped,
    Rewrote { before: usize, after: usize },
}

/// Milestone 9.18 decision 3: rewrites the store at `path` without (a) an `attempt`
/// line whose entry has a `distilled` or `gave_up` line, when it is at least
/// [`DAY_SECS`] old at `now` (ruling C1: the daily cap still counts the recent ones),
/// and (b) a line whose `record_id` a later line that parses repeats. A line that does
/// not parse is kept byte for byte, in place, a torn last line included. Decision 4:
/// skipped under [`LESSONS_COMPACT_BYTES`], or when less than a quarter of the lines
/// would go. The rewrite is `write_atomic`'s temporary file and rename, so a lock-free
/// reader reads the whole old file or the whole new one, and an error leaves the file
/// as it was, and the store keeps its mode. A failed `fsync` of the directory after the rename is no error: the file
/// is compacted, so it is a warning and `Rewrote`. Blocking; the caller holds the
/// repository's [`LessonLocks`] entry ([`compact_locked`]).
pub fn compact(path: &Path, now: u64) -> io::Result<Compacted> {
    let mode = match std::fs::metadata(path) {
        Ok(meta) if meta.len() >= LESSONS_COMPACT_BYTES => meta.permissions().mode() & 0o7777,
        Ok(_) => return Ok(Compacted::Skipped),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Compacted::Skipped),
        Err(error) => return Err(error),
    };
    let bytes = std::fs::read(path)?;
    let segments: Vec<&[u8]> = bytes.split_inclusive(|b| *b == b'\n').collect();
    let keep = kept(&segments, now);
    let before = segments.len();
    let after = keep.iter().filter(|k| **k).count();
    if (before - after) * 4 < before {
        return Ok(Compacted::Skipped);
    }
    let mut out = Vec::with_capacity(bytes.len());
    for (segment, _) in segments.iter().zip(&keep).filter(|(_, k)| **k) {
        out.extend_from_slice(segment);
    }
    // PR #80's review, item 1: the store keeps its own mode (`append` creates it with
    // the umask's), not `write_atomic`'s `0600`.
    crate::profile::store::replace_atomic_mode(path, &out, mode)?;
    // M9.18.3's review, M2: past the rename the store is compacted; a failed directory
    // `fsync` only leaves the rename's durability to the file system.
    if let Err(error) = crate::profile::store::sync_parent(path) {
        let shown = path.display();
        tracing::warn!("lessons: compacted {shown}, but its directory was not synced: {error}");
    }
    Ok(Compacted::Rewrote { before, after })
}

/// Ruling C19: [`compact`] under `repo_dir`'s [`LessonLocks`] entry, on
/// `spawn_blocking`. A failure is a warning and `None`, the file as it was. The lock is
/// held untimed across the rewrite's `fsync`s, as [`append`] holds it across its own
/// (AGENTS.md rule 2 is kept: no tokio worker blocks, only this repository's writers
/// wait). PR #80's review, item 1: first, under the same lock, [`sweep_rewrites`]; its
/// failure is a warning and the compaction still runs.
pub async fn compact_locked(locks: &LessonLocks, repo_dir: &Path, now: u64) -> Option<Compacted> {
    let path = repo_dir.join(LESSONS_FILE);
    let shown = path.display().to_string();
    let _guard = locks.lock(repo_dir).await;
    let dir = repo_dir.to_path_buf();
    match blocking(move || {
        if let Err(error) = sweep_rewrites(&dir) {
            let shown = dir.display();
            tracing::warn!("lessons: {shown}'s crashed rewrites were not swept: {error}");
        }
        compact(&path, now)
    })
    .await
    {
        Ok(done) => {
            if let Compacted::Rewrote { before, after } = done {
                tracing::info!("lessons: compacted {shown} from {before} lines to {after}");
            }
            Some(done)
        }
        Err(error) => {
            tracing::warn!("lessons: {shown} was not compacted: {error}");
            None
        }
    }
}

/// PR #80's review, item 1: removes every crashed compaction's temporary file in
/// `repo_dir` (`lessons.jsonl.<pid>.<n>.tmp`, `write_atomic`'s name for the store; a
/// file of exactly that shape, nothing else), and every crashed write's of the
/// pending results' file. The caller holds the repository's
/// [`LessonLocks`] entry, under which no rewrite is in flight, so every such file is a
/// crashed one's. It runs before each compaction, so a worker's start sweeps before
/// compaction can run again. Blocking. A missing directory has nothing to sweep.
fn sweep_rewrites(repo_dir: &Path) -> io::Result<()> {
    let entries = match std::fs::read_dir(repo_dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        if !is_rewrite_tmp(&entry.file_name().to_string_lossy()) || !entry.file_type()?.is_file() {
            continue;
        }
        match std::fs::remove_file(entry.path()) {
            Ok(()) => tracing::info!("lessons: removed {}", entry.path().display()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

/// Whether `name` is `lessons.jsonl.<pid>.<n>.tmp`, both numbers decimal, or the same
/// shape of the pending results' file (`lessons.pending.json`, written by the worker
/// that compacts, between its passes: `super::pending`).
fn is_rewrite_tmp(name: &str) -> bool {
    let decimal = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let Some(middle) = (name.strip_prefix(LESSONS_FILE))
        .or_else(|| name.strip_prefix(super::pending::PENDING_FILE))
        .and_then(|rest| rest.strip_prefix('.'))
        .and_then(|rest| rest.strip_suffix(".tmp"))
    else {
        return false;
    };
    matches!(middle.split_once('.'), Some((pid, n)) if decimal(pid) && decimal(n))
}

/// Which of the store's raw lines (`\n` included) [`compact`] keeps. Each line parses
/// as `fold::parse` parses it: lossy UTF-8, then `from_str` (M9.18.3's review, M1).
fn kept(segments: &[&[u8]], now: u64) -> Vec<bool> {
    let parsed: Vec<Option<LessonLine>> = (segments.iter())
        .map(|raw| {
            let raw = raw.strip_suffix(b"\n").unwrap_or(raw);
            let raw = raw.strip_suffix(b"\r").unwrap_or(raw);
            serde_json::from_str(&String::from_utf8_lossy(raw)).ok()
        })
        .collect();
    let mut last: HashMap<&str, usize> = HashMap::new();
    for (n, line) in parsed.iter().enumerate() {
        if let Some(line) = line {
            last.insert(line.record_id(), n);
        }
    }
    let live = |n: usize, line: &LessonLine| last.get(line.record_id()) == Some(&n);
    let closed: std::collections::HashSet<&str> = (parsed.iter().enumerate())
        .filter_map(|(n, line)| match line.as_ref()? {
            LessonLine::Distilled(d) if live(n, line.as_ref()?) => Some(d.queued.as_str()),
            LessonLine::GaveUp(g) if live(n, line.as_ref()?) => Some(g.queued.as_str()),
            _ => None,
        })
        .collect();
    (parsed.iter().enumerate())
        .map(|(n, line)| match line {
            None => true,
            Some(line) if !live(n, line) => false,
            Some(LessonLine::Attempt(a)) => {
                !(closed.contains(a.queued.as_str()) && now.saturating_sub(a.at) >= DAY_SECS)
            }
            Some(_) => true,
        })
        .collect()
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "store_tests_compact.rs"]
mod tests_compact;

#[cfg(test)]
#[path = "store_tests_sweep.rs"]
mod tests_sweep;
