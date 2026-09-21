//! The persistence state file: `<data_dir>/state.json`. Milestone 6 brief, "State file"
//! decisions 8, 10, 12 and 13.
//!
//! [`load`] and [`save`] are blocking, filesystem-facing functions in the same spirit as
//! [`crate::logfile::RotatingFile`] — synchronous I/O, meant to be driven from
//! `tokio::task::spawn_blocking` by the daemon's lifecycle and persister (a later task).
//! Nothing in this module touches tokio or logs anything itself: `load` hands back the
//! [`Problem`]s it found, each carrying decision 12's severity, and the caller decides how
//! to log them — typically via `tracing`, at that severity. Decision 12 distinguishes
//! `error` for an unreadable file (a real, if recoverable, data-loss event: the window
//! list is gone for this boot even though the file survives on disk) from `warn` for a
//! corrupt/unsupported file or a skipped record (the module already recovered on its own).
//! A plain `String` cannot carry that distinction, so [`Problem`] mirrors `config::Problem`
//! (`crates/config/src/lib.rs`), the shape this milestone already established for exactly
//! this "here's what went wrong, and how it was handled" return value.
//!
//! `load` never panics and never returns an error: every recoverable problem — a missing
//! file, an unreadable one, corrupt bytes, a too-new version, one bad window record among
//! many good ones — downgrades to an empty (or partial) state instead of propagating,
//! because losing a user's window list on a daemon restart is strictly worse than
//! starting empty. The one thing it refuses to do silently is overwrite recoverable data:
//! a corrupt or too-new file is renamed aside, never deleted, so the bytes survive for a
//! human to look at.

use proto::{Runtime, Status};
use serde::{Deserialize, Deserializer, Serialize};
use std::collections::HashSet;
use std::fs::OpenOptions;
use std::io::{self, Write};
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The state file format this build writes, and the newest version it will load.
pub const STATE_VERSION: u32 = 2;

/// The whole state file: format version, the id the next created window gets, and the
/// windows themselves. `runs` is always empty and always serialized as `[]` in this
/// milestone — milestone 8 gives it a real element type; here it is opaque JSON so a
/// newer daemon's runs survive a round trip through an older build without this module
/// having to know their shape.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StateFile {
    pub version: u32,
    pub next_id: u32,
    #[serde(default)]
    pub windows: Vec<WindowRecord>,
    #[serde(default)]
    pub runs: Vec<serde_json::Value>,
}

/// One persisted window. Field order here is the wire order (decision 8's example, and
/// `record_json_shape` below pins it): `serde_json` emits struct fields in declaration
/// order, so reordering this struct reorders the file.
///
/// Every field beyond `id`, `name`, `runtime` and `cwd` is `#[serde(default)]` so that a
/// version-1 file (decision 13), which has none of `project`, `run` or this crate's
/// future additions, loads leniently instead of being rejected.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowRecord {
    pub id: u32,
    pub name: String,
    pub runtime: Runtime,
    pub cwd: PathBuf,
    #[serde(default)]
    pub project: Option<PathBuf>,
    #[serde(default)]
    pub worktree: Option<WorktreeRecord>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub initial_prompt: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub created_at: u64,
    #[serde(default = "exited", deserialize_with = "lenient_status")]
    pub status: Status,
    #[serde(default)]
    pub run: Option<serde_json::Value>,
}

fn exited() -> Status {
    Status::Exited
}

/// Decision 13: a version-1 (or hand-edited) file's `status` is read leniently. A known
/// value deserializes normally; anything else — an unrecognised string, a number, a
/// dropped enum variant from a future version — counts as `exited` rather than failing
/// the whole record the way a plain `#[derive(Deserialize)]` would.
fn lenient_status<'de, D>(deserializer: D) -> Result<Status, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_json::Value::deserialize(deserializer)?;
    Ok(serde_json::from_value::<Status>(value).unwrap_or(Status::Exited))
}

/// A window's worktree, as decision 8 shapes it: `null` when the window's `Entry.managed`
/// is `None`, otherwise the repo root, the worktree's own path, and its branch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorktreeRecord {
    pub repo_root: PathBuf,
    pub path: PathBuf,
    pub branch: String,
}

fn empty_state() -> StateFile {
    StateFile {
        version: STATE_VERSION,
        next_id: 1,
        windows: Vec::new(),
        runs: Vec::new(),
    }
}

