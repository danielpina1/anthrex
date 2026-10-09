//! A nonblocking line reader under a deadline, and the spawn every probe shares.
//!
//! The pipe is set `O_NONBLOCK` and polled every 5 ms exactly as
//! `lifecycle::codex_version::probe` does, so a CLI that never answers (or a descendant
//! that keeps the pipe open) cannot hold the probe past its deadline. That read loop is
//! deliberately duplicated rather than shared (preflight F30: `codex_version.rs` changes
//! only by the `ProbeChild` move); a follow-up makes it use this reader.

use super::{LINE_MAX_BYTES, ProbeError, REAP_RESERVE, REPLY_MAX_BYTES};
use crate::probe_child::ProbeChild;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::process::{ChildStdout, Command};
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

const POLL: Duration = Duration::from_millis(5);

pub struct LineReader {
    stdout: ChildStdout,
    pending: Vec<u8>,
    total: usize,
    eof: bool,
    /// Every line [`LineReader::next`] returned (decision 26).
    lines: Vec<String>,
}

impl LineReader {
    pub fn new(stdout: ChildStdout) -> io::Result<LineReader> {
        let fd = stdout.as_raw_fd();
        // SAFETY: stdout owns a live pipe fd; fcntl neither transfers nor closes it.
        let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
        if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
        {
            return Err(io::Error::last_os_error());
        }
        Ok(LineReader {
            stdout,
            pending: Vec::new(),
            total: 0,
            eof: false,
            lines: Vec::new(),
        })
    }

    /// The next complete line (without its `\n`), `Ok(None)` at EOF.
    pub fn next(
        &mut self,
        until: Instant,
        cancel: &CancellationToken,
    ) -> Result<Option<String>, ProbeError> {
        let mut buffer = [0; 8192];
        loop {
            if let Some(at) = self.pending.iter().position(|&b| b == b'\n') {
                let rest = self.pending.split_off(at + 1);
                let line = std::mem::replace(&mut self.pending, rest);
                return Ok(Some(self.keep(&line[..at])));
            }
            if self.pending.len() > LINE_MAX_BYTES {
                return Err(ProbeError::Failed(format!(
                    "a reply line was too long (over {LINE_MAX_BYTES} bytes)"
                )));
            }
            if self.eof {
                if self.pending.is_empty() {
                    return Ok(None);
                }
                let line = std::mem::take(&mut self.pending);
                return Ok(Some(self.keep(&line)));
            }
            if cancel.is_cancelled() {
                return Err(ProbeError::Cancelled);
            }
            if Instant::now() >= until {
                return Err(ProbeError::Timeout);
            }
            match self.stdout.read(&mut buffer) {
                Ok(0) => self.eof = true,
                Ok(count) => {
                    self.total += count;
                    if self.total > REPLY_MAX_BYTES {
                        return Err(ProbeError::Failed(format!(
                            "the reply was too long (over {REPLY_MAX_BYTES} bytes)"
                        )));
                    }
                    self.pending.extend_from_slice(&buffer[..count]);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => std::thread::sleep(POLL),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(ProbeError::Failed(format!("could not read the reply: {e}"))),
            }
        }
    }

    fn keep(&mut self, bytes: &[u8]) -> String {
        let line = String::from_utf8_lossy(bytes).into_owned();
        self.lines.push(line.clone());
        line
    }

    /// Every line returned so far.
    pub fn into_lines(self) -> Vec<String> {
        self.lines
    }
}

/// The read deadline: the probe's deadline less what `ProbeChild` needs to kill and reap.
pub(crate) fn read_deadline(deadline: Instant) -> Instant {
    deadline.checked_sub(REAP_RESERVE).unwrap_or(deadline)
}

/// Spawns `command` (already given `process_group(0)` and its pipes) owned by a
/// [`ProbeChild`]. A program that is not there (or is a directory) is `Missing`.
pub(crate) fn spawn(
    command: &mut Command,
    program: &str,
    deadline: Instant,
    cancel: &CancellationToken,
) -> Result<ProbeChild, ProbeError> {
    if cancel.is_cancelled() {
        return Err(ProbeError::Cancelled);
    }
    if Instant::now() >= read_deadline(deadline) {
        return Err(ProbeError::Timeout);
    }
    match command.spawn() {
        Ok(child) => Ok(ProbeChild {
            child,
            deadline,
            completed: false,
            reaped: false,
        }),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Err(ProbeError::Missing),
        Err(e)
            if e.kind() == io::ErrorKind::PermissionDenied
                && std::path::Path::new(program).is_dir() =>
        {
            Err(ProbeError::Missing)
        }
        Err(e) => Err(ProbeError::Failed(format!(
            "could not start {program}: {e}"
        ))),
    }
}

/// Waits until `until` for the child to exit on its own (its stdin closed); when it
/// does, it is reaped here and `ProbeChild` has nothing left to do. Otherwise
/// `ProbeChild::drop` kills and reaps it, as for any probe it owns.
pub(crate) fn wait_for_exit(owned: &mut ProbeChild, until: Instant) {
    while Instant::now() < until {
        match owned.child.try_wait() {
            Ok(Some(_)) => {
                owned.reaped = true;
                owned.completed = true;
                return;
            }
            Ok(None) => std::thread::sleep(POLL),
            Err(_) => return,
        }
    }
}
