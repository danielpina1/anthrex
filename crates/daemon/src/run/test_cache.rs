//! Milestone 9.1 decision 30: the result cache (I/O). One [`TestCache`] per daemon
//! (`RunService`) keeps, per repository, an index of the green tier steps recorded in
//! `<repo_dir>/test-cache.jsonl`, one JSON line per green step: its key
//! (`run::tiers::cache_key`, compared field by field) and what it did.
//!
//! - **Only green results are written.** [`TestCache::store`] refuses an entry that is
//!   not `ok`, and a line that says otherwise is never a hit. The executor stores a
//!   step only when its last whole run passed (ruling C-7): never a by-name pass.
//! - **Loading.** A repository's file is read once, lazily, by the first lookup or
//!   store that needs it. Lines older than `test_cache_days`, lines that are not
//!   green, and corrupt, torn or overlong lines are dropped; when any was, or when the
//!   file holds more than [`CACHE_LINES_MAX`] lines, it is rewritten atomically
//!   (`profile::store::write_atomic`: a temporary file and a rename). A file that
//!   cannot be read is a miss.
//! - **Storing** appends one line with one `write_all` and `sync_all`. When the file
//!   passes [`CACHE_LINES_MAX`] lines it is cut to the newest [`CACHE_LINES_KEPT`] and
//!   rewritten atomically, so a full cache is rewritten once per 2 000 stores, not at
//!   every one.
//! - **Memory.** The index holds each key and its entry without the tail; the tail
//!   (the last lines of the green run, at most [`CACHE_TAIL_BYTES`]) is in the file
//!   only.
//! - **Blocking.** Every method may read or write the file: call it only from a
//!   blocking thread (`spawn_blocking`, AGENTS.md rule 2). The index's mutex is taken
//!   with `daemon::lock` and only to read or update the index: never across a file read
//!   or write. Two stores racing a rewrite can lose one line (the last rename wins); the
//!   cache is advisory, so that costs one re-run.
//! - **Freshness** (ruling C-13): an entry more than 300 s in the future is expired, and
//!   every lookup stats the file, so a file deleted or replaced behind the daemon is
//!   read again (a deleted one is empty).
//! - `test_cache_days = 0` turns the cache off: nothing is read or written.

use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

use crate::profile::store::write_atomic;
use crate::run::driver::unix_now;
use crate::run::exec::CHECK_TAIL_LINES;
use crate::run::tiers::cache_key::CacheKey;

/// The cache file in a repository's data directory.
pub const CACHE_FILE: &str = "test-cache.jsonl";
/// At most this many lines per repository; the oldest go first.
pub const CACHE_LINES_MAX: usize = 20_000;
/// What a file past [`CACHE_LINES_MAX`] is cut to.
pub const CACHE_LINES_KEPT: usize = CACHE_LINES_MAX - CACHE_LINES_MAX / 10;
/// The most of a green run's tail a line keeps: its last whole lines, at most
/// [`CHECK_TAIL_LINES`], within this many bytes.
pub const CACHE_TAIL_BYTES: usize = 4 * 1024;
/// A line longer than this is corrupt: skipped without being held.
const LINE_BYTES_MAX: usize = 1024 * 1024;
const DAY_SECS: u64 = 86_400;
/// An entry whose `at` is further ahead of now than this is expired (ruling C-13 (3)):
/// a clock that jumped cannot keep an entry alive.
const FUTURE_SECS: u64 = 300;

/// What a green step did (decision 30's value).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheEntry {
    /// Always `true` in the file: only green steps are written.
    pub ok: bool,
    pub secs: u64,
    /// The tests that failed first and passed on the retry.
    #[serde(default)]
    pub flaky: Vec<String>,
    /// The green run's last lines. Empty in what [`TestCache::lookup`] returns: the
    /// index keeps no tail.
    #[serde(default)]
    pub tail: String,
    /// Unix seconds when the step ran.
    pub at: u64,
    /// The run and the tier that wrote it.
    pub run: String,
    pub tier: u8,
}

/// One line of the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheLine {
    pub key: CacheKey,
    pub entry: CacheEntry,
}

/// A repository's index: every usable line's key (the newest line of a key wins) and
/// the number of lines the file holds.
#[derive(Debug, Default)]
struct Index {
    map: HashMap<CacheKey, CacheEntry>,
    lines: usize,
    /// The file as this index last saw it (ruling C-13 (4)).
    stamp: Stamp,
}

