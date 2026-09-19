//! Project-root detection for daemon windows.

use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const DETECT_TIMEOUT: Duration = Duration::from_secs(5);

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Blocking. Never fails: falls back to the canonical `cwd`, or `cwd` itself.
pub fn detect_root(cwd: &Path) -> PathBuf {
    detect_root_with(OsStr::new("git"), cwd, DETECT_TIMEOUT)
}

/// Blocking, with the git program and timeout injectable for tests.
pub fn detect_root_with(git: &OsStr, cwd: &Path, timeout: Duration) -> PathBuf {
    let mut child = match Command::new(git)
        .arg("-C")
        .arg(cwd)
        .args([
            "rev-parse",
            "--path-format=absolute",
            "--git-common-dir",
            "--show-toplevel",
        ])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            tracing::debug!(?error, ?git, ?cwd, "project detection could not start git");
            return fallback(cwd);
        }
    };

    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if started.elapsed() >= timeout => {
                tracing::debug!(?git, ?cwd, ?timeout, "project detection timed out");
                if let Err(error) = child.kill() {
                    tracing::debug!(?error, ?git, ?cwd, "could not kill timed-out git");
                }
                if let Err(error) = child.wait() {
                    tracing::debug!(?error, ?git, ?cwd, "could not reap timed-out git");
                }
                return fallback(cwd);
            }
            Ok(None) => {
                let remaining = timeout.saturating_sub(started.elapsed());
                std::thread::sleep(POLL_INTERVAL.min(remaining));
            }
            Err(error) => {
                tracing::debug!(?error, ?git, ?cwd, "could not poll git for project root");
                if let Err(kill_error) = child.kill() {
                    tracing::debug!(?kill_error, ?git, ?cwd, "could not kill unpollable git");
                }
                if let Err(wait_error) = child.wait() {
                    tracing::debug!(?wait_error, ?git, ?cwd, "could not reap unpollable git");
                }
                return fallback(cwd);
            }
        }
    };

    if !status.success() {
        tracing::debug!(?status, ?git, ?cwd, "git could not detect a project root");
        return fallback(cwd);
    }

    let mut output = Vec::new();
    let Some(mut stdout) = child.stdout.take() else {
        tracing::debug!(?git, ?cwd, "git project detection had no stdout");
        return fallback(cwd);
    };
    if let Err(error) = stdout.read_to_end(&mut output) {
        tracing::debug!(
            ?error,
            ?git,
            ?cwd,
            "could not read git project detection output"
        );
        return fallback(cwd);
    }

    let Some(root) = parse_root(&output) else {
        tracing::debug!(
            ?git,
            ?cwd,
            "git project detection returned malformed output"
        );
        return fallback(cwd);
    };
    canonicalize_or(root)
}

/// Runs [`detect_root`] on a blocking thread.
pub async fn resolve_root(cwd: PathBuf) -> PathBuf {
    resolve_root_with(OsString::from("git"), cwd, DETECT_TIMEOUT).await
}

/// Injectable async twin of [`detect_root_with`].
#[doc(hidden)]
pub async fn resolve_root_with(git: OsString, cwd: PathBuf, timeout: Duration) -> PathBuf {
    let unchanged = cwd.clone();
    match tokio::task::spawn_blocking(move || detect_root_with(&git, &cwd, timeout)).await {
        Ok(root) => root,
        Err(error) => {
            tracing::debug!(?error, ?unchanged, "project detection task failed");
            unchanged
        }
    }
}

fn parse_root(output: &[u8]) -> Option<PathBuf> {
    let output = output.strip_suffix(b"\n").unwrap_or(output);
    let mut lines = output.split(|byte| *byte == b'\n');
    let common_dir = Path::new(OsStr::from_bytes(lines.next()?));
    let toplevel = Path::new(OsStr::from_bytes(lines.next()?));
    if common_dir.as_os_str().is_empty()
        || toplevel.as_os_str().is_empty()
        || lines.next().is_some()
    {
        return None;
    }

    if common_dir.file_name() == Some(OsStr::new(".git")) {
        common_dir.parent().map(Path::to_path_buf)
    } else {
        Some(toplevel.to_path_buf())
    }
}

fn fallback(cwd: &Path) -> PathBuf {
    canonicalize_or(cwd.to_path_buf())
}

fn canonicalize_or(path: PathBuf) -> PathBuf {
    path.canonicalize().unwrap_or(path)
}
