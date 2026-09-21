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
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio_util::sync::CancellationToken;

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
/// [`Severity::Error`]; invalid JSON, a non-object, a missing/non-integer `version` or
/// `next_id`, or a present-but-wrong-typed `windows` or `runs` counts as corrupt and gets
/// renamed to `<path>.corrupt-<unix-seconds>` (with a `-1`, `-2`, ... suffix added until
/// the name is free); a `version` newer than [`STATE_VERSION`] is renamed the same way to
/// `<path>.unsupported-<unix-seconds>`, protecting a newer daemon's file from an older
/// one; otherwise each entry of `windows` is deserialized on its own, so one bad record —
/// an unknown runtime, or one that repeats an earlier entry's id or name — is skipped and
/// reported instead of failing the whole load. Every case past "missing file" is
/// [`Severity::Warn`]: the module already recovered on its own.
pub fn load(path: &Path) -> (StateFile, Vec<Problem>) {
    load_with(path, SystemTime::now())
}

/// Injectable twin of [`load`]: `now` is the clock a corrupt/unsupported file's
/// `mark_aside` rename timestamps itself with, following the same pattern as
/// `project::detect_roots_with` — a real caller always uses [`load`], and tests drive
/// `now` directly so a same-second collision (or its absence) is a property of the
/// input instead of a race against the wall clock.
pub fn load_with(path: &Path, now: SystemTime) -> (StateFile, Vec<Problem>) {
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
            return mark_aside_with(
                path,
                "corrupt",
                &format!("state file is not valid JSON: {e}"),
                now,
            );
        }
    };

    let Some(object) = value.as_object() else {
        return mark_aside_with(path, "corrupt", "state file is not a JSON object", now);
    };

    let Some(version) = object.get("version").and_then(serde_json::Value::as_u64) else {
        return mark_aside_with(
            path,
            "corrupt",
            "state file is missing an integer \"version\"",
            now,
        );
    };
    let Some(saved_next_id) = object.get("next_id").and_then(serde_json::Value::as_u64) else {
        return mark_aside_with(
            path,
            "corrupt",
            "state file is missing an integer \"next_id\"",
            now,
        );
    };
    let mut problems = Vec::new();

    // A saved value past `u32::MAX` cannot be represented by `StateFile::next_id`
    // (`u32`, per decision 8) any more faithfully than by saturating: there is no
    // narrower valid id to fall back to, and silently truncating is exactly the bug
    // this module must not repeat (see the `windows` loop below). Decision 12: the
    // module already recovered on its own, but the loaded state still differs from what
    // the file said, silently, unless this is reported — see Minor #3.
    let saved_next_id = match u32::try_from(saved_next_id) {
        Ok(v) => v,
        Err(_) => {
            problems.push(Problem {
                severity: Severity::Warn,
                key: "next_id".to_string(),
                message: format!(
                    "saved next_id {saved_next_id} does not fit in a u32; using {}",
                    u32::MAX
                ),
            });
            u32::MAX
        }
    };

    if version > u64::from(STATE_VERSION) {
        return mark_aside_with(
            path,
            "unsupported",
            &format!("state file version {version} is newer than {STATE_VERSION}"),
            now,
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
            return mark_aside_with(
                path,
                "corrupt",
                "state file's \"windows\" field is not an array",
                now,
            );
        }
    };

    // Minor #6: `runs` gets exactly the treatment `windows` above just got, for the same
    // reason — a present-but-wrong-typed field is structurally invalid, not defaultable,
    // and must not be silently swallowed into an empty list with zero warnings. `runs` is
    // always `[]` this milestone and its element type is opaque `serde_json::Value` (see
    // the `StateFile` doc), so any JSON *array* is already valid content — there is
    // nothing per-element left to validate the way the `windows` loop below validates
    // each `WindowRecord`.
    let runs: Vec<serde_json::Value> = match object.get("runs") {
        None => Vec::new(),
        Some(serde_json::Value::Array(items)) => items.clone(),
        Some(_) => {
            return mark_aside_with(
                path,
                "corrupt",
                "state file's \"runs\" field is not an array",
                now,
            );
        }
    };

    let mut windows = Vec::new();
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
    // arithmetic would) would immediately collide with the lowest live id. Saturating at
    // `u32::MAX` instead means `next_id` ends up equal to that already-loaded id -
    // deliberately, since there is no other representable choice. Nothing in this module
    // creates windows, so it cannot refuse the collision itself — that refusal is
    // `manager::create`'s `admit` (`crates/daemon/src/manager/create.rs`), which bails
    // outright the moment `next_id` has nowhere left to advance, before it is ever
    // handed out as a new window's id. Loading the file is silent about the collision
    // itself, though, unless this reports it — see Minor #3.
    let next_id = match windows.iter().map(|w| w.id).max() {
        Some(max_id) => {
            let candidate = max_id.saturating_add(1);
            if candidate == max_id {
                problems.push(Problem {
                    severity: Severity::Warn,
                    key: "next_id".to_string(),
                    message: format!(
                        "the highest loaded window id {max_id} has no valid successor; \
                         next_id is also {max_id}, so the next window created will \
                         collide with it until the daemon restarts"
                    ),
                });
            }
            saved_next_id.max(candidate)
        }
        None => saved_next_id,
    };

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
fn mark_aside_with(
    path: &Path,
    tag: &str,
    why: &str,
    now: SystemTime,
) -> (StateFile, Vec<Problem>) {
    let aside = aside_path_with(path, tag, now);
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
///
/// `now` is injectable (following `project::detect_roots_with`'s convention) so a test
/// can drive two, three, or more loads through the identical timestamp deterministically,
/// instead of depending on the wall clock not ticking over between two real `load` calls —
/// see `corrupt_file_is_moved_aside`'s history for why that assumption doesn't hold.
fn aside_path_with(path: &Path, tag: &str, now: SystemTime) -> PathBuf {
    let ts = now.duration_since(UNIX_EPOCH).unwrap_or_default().as_secs();
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

/// How long [`spawn_persister`] waits, after a change, for further changes before it
/// writes (decision 9).
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(100);

/// Subscribes to `manager`'s window list and keeps `path` current (decision 9).
///
/// Every publish on [`crate::manager::WindowManager::watch`] is one *debounced* write,
/// not one write each: on a change this waits [`SAVE_DEBOUNCE`], restarting the wait on
/// every further change, so a burst — a rename typed character by character, twenty
/// renames in a tight loop — reaches disk as a single write of the *final* state rather
/// than one write per edit. The wait is skipped entirely, and this task returns at once
/// with no write of its own, the moment `shutdown` is cancelled: a debounce must never
/// hold shutdown up, and [`crate::lifecycle::run`]'s own flush after this task has
/// stopped is what guarantees the last change reaches disk (decision 11) — this loop's
/// job is only to keep the file *reasonably* current while the daemon is up.
///
/// The snapshot itself, [`crate::manager::WindowManager::state_snapshot`], is a clone
/// taken under the manager lock with no I/O under it (decision 9); only the write that
/// follows runs on [`tokio::task::spawn_blocking`]. A write is skipped when its
/// serialized bytes equal the last ones actually written, so a change that round-trips
/// to identical bytes (a rename back to the same name, a status flap that settles where
/// it started) costs no I/O. A failed write is logged at `warn` and left for the next
/// change to retry — this task never retries on its own, since decision 11's final flush
/// is the backstop for any write this loop never gets to.
pub fn spawn_persister(
    manager: Arc<crate::manager::WindowManager>,
    path: PathBuf,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut changes = manager.watch();
        let mut last_written: Option<Vec<u8>> = None;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                changed = changes.changed() => {
                    if changed.is_err() {
                        // The manager itself is gone; nothing left to persist for.
                        return;
                    }
                }
            }

            // Collect further changes for SAVE_DEBOUNCE, restarting the wait on each
            // one. A fresh `sleep` future is constructed every time this inner loop
            // runs, which is what makes the wait restart rather than merely continue a
            // clock that started on the first change.
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    () = tokio::time::sleep(SAVE_DEBOUNCE) => break,
                    changed = changes.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            }

            let state = manager.state_snapshot();
            let bytes = match serde_json::to_vec_pretty(&state) {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::warn!(%error, "failed to serialize state for persistence");
                    continue;
                }
            };
            if last_written.as_deref() == Some(bytes.as_slice()) {
                continue;
            }

            let save_path = path.clone();
            let save_state = state.clone();
            match tokio::task::spawn_blocking(move || save(&save_path, &save_state)).await {
                Ok(Ok(())) => last_written = Some(bytes),
                Ok(Err(error)) => {
                    tracing::warn!(
                        %error,
                        path = %path.display(),
                        "failed to save state file; will retry on the next change"
                    );
                }
                Err(error) => {
                    tracing::warn!(%error, "state save task panicked; will retry on the next change");
                }
            }
        }
    })
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