/// A file's identity and its last change: `(dev, inode, mtime s, mtime ns, length)`;
/// `None` when it is missing or cannot be read.
type Stamp = Option<(u64, u64, i64, i64, u64)>;

fn stamp(path: &Path) -> Stamp {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    Some((
        meta.dev(),
        meta.ino(),
        meta.mtime(),
        meta.mtime_nsec(),
        meta.len(),
    ))
}

struct Inner {
    days: u32,
    repos: Mutex<HashMap<PathBuf, Index>>,
}

/// The daemon's result cache (decision 30). Cheap to clone: every clone is the same
/// cache.
#[derive(Clone)]
pub struct TestCache {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for TestCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestCache")
            .field("days", &self.inner.days)
            .finish_non_exhaustive()
    }
}

impl TestCache {
    /// A cache whose entries count for `days` (`[testing] test_cache_days`; 0: off).
    pub fn new(days: u32) -> Self {
        TestCache {
            inner: Arc::new(Inner {
                days,
                repos: Mutex::new(HashMap::new()),
            }),
        }
    }

    pub fn enabled(&self) -> bool {
        self.inner.days > 0
    }

    fn max_age(&self) -> u64 {
        u64::from(self.inner.days) * DAY_SECS
    }

    /// The green entry stored under `key` in the repository whose data directory is
    /// `repo_dir`, unless it is older than the cache's days. Blocking.
    pub fn lookup(&self, repo_dir: &Path, key: &CacheKey) -> Option<CacheEntry> {
        if !self.enabled() {
            return None;
        }
        if let Err(error) = self.ensure(repo_dir) {
            tracing::debug!(%error, repo = %repo_dir.display(), "test cache unreadable: a miss");
            return None;
        }
        let now = unix_now();
        let repos = crate::lock(&self.inner.repos);
        let entry = repos.get(repo_dir)?.map.get(key)?;
        (entry.ok && !expired(entry, now, self.max_age())).then(|| entry.clone())
    }

    /// Records a green step. An entry that is not `ok` is not stored. Blocking.
    pub fn store(&self, repo_dir: &Path, key: CacheKey, entry: CacheEntry) -> io::Result<()> {
        if !self.enabled() || !entry.ok {
            return Ok(());
        }
        self.ensure(repo_dir)?;
        let mut entry = entry;
        entry.tail = cut_tail(&entry.tail);
        let mut line = serde_json::to_vec(&CacheLine {
            key: key.clone(),
            entry: entry.clone(),
        })
        .map_err(io::Error::other)?;
        line.push(b'\n');
        let path = repo_dir.join(CACHE_FILE);
        append(&path, &line)?;
        let after = stamp(&path);
        entry.tail.clear();
        let over = {
            let mut repos = crate::lock(&self.inner.repos);
            let index = repos.entry(repo_dir.to_path_buf()).or_default();
            index.map.insert(key, entry);
            index.lines += 1;
            index.stamp = after;
            index.lines > CACHE_LINES_MAX
        };
        if over {
            let mut index = rewrite(&path, unix_now(), self.max_age(), CACHE_LINES_KEPT)?;
            index.stamp = stamp(&path);
            crate::lock(&self.inner.repos).insert(repo_dir.to_path_buf(), index);
        }
        Ok(())
    }

    /// Loads `repo_dir`'s index unless it is loaded and the file is as it last saw it
    /// (ruling C-13 (4): a file deleted or replaced behind the daemon is read again; a
    /// deleted one gives an empty index). The file is stat'ed and read with no lock
    /// held.
    fn ensure(&self, repo_dir: &Path) -> io::Result<()> {
        let path = repo_dir.join(CACHE_FILE);
        let seen = stamp(&path);
        if let Some(index) = crate::lock(&self.inner.repos).get(repo_dir)
            && index.stamp == seen
        {
            return Ok(());
        }
        let mut index = load(&path, unix_now(), self.max_age())?;
        index.stamp = stamp(&path);
        crate::lock(&self.inner.repos).insert(repo_dir.to_path_buf(), index);
        Ok(())
    }
}

fn expired(entry: &CacheEntry, now: u64, max_age: u64) -> bool {
    entry.at > now.saturating_add(FUTURE_SECS) || now.saturating_sub(entry.at) > max_age
}

