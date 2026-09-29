//! M8b decisions 32 and 33: `history.jsonl` on disk, and the diff measurement; M8b.17,
//! decisions 34 and 35: revert detection, and `run stats`' read. Blocking
//! I/O (M8b decision 1): the driver calls every function here on `spawn_blocking`,
//! never under a lock. Every git call goes through `run::git::Git` (`worktree::run_git`'s
//! code: `--no-optional-locks`, a scrubbed environment, hooks off, a deadline).

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::io::{BufRead, Write};
use std::path::Path;
use std::time::Duration;

use proto::{DiffStats, HISTORY_VERSION, HistoryLine, HistoryStats, RevertRecord};

use super::git::{DIFF_FLAGS, Git, os, read_ref};

/// Appends `line` as one JSON line, then `sync_all`. The file and its directory are
/// created when missing. The line is written with one `write_all` of the whole line,
/// so a reader sees it whole or torn at its end, never interleaved. A file whose last
/// line was torn (a crash or a full disk mid-append) gets a `\n` in front of the line,
/// in the same write, so the torn line cannot swallow it (M8b.17 review, m7).
pub fn append_line(path: &Path, line: &HistoryLine) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut text = serde_json::to_string(line).map_err(std::io::Error::other)?;
    text.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    if ends_torn(&mut file)? {
        text.insert(0, '\n');
    }
    file.write_all(text.as_bytes())?;
    file.sync_all()
}

/// Milestone 9 decision 43, for a record the driver writes itself (pre-run triage's):
/// `line` appended with [`append_line`] unless the file holds its record already.
/// True when it was appended. Blocking: the driver calls it on `spawn_blocking`.
pub fn append_once(path: &Path, line: &HistoryLine) -> std::io::Result<bool> {
    if contains_record(path, record_id(line))? {
        return Ok(false);
    }
    append_line(path, line).map(|()| true)
}

/// Whether `file` is non-empty and its last byte is not `\n`.
fn ends_torn(file: &mut std::fs::File) -> std::io::Result<bool> {
    use std::io::{Read, Seek, SeekFrom};
    if file.metadata()?.len() == 0 {
        return Ok(false);
    }
    file.seek(SeekFrom::End(-1))?;
    let mut last = [0u8; 1];
    file.read_exact(&mut last)?;
    Ok(last[0] != b'\n')
}

/// How [`read_history`]'s problem for a file it could not read starts (M8b.17 review,
/// m6): `run stats` shows it as it is, never as a skipped line.
pub const UNREADABLE: &str = "could not read ";

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
            return (
                Vec::new(),
                vec![format!("{UNREADABLE}{}: {error}", path.display())],
            );
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
            Err(error) => problems.push(format!("{} line {}: {error}", path.display(), n + 1)),
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
        HistoryLine::RoleRoute(r) => &r.record_id,
        HistoryLine::Tier(r) => &r.record_id,
        HistoryLine::Flaky(r) => &r.record_id,
        HistoryLine::Bisect(r) => &r.record_id,
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

/// Decision 34: only accepts at most this old are looked for.
pub const REVERT_WINDOW_SECS: u64 = 90 * 86_400;

/// Decision 34: how many commits of the base branch are read.
pub const REVERT_LOG_MAX: u32 = 2000;

/// `git revert`'s standard message names the reverted commit after this.
const REVERTS_MARKER: &str = "This reverts commit ";

/// A full commit id: 40 (SHA-1) or 64 (SHA-256) lowercase hex digits.
fn is_commit_id(id: &str) -> bool {
    matches!(id.len(), 40 | 64) && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The shortest abbreviated sha matched: git's default abbreviation length.
const MIN_ABBREV: usize = 7;

/// The candidate `sha` names: itself when it is one, else the one candidate it is an
/// abbreviation of (at least [`MIN_ABBREV`] digits: `git revert --reference`'s
/// `This reverts commit 75a8dda (subject, date).`). An ambiguous abbreviation is none
/// (M8b.17 review, m2).
fn candidate_of<'m, 'a>(
    wanted: &'m HashMap<&'a str, (&'a str, Option<&'a str>)>,
    sha: &str,
) -> Option<(&'m &'a str, &'m (&'a str, Option<&'a str>))> {
    let sha = sha.to_ascii_lowercase();
    if let Some(hit) = wanted.get_key_value(sha.as_str()) {
        return Some(hit);
    }
    if sha.len() < MIN_ABBREV {
        return None;
    }
    let mut hits = wanted.iter().filter(|(full, _)| full.starts_with(&sha));
    let hit = hits.next()?;
    hits.next().is_none().then_some(hit)
}

/// The shas a commit message says it reverts.
fn reverted_shas(body: &str) -> impl Iterator<Item = &str> {
    body.match_indices(REVERTS_MARKER).map(|(at, marker)| {
        let rest = &body[at + marker.len()..];
        let end = rest
            .find(|c: char| !c.is_ascii_hexdigit())
            .unwrap_or(rest.len());
        &rest[..end]
    })
}

/// Decision 34's candidates on `base_branch`: each accepted run's accept merge
/// (`task_id: None`) and its merged tasks' merge commits, for runs accepted into that
/// branch at most [`REVERT_WINDOW_SECS`] before `now`, less every commit a revert
/// record already names. Keyed by sha: `(run, task)`.
fn candidates<'a>(
    history: &'a [HistoryLine],
    base_branch: &str,
    now: u64,
) -> HashMap<&'a str, (&'a str, Option<&'a str>)> {
    let mut runs = HashSet::new();
    let mut shas = HashMap::new();
    for line in history {
        if let HistoryLine::Run(r) = line
            && r.base_branch == base_branch
            && now.saturating_sub(r.at) <= REVERT_WINDOW_SECS
            && let Some(accepted) = r.accepted_commit.as_deref()
        {
            runs.insert(r.run_id.as_str());
            shas.insert(accepted, (r.run_id.as_str(), None));
        }
    }
    for line in history {
        if let HistoryLine::Task(t) = line
            && runs.contains(t.run_id.as_str())
            && let Some(merge) = t.merge_commit.as_deref()
        {
            shas.insert(merge, (t.run_id.as_str(), Some(t.task_id.as_str())));
        }
    }
    for line in history {
        if let HistoryLine::Revert(r) = line {
            shas.remove(r.reverted.as_str());
        }
    }
    shas
}

