//! Project-root detection for daemon windows.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::subprocess::{self, Outcome};

pub const DETECT_TIMEOUT: Duration = Duration::from_secs(5);

const MAX_OUTPUT_BYTES: usize = 64 * 1024;

/// The two roots a window's directory can resolve to.
///
/// `project` groups the tree and is shared by every linked worktree of a repository.
/// `worktree` is the checkout the window's directory actually lives in, `None` when it
/// is not inside a git working tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DetectedRoots {
    pub project: PathBuf,
    pub worktree: Option<PathBuf>,
}

/// Blocking. Never fails: falls back to the canonical `cwd`, or `cwd` itself.
pub fn detect_roots(cwd: &Path) -> DetectedRoots {
    detect_roots_with(OsStr::new("git"), cwd, DETECT_TIMEOUT)
}

/// Blocking, with the git program and timeout injectable for tests.
pub fn detect_roots_with(git: &OsStr, cwd: &Path, timeout: Duration) -> DetectedRoots {
    let mut command = Command::new(git);
    command.arg("-C").arg(cwd).args([
        "rev-parse",
        "--path-format=absolute",
        "--git-common-dir",
        "--show-toplevel",
    ]);

    let output = match subprocess::run(&mut command, MAX_OUTPUT_BYTES, timeout) {
        Outcome::Complete(output) => output,
        // A timeout or an over-cap read gets no special treatment here, unlike in
        // `git::probe`: project-root detection has no "stale" concept, so there is
        // nothing useful to do with a partial read but fall back, same as any other
        // failure to detect.
        Outcome::Failed | Outcome::TimedOut(_) | Outcome::Truncated(_) => {
            return fallback_roots(cwd);
        }
    };

    let Some(roots) = parse_roots(&output) else {
        tracing::debug!(
            ?git,
            ?cwd,
            "git project detection returned malformed output"
        );
        return fallback_roots(cwd);
    };
    DetectedRoots {
        project: canonicalize_or(roots.project),
        worktree: roots.worktree.map(canonicalize_or),
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
    })
}

fn fallback(cwd: &Path) -> PathBuf {
    canonicalize_or(cwd.to_path_buf())
}

fn fallback_roots(cwd: &Path) -> DetectedRoots {
    DetectedRoots {
        project: fallback(cwd),
        worktree: None,
    }
}

fn canonicalize_or(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}