/// How seriously the caller should treat a [`Problem`] (decision 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The state file could not be read at all: the user's windows are gone for this boot
    /// even though the file itself is untouched on disk wherever it was. A real, if
    /// recoverable, data-loss event — the caller should log this at `error`.
    Error,
    /// The module already recovered on its own: a corrupt or too-new file was moved aside
    /// (the bytes survive), or one bad record was skipped among otherwise-good ones. Worth
    /// a human's attention, not urgent — the caller should log this at `warn`.
    Warn,
}

/// One thing [`load`] noticed while reading the state file. Mirrors `config::Problem`
/// (`crates/config/src/lib.rs`) — a struct the caller formats or matches on, not a bare
/// `String` — with the [`Severity`] decision 12 needs added, since a plain string cannot
/// carry it and this module does not call `tracing` itself to supply another way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    pub severity: Severity,
    /// What the problem is about: `"<state file>"` for the file as a whole (unreadable,
    /// corrupt, unsupported version), or `"windows[<index>]"` for one bad record.
    pub key: String,
    pub message: String,
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.key, self.message)
    }
}

/// Reads and validates the state file at `path`, returning the state to run with and any
/// [`Problem`]s the caller should log. See the module doc for the "never panics, never
/// errors, never silently destroys recoverable bytes" contract.
///
/// Decision 12's steps, in order: a missing file starts empty with no problems; any other
/// read error starts empty too but leaves the file exactly where it was, reported as
/// [`Severity::Error`]; invalid JSON, a non-object, or a missing/non-integer `version` or
/// `next_id` counts as corrupt and gets renamed to `<path>.corrupt-<unix-seconds>` (with a
/// `-1`, `-2`, ... suffix added until the name is free); a `version` newer than
/// [`STATE_VERSION`] is renamed the same way to `<path>.unsupported-<unix-seconds>`,
/// protecting a newer daemon's file from an older one; otherwise each entry of `windows`
/// is deserialized on its own, so one bad record — an unknown runtime, or one that repeats
/// an earlier entry's id or name — is skipped and reported instead of failing the whole
/// load. Every case past "missing file" is [`Severity::Warn`]: the module already
/// recovered on its own.
pub fn load(path: &Path) -> (StateFile, Vec<Problem>) {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (empty_state(), Vec::new()),
        Err(e) => {
            return (
                empty_state(),
                vec![Problem {
                    severity: Severity::Error,
                    key: "<state file>".to_string(),
                    message: format!(
                        "could not read {}: {e}; starting empty and leaving the file alone",
                        path.display()
                    ),
                }],
            );
        }
    };

    let value: serde_json::Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(e) => {
            return mark_aside(
                path,
                "corrupt",
                &format!("state file is not valid JSON: {e}"),
            );
        }
    };

    let Some(object) = value.as_object() else {
        return mark_aside(path, "corrupt", "state file is not a JSON object");
    };

    let Some(version) = object.get("version").and_then(serde_json::Value::as_u64) else {
        return mark_aside(
            path,
            "corrupt",
            "state file is missing an integer \"version\"",
        );
    };
    let Some(saved_next_id) = object.get("next_id").and_then(serde_json::Value::as_u64) else {
        return mark_aside(
            path,
            "corrupt",
            "state file is missing an integer \"next_id\"",
        );
    };
    // A saved value past `u32::MAX` cannot be represented by `StateFile::next_id`
    // (`u32`, per decision 8) any more faithfully than by saturating: there is no
    // narrower valid id to fall back to, and silently truncating is exactly the bug
    // this module must not repeat (see the `windows` loop below).
    let saved_next_id = u32::try_from(saved_next_id).unwrap_or(u32::MAX);

    if version > u64::from(STATE_VERSION) {
        return mark_aside(
            path,
            "unsupported",
            &format!("state file version {version} is newer than {STATE_VERSION}"),
        );
    }

    // `windows` missing entirely is legitimate (the field is `#[serde(default)]`, e.g. a
    // hand-written file that only sets `version`/`next_id`) and must load silently. A
    // `windows` key that *is* present but is not a JSON array is structurally invalid,
    // not defaultable — treated as corrupt like every other malformed part of this file,
    // not silently swallowed into an empty list with zero warnings.
    let raw_windows: &[serde_json::Value] = match object.get("windows") {
        None => &[],
        Some(serde_json::Value::Array(items)) => items,
        Some(_) => {
            return mark_aside(
                path,
                "corrupt",
                "state file's \"windows\" field is not an array",
            );
        }
    };

    let mut windows = Vec::new();
    let mut problems = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut seen_names = HashSet::new();
    for (index, entry) in raw_windows.iter().enumerate() {
        match serde_json::from_value::<WindowRecord>(entry.clone()) {
            Ok(record) => {
                // Both uniqueness checks run *before* either `HashSet` is touched: a
                // record that is ultimately rejected must never claim the id or the
                // name it arrived with, or a later, legitimate record that reuses that
                // id/name would be wrongly rejected as a duplicate even though nothing
                // surviving holds it.
                if seen_ids.contains(&record.id) {
                    problems.push(Problem {
                        severity: Severity::Warn,
                        key: format!("windows[{index}]"),
                        message: format!("id {} repeats an earlier entry's id; skipped", record.id),
                    });
                    continue;
                }
                if seen_names.contains(&record.name) {
                    problems.push(Problem {
                        severity: Severity::Warn,
                        key: format!("windows[{index}]"),
                        message: format!(
                            "name {:?} repeats an earlier entry's name; skipped",
                            record.name
                        ),
                    });
                    continue;
                }
                seen_ids.insert(record.id);
                seen_names.insert(record.name.clone());
                windows.push(record);
            }
            Err(e) => {
                problems.push(Problem {
                    severity: Severity::Warn,
                    key: format!("windows[{index}]"),
                    message: format!("invalid ({e}); skipped"),
                });
            }
        }
    }

    // `saturating_add` rather than `+ 1`: a loaded record already holding `u32::MAX`
    // has no valid successor id, and wrapping to 0 (as an unchecked cast from wider
    // arithmetic would) would immediately collide with the lowest live id. Saturating
    // at `u32::MAX` instead means `next_id` ends up equal to that already-loaded id -
    // deliberately, since there is no other representable choice - so the next
    // window-creation attempt must notice the collision and fail loudly rather than
    // silently reuse it. Nothing in this module creates windows; that check belongs to
    // whichever later milestone wires `next_id` up to window creation.
    let next_id = match windows.iter().map(|w| w.id).max() {
        Some(max_id) => saved_next_id.max(max_id.saturating_add(1)),
        None => saved_next_id,
    };

    let runs = object
        .get("runs")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    (
        StateFile {
            version: STATE_VERSION,
            next_id,
            windows,
            runs,
        },
        problems,
    )
}

