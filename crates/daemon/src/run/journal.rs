//! Decision 43, the I/O side (M8a.21): each run's `run.json` and its intent journal
//! `journal.jsonl`, under `<data_dir>/runs/<run>/`.
//!
//! Does I/O (design decision 2), all of it **blocking**: call it only from
//! `spawn_blocking` or before the runtime serves anything (decision 44's restore).
//!
//! **Durability.** Every write here is followed by `sync_all` on the file written, and
//! every rename or file creation by `sync_all` on its directory:
//! - [`save_run`]: `run.json.tmp` written and `sync_all`ed, renamed over `run.json`,
//!   the run's directory `sync_all`ed (and `runs/` when the run's directory is new).
//! - [`append`]: a torn tail left by a crash is first cut back to the last `\n` and
//!   `sync_all`ed; then one line in one `write_all`, then `sync_all`; the directory is
//!   `sync_all`ed when the journal file was created by this append.
//! - [`compact`]: `journal.jsonl.tmp` written and `sync_all`ed, renamed, the directory
//!   `sync_all`ed.
//!
//! **What a crash leaves.** A `run.json.tmp` is a save that never reached its rename;
//! [`load_all`] ignores and removes it, and `run.json` is the last complete save. A
//! journal whose last line has no `\n` was torn mid-append; that line is dropped with a
//! problem (its op then has no `intent`, or no `done`, which decision 44 handles).
//!
//! The journal is not safe against concurrent writers: the driver (M8a.22) appends and
//! compacts one run's journal from one place at a time.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::FileExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::engine::{OpKind, OpResult};
use super::model::{OpId, PendingOp, Run};

/// The journal is rewritten with only its pending ops once it passes this (decision 43).
pub const COMPACT_AFTER_BYTES: u64 = 1 << 20;

pub const RUNS_DIR: &str = "runs";
pub const RUN_FILE: &str = "run.json";
pub const RUN_TMP: &str = "run.json.tmp";
pub const JOURNAL_FILE: &str = "journal.jsonl";
const JOURNAL_TMP: &str = "journal.jsonl.tmp";

/// One journal line: `{"op":<id>,"intent":{…OpKind…}}` before an op runs, and
/// `{"op":<id>,"done":{…OpResult…}}` once it has returned (decision 43).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum JournalLine {
    Intent {
        op: OpId,
        #[serde(rename = "intent")]
        kind: OpKind,
    },
    Done {
        op: OpId,
        #[serde(rename = "done")]
        result: OpResult,
    },
}

impl JournalLine {
    pub fn op(&self) -> OpId {
        match self {
            JournalLine::Intent { op, .. } | JournalLine::Done { op, .. } => *op,
        }
    }
}

/// `<data_dir>/runs`.
pub fn runs_dir(data_dir: &Path) -> PathBuf {
    data_dir.join(RUNS_DIR)
}

/// Writes the whole run to `<run.data_dir>/run.json`: a temp file, `sync_all`, a
/// rename, then the directory's `sync_all`. A crash at any point leaves either the old
/// `run.json` or the new one, never a torn one.
pub fn save_run(run: &Run) -> io::Result<()> {
    let dir = &run.data_dir;
    ensure_dir(dir)?;
    let bytes = serde_json::to_vec(run).map_err(io::Error::other)?;
    let tmp = dir.join(RUN_TMP);
    write_synced(&tmp, &bytes)?;
    fs::rename(&tmp, dir.join(RUN_FILE))?;
    sync_dir(dir)
}

/// Appends one line to `<dir>/journal.jsonl` and `sync_all`s it before returning.
pub fn append(dir: &Path, line: &JournalLine) -> io::Result<()> {
    ensure_dir(dir)?;
    let path = dir.join(JOURNAL_FILE);
    let created = !path.exists();
    let mut bytes = serde_json::to_vec(line).map_err(io::Error::other)?;
    bytes.push(b'\n');
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&path)?;
    heal_torn_tail(&mut file)?;
    // One `write_all` of the whole line: a crash tears at most this line, which
    // `load_all` recognises by its missing `\n`.
    file.write_all(&bytes)?;
    file.sync_all()?;
    if created {
        sync_dir(dir)?;
    }
    Ok(())
}

/// Fix round 1 (I2): a journal whose last byte is not `\n` ends in a line a crash (or a
/// failed write) tore. Appending after it would glue the new line onto the fragment and
/// lose both, so the fragment is cut back to the last `\n` first, and the cut
/// `sync_all`ed. `load_all` has already reported the torn line as a problem.
fn heal_torn_tail(file: &mut File) -> io::Result<()> {
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(());
    }
    let mut last = [0u8; 1];
    file.read_exact_at(&mut last, len - 1)?;
    if last[0] == b'\n' {
        return Ok(());
    }
    let mut bytes = Vec::with_capacity(len as usize);
    file.seek(SeekFrom::Start(0))?;
    file.read_to_end(&mut bytes)?;
    let keep = bytes
        .iter()
        .rposition(|b| *b == b'\n')
        .map_or(0, |at| at as u64 + 1);
    file.set_len(keep)?;
    file.sync_all()
}

/// Whether `<dir>/journal.jsonl` has passed [`COMPACT_AFTER_BYTES`].
pub fn needs_compact(dir: &Path) -> bool {
    fs::metadata(dir.join(JOURNAL_FILE)).is_ok_and(|m| m.len() > COMPACT_AFTER_BYTES)
}