/// A line's parse, when it is green and within its days.
fn usable(bytes: &[u8], now: u64, max_age: u64) -> Option<CacheLine> {
    let line: CacheLine = serde_json::from_slice(bytes).ok()?;
    (line.entry.ok && !expired(&line.entry, now, max_age)).then_some(line)
}

/// Reads `path`'s index; rewrites the file when it dropped a line or holds too many.
/// A missing file is empty.
fn load(path: &Path, now: u64, max_age: u64) -> io::Result<Index> {
    let mut index = Index::default();
    let mut dropped = false;
    let read = each_line(path, |line| {
        match line.and_then(|l| usable(l, now, max_age)) {
            Some(CacheLine { key, mut entry }) => {
                entry.tail.clear();
                index.map.insert(key, entry);
                index.lines += 1;
            }
            None => dropped = true,
        }
    });
    match read {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(index),
        Err(error) => return Err(error),
    }
    if dropped || index.lines > CACHE_LINES_MAX {
        let keep = if index.lines > CACHE_LINES_MAX {
            CACHE_LINES_KEPT
        } else {
            CACHE_LINES_MAX
        };
        match rewrite(path, now, max_age, keep) {
            Ok(rewritten) => return Ok(rewritten),
            // The index read stands; the next load tries again.
            Err(error) => tracing::debug!(%error, "could not rewrite the test cache"),
        }
    }
    Ok(index)
}

/// Rewrites `path` atomically with its newest `keep` usable lines, and returns their
/// index.
fn rewrite(path: &Path, now: u64, max_age: u64, keep: usize) -> io::Result<Index> {
    let mut kept: Vec<(Vec<u8>, CacheLine)> = Vec::new();
    each_line(path, |line| {
        if let Some(bytes) = line
            && let Some(parsed) = usable(bytes, now, max_age)
        {
            kept.push((bytes.to_vec(), parsed));
        }
    })?;
    let skip = kept.len().saturating_sub(keep);
    let mut bytes = Vec::new();
    let mut index = Index::default();
    for (raw, CacheLine { key, mut entry }) in kept.into_iter().skip(skip) {
        bytes.extend_from_slice(&raw);
        bytes.push(b'\n');
        entry.tail.clear();
        index.map.insert(key, entry);
        index.lines += 1;
    }
    write_atomic(path, &bytes)?;
    Ok(index)
}

/// Appends `line` with one `write_all` and `sync_all`, the file made `0600`.
fn append(path: &Path, line: &[u8]) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(line)?;
    file.sync_all()
}

/// Calls `f` with each newline-ended line of `path` (without its newline), or `None`
/// for a line past [`LINE_BYTES_MAX`] or a last line with no newline (a torn append).
/// No line past the cap is held in memory.
fn each_line(path: &Path, mut f: impl FnMut(Option<&[u8]>)) -> io::Result<()> {
    let mut reader = BufReader::new(std::fs::File::open(path)?);
    let mut line = Vec::new();
    let mut over = false;
    loop {
        let buf = reader.fill_buf()?;
        if buf.is_empty() {
            if over || !line.is_empty() {
                f(None);
            }
            return Ok(());
        }
        let (take, end) = match buf.iter().position(|&b| b == b'\n') {
            Some(at) => (at, true),
            None => (buf.len(), false),
        };
        if !over {
            if line.len() + take > LINE_BYTES_MAX {
                over = true;
                line = Vec::new();
            } else {
                line.extend_from_slice(&buf[..take]);
            }
        }
        reader.consume(take + usize::from(end));
        if end {
            f((!over).then_some(line.as_slice()));
            line.clear();
            over = false;
        }
    }
}

/// The last whole lines of `tail`, at most [`CHECK_TAIL_LINES`] and
/// [`CACHE_TAIL_BYTES`]; a last line longer than that alone keeps its end.
fn cut_tail(tail: &str) -> String {
    let mut kept: Vec<&str> = Vec::new();
    let mut bytes = 0usize;
    for line in tail.lines().rev().take(CHECK_TAIL_LINES) {
        let cost = line.len() + usize::from(!kept.is_empty());
        if bytes + cost > CACHE_TAIL_BYTES {
            if kept.is_empty() {
                let mut from = line.len() - CACHE_TAIL_BYTES;
                while !line.is_char_boundary(from) {
                    from += 1;
                }
                return line[from..].to_string();
            }
            break;
        }
        bytes += cost;
        kept.push(line);
    }
    kept.reverse();
    kept.join("\n")
}

#[cfg(test)]
#[path = "test_cache_tests.rs"]
mod tests;