/// Renames `path` aside (decision 12) and returns an empty state plus one [`Severity::Warn`]
/// problem naming the new path. The rename itself failing (permissions, a concurrent
/// deletion) is reported instead of panicking, and still leaves the caller with an empty
/// state rather than a corrupt one it would have to guard against everywhere else.
fn mark_aside(path: &Path, tag: &str, why: &str) -> (StateFile, Vec<Problem>) {
    let aside = aside_path(path, tag);
    let message = match std::fs::rename(path, &aside) {
        Ok(()) => format!("{why}; moved it aside to {}", aside.display()),
        Err(e) => format!(
            "{why}; could not move it aside to {} ({e}); starting empty and leaving it in place",
            aside.display()
        ),
    };
    (
        empty_state(),
        vec![Problem {
            severity: Severity::Warn,
            key: "<state file>".to_string(),
            message,
        }],
    )
}

/// Picks `<path>.<tag>-<unix-seconds>`, or `-1`, `-2`, ... appended to that if it is
/// already taken. The suffix search — not the timestamp alone — is what makes two corrupt
/// loads inside the same wall-clock second land on different names.
fn aside_path(path: &Path, tag: &str) -> PathBuf {
    let ts = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let base = format!("{}.{tag}-{ts}", path.display());
    let candidate = PathBuf::from(&base);
    if !candidate.exists() {
        return candidate;
    }
    let mut n = 1u32;
    loop {
        let candidate = PathBuf::from(format!("{base}-{n}"));
        if !candidate.exists() {
            return candidate;
        }
        n += 1;
    }
}

/// Serializes `state` and atomically replaces `path` with it (decision 10). Blocking.
///
/// Writes `<path>.tmp` created with mode `0o600` (prompts in `initial_prompt` can be
/// private), calls `sync_all` on it, renames it over `path`, then opens `path`'s parent
/// directory and calls `sync_all` on *that* — the step most often skipped, and the one
/// that makes the rename itself durable across a power failure. A leftover `.tmp` from an
/// earlier crashed run is overwritten, not treated as an error: the temp file is opened
/// with `truncate(true)`, not created exclusively. Because `OpenOptionsExt::mode` only
/// applies when `open` actually creates the file — not when it reuses that leftover one —
/// the permissions are set explicitly after opening too, so a leftover `.tmp` with looser
/// permissions from a previous run still ends up private.
pub fn save(path: &Path, state: &StateFile) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&tmp_path)?;
    #[cfg(unix)]
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);

    std::fs::rename(&tmp_path, path)?;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let dir_file = std::fs::File::open(dir)?;
    dir_file.sync_all()?;

    Ok(())
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