/// Rewrites `<dir>/journal.jsonl` with the lines of the ops still pending: each one's
/// `intent`, from `pending` (the persisted `run.json`'s, which is authoritative), then
/// any `done` line the journal already has for it. Keeping that `done` is M8a.21's
/// addition to decision 43's "only pending ops' intents": a result written between the
/// op's return and the `Persist` that retires it is replayed rather than re-checked.
/// Temp file, `sync_all`, rename, directory `sync_all`. Returns the problems the old
/// journal had (a torn or unparseable line), whose lines the rewrite drops; the driver
/// logs them (fix round 1, m3).
pub fn compact(dir: &Path, pending: &BTreeMap<OpId, PendingOp>) -> io::Result<Vec<String>> {
    let (existing, problems) = read_journal(&dir.join(JOURNAL_FILE))?;
    let mut bytes = Vec::new();
    for (op, p) in pending {
        let intent = JournalLine::Intent {
            op: *op,
            kind: p.kind.clone(),
        };
        push_line(&mut bytes, &intent)?;
    }
    for line in existing {
        if let JournalLine::Done { op, .. } = &line
            && pending.contains_key(op)
        {
            push_line(&mut bytes, &line)?;
        }
    }
    let tmp = dir.join(JOURNAL_TMP);
    write_synced(&tmp, &bytes)?;
    fs::rename(&tmp, dir.join(JOURNAL_FILE))?;
    sync_dir(dir)?;
    Ok(problems)
}

/// Every run under `<data_dir>/runs/`, in directory-name order, with its journal, and
/// the problems found on the way. A run whose `run.json` is missing or does not parse
/// is skipped with a problem; so is a journal line that does not parse, and a torn last
/// line (no `\n`). A leftover `run.json.tmp` or `journal.jsonl.tmp` is removed.
pub fn load_all(data_dir: &Path) -> (Vec<(Run, Vec<JournalLine>)>, Vec<String>) {
    let mut runs = Vec::new();
    let mut problems = Vec::new();
    let root = runs_dir(data_dir);
    let entries = match fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return (runs, problems),
        Err(err) => {
            problems.push(format!("cannot read {}: {err}", root.display()));
            return (runs, problems);
        }
    };
    let mut dirs: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    for dir in dirs {
        for leftover in [RUN_TMP, JOURNAL_TMP] {
            let path = dir.join(leftover);
            if path.exists()
                && let Err(err) = fs::remove_file(&path)
            {
                problems.push(format!("cannot remove {}: {err}", path.display()));
            }
        }
        let run_file = dir.join(RUN_FILE);
        let mut run: Run = match fs::read(&run_file) {
            Ok(bytes) => match serde_json::from_slice(&bytes) {
                Ok(run) => run,
                Err(err) => {
                    problems.push(format!(
                        "run {} skipped: {} does not parse: {err}",
                        name(&dir),
                        run_file.display()
                    ));
                    continue;
                }
            },
            Err(err) => {
                problems.push(format!(
                    "run {} skipped: cannot read {}: {err}",
                    name(&dir),
                    run_file.display()
                ));
                continue;
            }
        };
        // Final review B-12: a run lives where it is found. A directory whose name is
        // not its run's id is a copy, and would load as a second run of the same id.
        if name(&dir) != run.id {
            problems.push(format!(
                "run {} skipped: its run.json is run {}'s",
                name(&dir),
                run.id
            ));
            continue;
        }
        run.data_dir = dir.clone();
        let journal_file = dir.join(JOURNAL_FILE);
        match read_journal(&journal_file) {
            Ok((lines, found)) => {
                problems.extend(found);
                runs.push((run, lines));
            }
            Err(err) => {
                problems.push(format!(
                    "run {}: cannot read {}: {err}; reconciling without it",
                    name(&dir),
                    journal_file.display()
                ));
                runs.push((run, Vec::new()));
            }
        }
    }
    (runs, problems)
}

fn name(dir: &Path) -> String {
    dir.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| dir.display().to_string())
}

/// The journal's parsed lines and the problems: a missing file is an empty journal.
fn read_journal(path: &Path) -> io::Result<(Vec<JournalLine>, Vec<String>)> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok((Vec::new(), Vec::new())),
        Err(err) => return Err(err),
    };
    let mut lines = Vec::new();
    let mut problems = Vec::new();
    let mut rest: &[u8] = &bytes;
    let mut number = 0;
    while !rest.is_empty() {
        number += 1;
        let Some(end) = rest.iter().position(|b| *b == b'\n') else {
            problems.push(format!(
                "{}: line {number} is torn (no newline; a crash mid-append) and was dropped",
                path.display()
            ));
            break;
        };
        let line = &rest[..end];
        rest = &rest[end + 1..];
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        match serde_json::from_slice::<JournalLine>(line) {
            Ok(parsed) => lines.push(parsed),
            Err(err) => problems.push(format!(
                "{}: line {number} does not parse and was dropped: {err}",
                path.display()
            )),
        }
    }
    Ok((lines, problems))
}

fn push_line(bytes: &mut Vec<u8>, line: &JournalLine) -> io::Result<()> {
    serde_json::to_writer(&mut *bytes, line).map_err(io::Error::other)?;
    bytes.push(b'\n');
    Ok(())
}

/// Creates `dir` (and its parents) if missing; a new directory's parent is `sync_all`ed
/// so the new entry survives a crash.
fn ensure_dir(dir: &Path) -> io::Result<()> {
    if dir.is_dir() {
        return Ok(());
    }
    fs::create_dir_all(dir)?;
    // `runs/<id>` may be new, and `runs/` with it.
    for parent in dir.ancestors().skip(1).take(2) {
        if parent.is_dir() {
            sync_dir(parent)?;
        }
    }
    Ok(())
}

/// Writes `bytes` to `path` (truncating) and `sync_all`s it.
fn write_synced(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = File::create(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// `sync_all` on a directory, so a rename or a new entry in it is durable.
fn sync_dir(dir: &Path) -> io::Result<()> {
    File::open(dir)?.sync_all()
}
