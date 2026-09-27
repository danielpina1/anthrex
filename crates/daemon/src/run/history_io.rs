//! M8b decisions 32 and 33: `history.jsonl` on disk, and the diff measurement. Blocking
//! I/O (M8b decision 1): the driver calls every function here on `spawn_blocking`,
//! never under a lock. Every git call goes through `run::git::Git` (`worktree::run_git`'s
//! code: `--no-optional-locks`, a scrubbed environment, hooks off, a deadline).

use std::collections::HashMap;
use std::ffi::OsStr;
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Duration;

use proto::{DiffStats, HistoryLine};

use super::git::{DIFF_FLAGS, Git, os, read_ref};

/// Appends `line` as one JSON line, then `sync_all`. The file and its directory are
/// created when missing. The line is written with one `write_all` of the whole line,
/// so a reader sees it whole or torn at its end, never interleaved.
pub fn append_line(path: &Path, line: &HistoryLine) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = serde_json::to_string(line).map_err(std::io::Error::other)?;
    text.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// `"record_id":"<id>"` as `serde_json` writes it.
fn needle(record_id: &str) -> String {
    let id = serde_json::to_string(record_id).unwrap_or_default();
    format!("\"record_id\":{id}")
}

/// Whether the file holds a line of `record_id` (decision 33's reconcile row). No
/// file holds nothing.
pub fn contains_record(path: &Path, record_id: &str) -> std::io::Result<bool> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(error),
    };
    let needle = needle(record_id);
    for line in std::io::BufReader::new(file).split(b'\n') {
        if String::from_utf8_lossy(&line?).contains(&needle) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Every record, the last line of each `record_id` kept, in the order of those lines;
/// and one problem per line that does not parse (a torn last line included). No file
/// is no history.
pub fn read_history(path: &Path) -> (Vec<HistoryLine>, Vec<String>) {
    let text = match std::fs::read(path) {
        Ok(bytes) => String::from_utf8_lossy(&bytes).into_owned(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return (Vec::new(), Vec::new());
        }
        Err(error) => {
            return (Vec::new(), vec![format!("{}: {error}", path.display())]);
        }
    };
    let mut lines: Vec<Option<HistoryLine>> = Vec::new();
    let mut last: HashMap<String, usize> = HashMap::new();
    let mut problems = Vec::new();
    for (n, raw) in text.lines().enumerate() {
        if raw.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<HistoryLine>(raw) {
            Ok(line) => {
                let id = record_id(&line).to_string();
                if let Some(earlier) = last.insert(id, lines.len()) {
                    lines[earlier] = None;
                }
                lines.push(Some(line));
            }
            Err(error) => problems.push(format!(
                "{} line {}: skipped: {error}",
                path.display(),
                n + 1
            )),
        }
    }
    (lines.into_iter().flatten().collect(), problems)
}

/// A record's id.
pub fn record_id(line: &HistoryLine) -> &str {
    match line {
        HistoryLine::Task(r) => &r.record_id,
        HistoryLine::Run(r) => &r.record_id,
        HistoryLine::Revert(r) => &r.record_id,
    }
}

/// `git diff --numstat` and `git diff -U0` from `from` to `to` in `root` (decision 32):
/// the files changed (a binary file counts, with no lines), the hunks (the `@@` lines),
/// and the lines added and removed. `three_dot` measures from their merge base. Only
/// commits in the user's object store are ever named.
pub fn measure_diff(
    git: &OsStr,
    root: &Path,
    from: &str,
    to: &str,
    three_dot: bool,
    timeout: Duration,
) -> Result<DiffStats, String> {
    let g = Git::new(git, timeout);
    let range = if three_dot {
        format!("{from}...{to}")
    } else {
        format!("{from}..{to}")
    };
    let diff = |extra: &[&str]| {
        let mut args = vec![os("diff"), os("--no-renames")];
        args.extend(DIFF_FLAGS.map(os));
        args.extend(extra.iter().map(|a| os(a)));
        args.push(os(&range));
        args.push(os("--"));
        g.ok(root, &args)
    };
    let mut stats = DiffStats::default();
    for line in diff(&["--numstat"])?.lines() {
        let mut fields = line.splitn(3, '\t');
        let (added, removed) = (fields.next(), fields.next());
        if fields.next().is_none() {
            continue;
        }
        stats.files += 1;
        let count = |n: Option<&str>| n.and_then(|n| n.parse::<u32>().ok()).unwrap_or(0);
        stats.added = stats.added.saturating_add(count(added));
        stats.removed = stats.removed.saturating_add(count(removed));
    }
    let patch = diff(&["-U0"])?;
    stats.hunks = patch.lines().filter(|l| l.starts_with("@@")).count() as u32;
    Ok(stats)
}

/// A `run` record of an accepted run with `accepted_commit` read from
/// `refs/heads/<base>` in `root`; any other line as it is. A failed read leaves it
/// unset (the record is still written).
pub fn fill_accepted_commit(
    git: &OsStr,
    root: &Path,
    line: HistoryLine,
    timeout: Duration,
) -> HistoryLine {
    let HistoryLine::Run(mut record) = line else {
        return line;
    };
    if record.outcome == "accepted" {
        let reference = format!("refs/heads/{}", record.base_branch);
        match read_ref(git, root, &reference, timeout) {
            Ok(commit) => record.accepted_commit = commit,
            Err(error) => {
                tracing::warn!(run = %record.run_id, %error, "could not read the accepted commit");
            }
        }
    }
    HistoryLine::Run(record)
}
