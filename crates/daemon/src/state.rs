//! The persistence state file: `<data_dir>/state.json`. Milestone 6 brief, "State file"
//! decisions 8, 10, 12 and 13.
//!
//! [`load`] and [`save`] are blocking, filesystem-facing functions in the same spirit as
//! [`crate::logfile::RotatingFile`] — synchronous I/O, meant to be driven from
//! `tokio::task::spawn_blocking` by the daemon's lifecycle and persister (a later task).
//! Nothing in this module touches tokio or logs anything itself: `load` hands back the
//! warnings it would like logged as plain strings, and the caller decides how (decision
//! 12 distinguishes "log at error" for an unreadable file from "log at warn" for a
//! corrupt/unsupported file or a skipped record, but that distinction is the caller's to
//! make — see the module's tests for exactly which case produces which message).
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

/// Reads and validates the state file at `path`, returning the state to run with and any
/// warnings the caller should log. See the module doc for the "never panics, never
/// errors, never silently destroys recoverable bytes" contract.
///
/// Decision 12's steps, in order: a missing file starts empty with no warnings; any other
/// read error starts empty too but leaves the file exactly where it was; invalid JSON, a
/// non-object, or a missing/non-integer `version` or `next_id` counts as corrupt and gets
/// renamed to `<path>.corrupt-<unix-seconds>` (with a `-1`, `-2`, ... suffix added until
/// the name is free); a `version` newer than [`STATE_VERSION`] is renamed the same way to
/// `<path>.unsupported-<unix-seconds>`, protecting a newer daemon's file from an older
/// one; otherwise each entry of `windows` is deserialized on its own, so one bad record —
/// an unknown runtime, or one that repeats an earlier entry's id or name — is skipped and
/// warned about instead of failing the whole load.
pub fn load(path: &Path) -> (StateFile, Vec<String>) {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (empty_state(), Vec::new()),
        Err(e) => {
            return (
                empty_state(),
                vec![format!(
                    "could not read state file {}: {e}; starting empty and leaving the file alone",
                    path.display()
                )],
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

    if version > u64::from(STATE_VERSION) {
        return mark_aside(
            path,
            "unsupported",
            &format!("state file version {version} is newer than {STATE_VERSION}"),
        );
    }

    let mut windows = Vec::new();
    let mut warnings = Vec::new();
    let mut seen_ids = HashSet::new();
    let mut seen_names = HashSet::new();
    let raw_windows = object.get("windows").and_then(serde_json::Value::as_array);
    for (index, entry) in raw_windows.into_iter().flatten().enumerate() {
        match serde_json::from_value::<WindowRecord>(entry.clone()) {
            Ok(record) => {
                if !seen_ids.insert(record.id) {
                    warnings.push(format!(
                        "state file window[{index}] (id {}) repeats an earlier entry's id; skipped",
                        record.id
                    ));
                    continue;
                }
                if !seen_names.insert(record.name.clone()) {
                    warnings.push(format!(
                        "state file window[{index}] ({:?}) repeats an earlier entry's name; skipped",
                        record.name
                    ));
                    continue;
                }
                windows.push(record);
            }
            Err(e) => {
                warnings.push(format!(
                    "state file window[{index}] is invalid ({e}); skipped"
                ));
            }
        }
    }

    let next_id = match windows.iter().map(|w| w.id).max() {
        Some(max_id) => saved_next_id.max(u64::from(max_id) + 1),
        None => saved_next_id,
    };

    let runs = object
        .get("runs")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    (
        StateFile {
            version: STATE_VERSION,
            #[allow(clippy::cast_possible_truncation)]
            next_id: next_id as u32,
            windows,
            runs,
        },
        warnings,
    )
}

/// Renames `path` aside (decision 12) and returns an empty state plus one warning naming
/// the new path. The rename itself failing (permissions, a concurrent deletion) is
/// reported instead of panicking, and still leaves the caller with an empty state rather
/// than a corrupt one it would have to guard against everywhere else.
fn mark_aside(path: &Path, tag: &str, why: &str) -> (StateFile, Vec<String>) {
    let aside = aside_path(path, tag);
    let warning = match std::fs::rename(path, &aside) {
        Ok(()) => format!("{why}; moved it aside to {}", aside.display()),
        Err(e) => format!(
            "{why}; could not move it aside to {} ({e}); starting empty and leaving it in place",
            aside.display()
        ),
    };
    (empty_state(), vec![warning])
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