/// Decision 34: the commits on `base_branch` (its newest [`REVERT_LOG_MAX`], through
/// `run::git::Git`: `--no-optional-locks`, the scrubbed environment, hooks off, the
/// deadline) whose message says `This reverts commit <sha>` of a candidate (see
/// [`candidates`]; `<sha>` in full or abbreviated, [`candidate_of`]), oldest first, one record each (`revert/<revert commit>`; a commit
/// already recorded is skipped). Only reads: it never writes to the repository or any
/// ref. A failed read (a branch that does not exist included) is an error.
pub fn detect_reverts(
    git: &OsStr,
    root: &Path,
    base_branch: &str,
    history: &[HistoryLine],
    now: u64,
    timeout: Duration,
) -> Result<Vec<RevertRecord>, String> {
    let wanted = candidates(history, base_branch, now);
    if wanted.is_empty() {
        return Ok(Vec::new());
    }
    let recorded: HashSet<&str> = history.iter().map(record_id).collect();
    let limit = format!("-n{REVERT_LOG_MAX}");
    // `refs/heads/` in front: a branch name from the file can never read as an option.
    let branch = format!("refs/heads/{base_branch}");
    let log = Git::new(git, timeout).ok(
        root,
        &[
            os("log"),
            os(&limit),
            os("--no-show-signature"),
            // NUL cannot occur in a commit message, so a message cannot make up a
            // record (M8b.17 review, m1).
            os("--format=%H%x1f%B%x00"),
            os(&branch),
            os("--"),
        ],
    )?;
    let mut found = Vec::new();
    for entry in log.split('\0') {
        let Some((commit, body)) = entry.trim_start().split_once('\x1f') else {
            continue;
        };
        if !is_commit_id(commit) {
            continue;
        }
        let record_id = format!("revert/{commit}");
        if recorded.contains(record_id.as_str()) {
            continue;
        }
        let hit = reverted_shas(body).find_map(|sha| candidate_of(&wanted, sha));
        if let Some((reverted, &(run_id, task_id))) = hit {
            found.push(RevertRecord {
                v: HISTORY_VERSION,
                record_id,
                at: now,
                run_id: run_id.to_string(),
                task_id: task_id.map(str::to_string),
                reverted: reverted.to_string(),
                revert_commit: commit.to_string(),
            });
        }
    }
    found.reverse();
    Ok(found)
}

/// Decision 34 at `run start` and `run stats`: [`detect_reverts`] on each base branch
/// an accepted run of the history names, each record appended with [`append_line`].
/// Every failure is one warning returned, never an error: history never gates anything.
pub fn record_reverts(
    git: &OsStr,
    root: &Path,
    path: &Path,
    now: u64,
    timeout: Duration,
) -> Vec<String> {
    let (history, _) = read_history(path);
    let mut branches: Vec<&str> = history
        .iter()
        .filter_map(|line| match line {
            HistoryLine::Run(r) if r.accepted_commit.is_some() => Some(r.base_branch.as_str()),
            _ => None,
        })
        .collect();
    branches.sort_unstable();
    branches.dedup();
    let mut warnings = Vec::new();
    for branch in branches {
        match detect_reverts(git, root, branch, &history, now, timeout) {
            Ok(records) => {
                for record in records {
                    let id = record.record_id.clone();
                    if let Err(error) = append_line(path, &HistoryLine::Revert(record)) {
                        warnings.push(format!("revert record {id} was not written: {error}"));
                    }
                }
            }
            Err(error) => warnings.push(format!("could not look for reverts on {branch}: {error}")),
        }
    }
    warnings
}

/// Decision 35's blocking core: reverts recorded first ([`record_reverts`], each
/// warning logged), then the history read once more and aggregated, with that read's
/// skipped lines as the problems (so each is reported once).
pub fn summarise(
    git: &OsStr,
    root: &Path,
    path: &Path,
    now: u64,
    timeout: Duration,
) -> HistoryStats {
    for warning in record_reverts(git, root, path, now, timeout) {
        tracing::warn!(path = %path.display(), %warning, "revert detection");
    }
    let (lines, problems) = read_history(path);
    let mut stats = super::stats::aggregate(&lines, path);
    stats.problems = problems;
    stats
}
