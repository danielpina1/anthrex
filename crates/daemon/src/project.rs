//! Project-root detection for daemon windows.

use std::ffi::{OsStr, OsString};
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

pub const DETECT_TIMEOUT: Duration = Duration::from_secs(5);

const POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_OUTPUT_BYTES: usize = 64 * 1024;

enum DrainState {
    Open,
    Eof,
    Deadline,
}

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

    let mut status = None;
    let Some(mut stdout) = child.stdout.take() else {
        tracing::debug!(?git, ?cwd, "git project detection had no stdout");
        terminate_unreaped(&mut child, &mut status, git, cwd);
        return fallback(cwd);
    };
    if let Err(error) = set_nonblocking(&stdout) {
        tracing::debug!(
            ?error,
            ?git,
            ?cwd,
            "could not make git project detection output nonblocking"
        );
        terminate_unreaped(&mut child, &mut status, git, cwd);
        return fallback(cwd);
    }

    let started = Instant::now();
    let mut output = Vec::new();
    let mut eof = false;
    loop {
        if !eof {
            match drain_stdout(&mut stdout, &mut output, started, timeout) {
                Ok(DrainState::Open) => {}
                Ok(DrainState::Eof) => eof = true,
                Ok(DrainState::Deadline) => {
                    tracing::debug!(?git, ?cwd, ?timeout, "project detection timed out");
                    terminate_unreaped(&mut child, &mut status, git, cwd);
                    return fallback(cwd);
                }
                Err(error) => {
                    tracing::debug!(
                        ?error,
                        ?git,
                        ?cwd,
                        "could not read git project detection output"
                    );
                    terminate_unreaped(&mut child, &mut status, git, cwd);
                    return fallback(cwd);
                }
            }
        }

        if status.is_none() {
            match child.try_wait() {
                Ok(found) => status = found,
                Err(error) => {
                    tracing::debug!(?error, ?git, ?cwd, "could not poll git for project root");
                    terminate_unreaped(&mut child, &mut status, git, cwd);
                    return fallback(cwd);
                }
            }
        }

        if let Some(status) = status {
            if !status.success() {
                tracing::debug!(?status, ?git, ?cwd, "git could not detect a project root");
                return fallback(cwd);
            }
            if eof {
                break;
            }
        }

        if started.elapsed() >= timeout {
            tracing::debug!(?git, ?cwd, ?timeout, "project detection timed out");
            terminate_unreaped(&mut child, &mut status, git, cwd);
            return fallback(cwd);
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        std::thread::sleep(POLL_INTERVAL.min(remaining));
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

fn set_nonblocking(stdout: &ChildStdout) -> io::Result<()> {
    let fd = stdout.as_raw_fd();
    // SAFETY: stdout owns this live pipe descriptor. fcntl changes only its status flags
    // and neither transfers nor closes the descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn drain_stdout(
    stdout: &mut ChildStdout,
    output: &mut Vec<u8>,
    started: Instant,
    timeout: Duration,
) -> io::Result<DrainState> {
    let mut buffer = [0; 4096];
    loop {
        if started.elapsed() >= timeout {
            return Ok(DrainState::Deadline);
        }
        match stdout.read(&mut buffer) {
            Ok(0) => return Ok(DrainState::Eof),
            Ok(count) => {
                if output.len() + count > MAX_OUTPUT_BYTES {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "git project detection output exceeded 64 KiB",
                    ));
                }
                output.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Ok(DrainState::Open);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

fn terminate_unreaped(child: &mut Child, status: &mut Option<ExitStatus>, git: &OsStr, cwd: &Path) {
    if status.is_some() {
        return;
    }
    if let Ok(Some(found)) = child.try_wait() {
        *status = Some(found);
        return;
    }
    if let Err(error) = child.kill() {
        tracing::debug!(?error, ?git, ?cwd, "could not kill project detection git");
    }
    match child.wait() {
        Ok(found) => *status = Some(found),
        Err(error) => {
            tracing::debug!(?error, ?git, ?cwd, "could not reap project detection git");
        }
    }
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
