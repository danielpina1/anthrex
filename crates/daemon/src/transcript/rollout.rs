//! Milestone 9.16 decision 9: where a Codex session's rollout is. Blocking; called only
//! inside `spawn_blocking`.

use std::ffi::OsString;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// Decision 22: how much of a transcript's or a rollout's end is read.
pub const TRANSCRIPT_TAIL_BYTES: u64 = 262_144;

/// One day, in seconds.
const DAY: u64 = 86_400;

/// Ruling T8-4: the folder Codex saves its sessions in, `$CODEX_HOME/sessions`, else
/// `~/.codex/sessions`, from the daemon's environment (`var`).
pub fn codex_sessions_dir(var: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let set = |key: &str| var(key).filter(|v| !v.is_empty()).map(PathBuf::from);
    match set("CODEX_HOME") {
        Some(home) => Some(home.join("sessions")),
        None => set("HOME").map(|home| home.join(".codex").join("sessions")),
    }
}

/// Decision 9: the rollout of thread `thread_id`, the one file whose name is
/// `rollout-…-<thread_id>.jsonl` in `<sessions>/<YYYY>/<MM>/<DD>/`, for the UTC dates of
/// `started_at` (Unix seconds), then the day before, then the day after: three
/// directories listed, nothing below them walked.
///
/// M9.16.1: Codex names the file by local time, and whether the date directory is local
/// time or UTC was not distinguished. A local date is within a day of the UTC date, so
/// the three UTC days cover both (ruling C27; fake-agent's `Rollout` step names files in
/// local time).
pub fn find_rollout(sessions: &Path, thread_id: &str, started_at: u64) -> Option<PathBuf> {
    let plain = |c: char| c.is_ascii_alphanumeric() || c == '-' || c == '_';
    if thread_id.is_empty() || !thread_id.chars().all(plain) {
        return None;
    }
    let suffix = format!("-{thread_id}.jsonl");
    let days = [
        Some(started_at),
        started_at.checked_sub(DAY),
        started_at.checked_add(DAY),
    ];
    days.into_iter().flatten().find_map(|day| {
        // `YYYY-MM-DD HH:MM:SSZ`.
        let date = common::text::format_utc(day);
        let dir = date
            .get(..10)?
            .split('-')
            .fold(sessions.to_path_buf(), |d, p| d.join(p));
        let entries = std::fs::read_dir(&dir).ok()?;
        entries.flatten().find_map(|entry| {
            let name = entry.file_name();
            let name = name.to_str()?;
            // Task 7's review M1: a regular file only, never a FIFO, a socket or a link
            // (an `open` of a FIFO nobody writes never returns).
            let file = entry.file_type().is_ok_and(|t| t.is_file());
            (file && name.starts_with("rollout-") && name.ends_with(&suffix)).then(|| entry.path())
        })
    })
}

/// The last `max` bytes of `path` (all of it when shorter), as text; a character the cut
/// splits becomes U+FFFD, and the parsers skip the first line anyway (decision 22).
/// Only a regular file is opened (task 7's review M1): anything else is an error.
pub fn read_tail(path: &Path, max: u64) -> std::io::Result<String> {
    if !std::fs::symlink_metadata(path)?.is_file() {
        let what = format!("{} is not a regular file", path.display());
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, what));
    }
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len > max {
        file.seek(SeekFrom::Start(len - max))?;
    }
    let mut bytes = Vec::new();
    file.take(max).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
#[path = "rollout_tests.rs"]
mod tests;
