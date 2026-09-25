//! Project-root detection for daemon windows.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::subprocess::{self, Outcome};

pub const DETECT_TIMEOUT: Duration = Duration::from_secs(5);

const MAX_OUTPUT_BYTES: usize = 64 * 1024;
/// Git's own two-line reply to a failed `rev-parse` here is short; this is only for the
/// `tracing::debug!` that explains *why* detection fell back, not for anything shown to
/// a user, so a small cap is plenty.
const MAX_STDERR_BYTES: usize = 4 * 1024;

/// The two roots a window's directory can resolve to.
///
/// `project` groups the tree and is shared by every linked worktree of a repository.
/// `worktree` is the checkout the window's directory actually lives in, `None` when it
/// is not inside a git working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRoots {
    pub project: PathBuf,
    pub worktree: Option<PathBuf>,
    /// Whether `worktree: None` means something was actually tried and came back
    /// negative (`false`) or means detection could not get an answer at all (`true`):
    /// git could not be started, the command did not finish within [`DETECT_TIMEOUT`],
    /// its output exceeded the cap, or what it printed did not parse. Meaningless when
    /// `worktree` is `Some`.
    ///
    /// The whole-branch review's finding from fix wave C item 5: `worktree::create`
    /// refuses a worktree branch request with `WorktreeError::NotARepo` whenever
    /// `worktree` is `None`, worded as a flat claim that `dir` is not a git repository.
    /// That claim is true for the ordinary case — `run_captured`'s child ran to
    /// completion and `git rev-parse --show-toplevel` exited non-zero, which for this
    /// exact invocation reliably means "not inside a working tree" — but false for every
    /// case this field calls out, where nothing ever gave a negative answer and the
    /// honest statement is "could not tell". `resolve_roots`'s one production caller
    /// (`server::requests::create`) reads this field and answers with a message that
    /// says so and suggests retrying, before a repository the user really is standing in
    /// is ever called not one.
    pub detection_failed: bool,
}

/// Blocking. Never fails: falls back to the canonical `cwd`, or `cwd` itself.
pub fn detect_roots(cwd: &Path) -> DetectedRoots {
    detect_roots_with(OsStr::new("git"), cwd, DETECT_TIMEOUT)
}

/// Blocking, with the git program and timeout injectable for tests.
pub fn detect_roots_with(git: &OsStr, cwd: &Path, timeout: Duration) -> DetectedRoots {
    let mut command = Command::new(git);
    command.arg("-C").arg(cwd).args([
        "--no-optional-locks",
        "-c",
        "core.fsmonitor=false",
        "rev-parse",
        "--path-format=absolute",
        "--git-common-dir",
        "--show-toplevel",
    ]);

    // `run_captured`, not `run`: the plain `Outcome` this command produces cannot tell
    // "git could not even be started" apart from "git ran and exited non-zero", and
    // those two mean very different things here (`detection_failed`'s doc comment).
    // `run_captured`'s `spawn_error` is what makes that distinction; its `stderr` is
    // read only for the debug log below, never shown to a user.
    let captured =
        subprocess::run_captured(&mut command, MAX_OUTPUT_BYTES, MAX_STDERR_BYTES, timeout);
    if let Some(kind) = captured.spawn_error {
        tracing::debug!(
            ?kind,
            ?git,
            ?cwd,
            "could not start git for project detection"
        );
        return fallback_roots(cwd, true);
    }
    let output = match captured.outcome {
        Outcome::Complete(output) => output,
        // The child ran to completion and exited non-zero. For this exact invocation —
        // `rev-parse --show-toplevel` with no ref or object argument that could fail for
        // some other reason — that is git's own, reliable negative answer: `cwd` is not
        // inside a working tree. Not a transient failure, so `detection_failed` stays
        // false.
        Outcome::Failed => {
            tracing::debug!(
                ?cwd,
                stderr = %captured.stderr,
                "git project detection found no repository"
            );
            return fallback_roots(cwd, false);
        }
        // A timeout or an over-cap read gets no special treatment beyond that, unlike in
        // `git::probe`: project-root detection has no "stale" concept, so there is
        // nothing useful to do with a partial read but fall back — but unlike a clean
        // non-zero exit, neither of these is git answering the question at all.
        Outcome::TimedOut(_) | Outcome::Truncated(_) => {
            tracing::debug!(
                ?cwd,
                ?timeout,
                "git project detection did not finish in time"
            );
            return fallback_roots(cwd, true);
        }
    };

    let Some(roots) = parse_roots(&output) else {
        tracing::debug!(
            ?git,
            ?cwd,
            "git project detection returned malformed output"
        );
        return fallback_roots(cwd, true);
    };
    DetectedRoots {
        project: canonicalize_or(roots.project),
        worktree: roots.worktree.map(canonicalize_or),
        detection_failed: false,
    }
}

/// Runs [`detect_roots`] on a blocking thread.
pub async fn resolve_roots(cwd: PathBuf) -> DetectedRoots {
    resolve_roots_with(OsString::from("git"), cwd, DETECT_TIMEOUT).await
}

/// Injectable async twin of [`detect_roots_with`].
#[doc(hidden)]
pub async fn resolve_roots_with(git: OsString, cwd: PathBuf, timeout: Duration) -> DetectedRoots {
    let unchanged = cwd.clone();
    match tokio::task::spawn_blocking(move || detect_roots_with(&git, &cwd, timeout)).await {
        Ok(roots) => roots,
        Err(error) => {
            tracing::debug!(?error, ?unchanged, "project detection task failed");
            DetectedRoots {
                project: unchanged,
                worktree: None,
                // The blocking task panicked or was cancelled — nothing ever ran
                // `rev-parse`, so this is exactly the "could not tell" case, not git
                // answering "no".
                detection_failed: true,
            }
        }
    }
}

/// Reads both lines of `git rev-parse --path-format=absolute --git-common-dir
/// --show-toplevel`: line 1 the common dir, line 2 the toplevel. A missing or empty
/// second line means `worktree: None`.
fn parse_roots(output: &[u8]) -> Option<DetectedRoots> {
    let output = output.strip_suffix(b"\n").unwrap_or(output);
    let mut lines = output.split(|byte| *byte == b'\n');
    let common_dir = Path::new(OsStr::from_bytes(lines.next()?));
    if common_dir.as_os_str().is_empty() {
        return None;
    }
    let toplevel = lines
        .next()
        .map(OsStr::from_bytes)
        .map(Path::new)
        .filter(|path| !path.as_os_str().is_empty());
    if lines.next().is_some() {
        return None;
    }

    let project = if common_dir.file_name() == Some(OsStr::new(".git")) {
        common_dir.parent()?.to_path_buf()
    } else {
        toplevel?.to_path_buf()
    };

    Some(DetectedRoots {
        project,
        worktree: toplevel.map(Path::to_path_buf),
        // Discarded by every caller of `parse_roots`, which reads only `.project` and
        // `.worktree` off this intermediate value and builds its own final
        // `DetectedRoots`; `false` only because the field must hold something.
        detection_failed: false,
    })
}

fn fallback(cwd: &Path) -> PathBuf {
    canonicalize_or(cwd.to_path_buf())
}

fn fallback_roots(cwd: &Path, detection_failed: bool) -> DetectedRoots {
    DetectedRoots {
        project: fallback(cwd),
        worktree: None,
        detection_failed,
    }
}

fn canonicalize_or(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}
