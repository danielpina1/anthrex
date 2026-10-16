//! Milestone 9.5 decision 10: `tuning.toml` in a repository's data directory (next to
//! `history.jsonl`, never in the repository). Blocking: callers run these on
//! `spawn_blocking`, never under a lock, and writes only under the repository's tuning
//! lock (`run::driver::tuning::TuningLocks`).
//!
//! An absent file is every default. A file that does not parse is renamed to
//! `tuning.toml.bad-<unix secs>` and tuning starts again from history: a broken tuning
//! file never stops a run. Every write is `profile::store::write_atomic`, whose crashed
//! leftovers the run restore sweeps at daemon start (decision 36).

use std::io;
use std::path::{Path, PathBuf};

use proto::{SizeThresholds, TUNING_VERSION, TuningFile};

use crate::profile::store::write_atomic;

pub const TUNING_FILE: &str = "tuning.toml";

/// Milestone 9.18 decision 5: how many `tuning.toml.bad-*` files a move leaves.
pub const TUNING_BAD_KEEP: usize = 3;

/// What [`load`] found.
#[derive(Debug, Clone, PartialEq)]
pub enum Loaded {
    File(TuningFile),
    Absent,
    /// The file did not parse (`error`) and was moved to `moved_to`.
    MovedBad {
        error: String,
        moved_to: PathBuf,
    },
}

/// The file's text parsed, or why not, on one line (`line <n>: <message>`): TOML,
/// every table `deny_unknown_fields`, and this version only.
fn parse(text: &str) -> Result<TuningFile, String> {
    let file: TuningFile = toml::from_str(text).map_err(|e| {
        let message = e.message().trim().replace('\n', " ");
        match e.span() {
            Some(span) => {
                let line = text[..span.start.min(text.len())].matches('\n').count() + 1;
                format!("line {line}: {message}")
            }
            None => message,
        }
    })?;
    if file.v != TUNING_VERSION {
        return Err(format!(
            "version {} is not {TUNING_VERSION}, the one this anthrex reads",
            file.v
        ));
    }
    Ok(file)
}

/// The file's text, or `None` when there is none.
fn read(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).into_owned())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// `<repo_dir>/tuning.toml`; one that does not parse is moved to
/// `tuning.toml.bad-<now>` (`-<n>` added while that name is taken), and the older
/// moved-aside files are pruned ([`prune_bad`]).
pub fn load(repo_dir: &Path, now: u64) -> io::Result<Loaded> {
    let path = repo_dir.join(TUNING_FILE);
    let Some(text) = read(&path)? else {
        return Ok(Loaded::Absent);
    };
    let error = match parse(&text) {
        Ok(file) => return Ok(Loaded::File(file)),
        Err(error) => error,
    };
    let bad = |n: u32| {
        let suffix = if n == 0 {
            String::new()
        } else {
            format!("-{n}")
        };
        repo_dir.join(format!("{TUNING_FILE}.bad-{now}{suffix}"))
    };
    let moved_to = (0..).map(bad).find(|p| !p.exists()).expect("a free name");
    std::fs::rename(&path, &moved_to)?;
    prune_bad(repo_dir, &moved_to);
    Ok(Loaded::MovedBad { error, moved_to })
}

/// Milestone 9.18 decision 5 and ruling C20: removes every `tuning.toml.bad-<secs>[-n]`
/// in `repo_dir` but `moved_to` (the file just moved aside, kept even when the clock
/// went back) and the `TUNING_BAD_KEEP - 1` newest others by `(secs, n)`. A name that
/// does not parse is never removed; a failed read or removal is ignored.
fn prune_bad(repo_dir: &Path, moved_to: &Path) {
    let Ok(entries) = std::fs::read_dir(repo_dir) else {
        return;
    };
    let just_moved = moved_to.file_name();
    let mut others: Vec<((u64, u64), PathBuf)> = (entries.flatten())
        .filter(|entry| Some(entry.file_name().as_os_str()) != just_moved)
        .filter_map(|entry| Some((bad_stamp(entry.file_name().to_str()?)?, entry.path())))
        .collect();
    others.sort_by_key(|(stamp, _)| std::cmp::Reverse(*stamp));
    for (_, path) in others.into_iter().skip(TUNING_BAD_KEEP.saturating_sub(1)) {
        let _ = std::fs::remove_file(path);
    }
}

/// The `(secs, n)` of a `tuning.toml.bad-<secs>[-<n>]` name (`n` 0 when absent), or
/// `None` for any other name.
fn bad_stamp(name: &str) -> Option<(u64, u64)> {
    let rest = name.strip_prefix(TUNING_FILE)?.strip_prefix(".bad-")?;
    let (secs, n) = match rest.split_once('-') {
        Some((secs, n)) => (secs, Some(n)),
        None => (rest, None),
    };
    let number = |s: &str| {
        (!s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
            .then(|| s.parse::<u64>().ok())
            .flatten()
    };
    Some((number(secs)?, n.map_or(Some(0), number)?))
}

/// The file as [`load`] reads it, never moved (milestone 9.5 decision 48's read-only
/// `Stats`): `Ok(None)` when absent, the one-line parse error when it does not parse.
pub fn peek(repo_dir: &Path) -> io::Result<Result<Option<TuningFile>, String>> {
    Ok(match read(&repo_dir.join(TUNING_FILE))? {
        None => Ok(None),
        Some(text) => parse(&text).map(Some),
    })
}

/// Writes `file` as `<repo_dir>/tuning.toml`, atomically.
pub fn save(repo_dir: &Path, file: &TuningFile) -> io::Result<()> {
    let text = toml::to_string(file).map_err(io::Error::other)?;
    write_atomic(&repo_dir.join(TUNING_FILE), text.as_bytes())
}

/// Decision 13: the thresholds triage's prompt gives, read without moving or writing
/// anything (the defaults when the file is absent, unreadable or does not parse).
pub fn thresholds(repo_dir: &Path) -> SizeThresholds {
    read(&repo_dir.join(TUNING_FILE))
        .ok()
        .flatten()
        .and_then(|text| parse(&text).ok())
        .and_then(|file| file.thresholds)
        .unwrap_or_default()
}
