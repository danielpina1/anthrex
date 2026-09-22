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

mod load;
mod persist;

#[cfg(test)]
use load::aside_path_with;
pub use load::{load, load_with};
pub use persist::{SAVE_DEBOUNCE, SAVE_MAX_DELAY, save, spawn_persister};

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
/// windows themselves. This milestone never populates `runs` on its own — a daemon that
/// has never loaded a file with real content in it writes `[]`, and `create` never adds
/// anything here either (decision 8; milestone 8 gives this field a real element type
/// and something that actually writes to it) — but it is opaque JSON, not `()` or a
/// field this crate omits, precisely so that a *loaded* value survives a round trip
/// through this build unchanged: `WindowManager::restore` keeps whatever was here
/// (`Inner.runs`) and `state_snapshot` writes it back verbatim, rather than the module
/// needing to know the real element type milestone 8 eventually gives it. (Whole-branch-
/// review Major 2: this rationale used to be false as written — `state_snapshot` zeroed
/// `runs` and every record's `run` unconditionally, so the first save after loading a
/// file with real content in either erased it. Fixed at the `WindowManager` layer, not
/// here — this module's own `load`/`save` round trip was already faithful; see
/// `manager/restore.rs`.)
///
/// `STATE_VERSION` does not need to change for this: the whole point of keeping both
/// fields opaque is that carrying them through verbatim needs no knowledge of their
/// shape, so nothing about *this* fix depends on the format version at all. A version
/// bump remains milestone 8's call to make, if and when it gives `runs`/`run` an actual
/// schema this module needs to validate against.
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

#[cfg(test)]
#[path = "../state_tests.rs"]
mod tests;
