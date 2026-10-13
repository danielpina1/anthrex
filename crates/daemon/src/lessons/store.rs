//! Milestone 9.15 decisions 2 to 4: the lessons store on disk. Writes go through
//! `history_io`'s append (one whole line per write, a torn tail healed first) on
//! `spawn_blocking`, under the repository's [`LessonLocks`] entry, never the manager
//! lock or the engine lock (AGENTS.md rule 2). A read takes no lock: a line being
//! appended reads as a torn last line, which the parse skips as a problem.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use proto::LessonLine;
use tokio::sync::{Mutex, OwnedMutexGuard};

use super::LESSONS_FILE;
use super::fold;
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
pub async fn append_once(
    locks: &LessonLocks,
    repo_dir: &Path,
    line: LessonLine,
) -> io::Result<bool> {
    let path = repo_dir.join(LESSONS_FILE);
    let _guard = locks.lock(repo_dir).await;
    blocking(move || {
        if holds_record(&path, line.record_id())? {
            return Ok(false);
        }
        history_io::append_json(&path, &line).map(|()| true)
    })
    .await
}

/// Whether the store at `path` holds a line of `record_id` that parses, by the fold's
/// own parse: [`append_once`]'s test, and the `QueueDistill` reconcile row's (a torn
/// line is no record). No file holds nothing; an unreadable one is an error. Blocking.
pub fn holds_record(path: &Path, record_id: &str) -> io::Result<bool> {
    let text = match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
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

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
