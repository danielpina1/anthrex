//! Hardened execution of short-lived child processes whose stdout the caller wants to
//! capture in full, bounded by a byte cap and a deadline.
//!
//! This is the exact hardening `daemon::project`'s root detection needed (a scrubbed
//! git environment, a non-blocking stdout drain, and a child that is always terminated
//! and reaped rather than left running) generalised so `daemon::git::probe` can reuse
//! it with its own cap and its own timeout, per milestone 4.5 design decision 8. The
//! two callers differ only in what they do with a timeout or an over-cap read: project
//! detection discards the partial output and falls back to the bare `cwd`, while the
//! probe keeps the partial output and marks its result stale. That difference lives in
//! the callers; this module only reports what happened.

use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStdout, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

const POLL_INTERVAL: Duration = Duration::from_millis(10);

/// What happened when [`run`] tried to capture a child's stdout.
#[derive(Debug)]
pub enum Outcome {
    /// The child could not be spawned, its stdout could not be captured or made
    /// non-blocking, its output could not be read, or it exited with a non-zero
    /// status.
    Failed,
    /// The child exited zero and its stdout reached EOF before the cap or the
    /// deadline; `output` is everything it wrote.
    Complete(Vec<u8>),
    /// The deadline elapsed before stdout reached EOF. The child has already been
    /// terminated and reaped; `output` is whatever had been read so far.
    TimedOut(Vec<u8>),
    /// Stdout exceeded the byte cap before reaching EOF. The child has already been
    /// terminated and reaped; `output` is output up to (just under) the cap.
    Truncated(Vec<u8>),
}

enum DrainState {
    Open,
    Eof,
    Deadline,
    OverCap,
}

/// Spawns `command` with a scrubbed git environment (`GIT_DIR`, `GIT_WORK_TREE`,
/// `GIT_COMMON_DIR`, `GIT_INDEX_FILE`, `GIT_PREFIX` removed), null stdin and stderr,
/// piped stdout, and as the leader of its own process group; drains stdout
/// non-blockingly up to `max_output_bytes` or until `timeout` elapses, then reports
/// what happened.
///
/// A child that does not finish in time, or whose output exceeds the cap, is always
/// terminated and reaped before this function returns. Running it as its own process
/// group leader means termination reaches any descendant it spawned too (a pager, an
/// index-lock helper), not just the direct child — a probe that only killed the direct
/// child could leak a grandchild that keeps running, or keeps the stdout pipe open,
/// indefinitely.
pub fn run(command: &mut Command, max_output_bytes: usize, timeout: Duration) -> Outcome {
    command
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .process_group(0);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            tracing::debug!(?error, program = ?command.get_program(), "could not spawn child process");
            return Outcome::Failed;
        }
    };

    let mut status = None;
    let Some(mut stdout) = child.stdout.take() else {
        tracing::debug!(program = ?command.get_program(), "child process had no stdout");
        terminate_unreaped(&mut child, &mut status);
        return Outcome::Failed;
    };
    if let Err(error) = set_nonblocking(&stdout) {
        tracing::debug!(
            ?error,
            program = ?command.get_program(),
            "could not make child process stdout nonblocking"
        );
        terminate_unreaped(&mut child, &mut status);
        return Outcome::Failed;
    }

    let started = Instant::now();
    let mut output = Vec::new();
    let mut eof = false;
    loop {
        if !eof {
            match drain_stdout(&mut stdout, &mut output, max_output_bytes, started, timeout) {
                Ok(DrainState::Open) => {}
                Ok(DrainState::Eof) => eof = true,
                Ok(DrainState::Deadline) => {
                    tracing::debug!(program = ?command.get_program(), ?timeout, "child process timed out");
                    terminate_unreaped(&mut child, &mut status);
                    return Outcome::TimedOut(output);
                }
                Ok(DrainState::OverCap) => {
                    tracing::debug!(
                        program = ?command.get_program(),
                        max_output_bytes,
                        "child process output exceeded the cap"
                    );
                    terminate_unreaped(&mut child, &mut status);
                    return Outcome::Truncated(output);
                }
                Err(error) => {
                    tracing::debug!(?error, program = ?command.get_program(), "could not read child process output");
                    terminate_unreaped(&mut child, &mut status);
                    return Outcome::Failed;
                }
            }
        }

        if status.is_none() {
            match child.try_wait() {
                Ok(found) => status = found,
                Err(error) => {
                    tracing::debug!(?error, program = ?command.get_program(), "could not poll child process status");
                    terminate_unreaped(&mut child, &mut status);
                    return Outcome::Failed;
                }
            }
        }

        if let Some(status) = status {
            if !status.success() {
                tracing::debug!(?status, program = ?command.get_program(), "child process exited non-zero");
                return Outcome::Failed;
            }
            if eof {
                return Outcome::Complete(output);
            }
        }

        if started.elapsed() >= timeout {
            tracing::debug!(program = ?command.get_program(), ?timeout, "child process timed out");
            terminate_unreaped(&mut child, &mut status);
            return Outcome::TimedOut(output);
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        std::thread::sleep(POLL_INTERVAL.min(remaining));
    }
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
    max_output_bytes: usize,
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
                if output.len() + count > max_output_bytes {
                    return Ok(DrainState::OverCap);
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

/// Terminates and reaps `child` if it has not already exited. When `status` already
/// holds an exit status, the child was already reaped by an earlier `try_wait` and
/// there is nothing left to do here — the caller's own logic, not this function, is
/// responsible for anything the child left behind after exiting on its own (see the
/// project-detection test for exactly this: a child that exits immediately but leaves
/// a background grandchild holding the stdout pipe open).
fn terminate_unreaped(child: &mut Child, status: &mut Option<ExitStatus>) {
    if status.is_some() {
        return;
    }
    if let Ok(Some(found)) = child.try_wait() {
        *status = Some(found);
        return;
    }
    let pid = child.id() as libc::pid_t;
    // SAFETY: `run` starts this child as the leader of its own process group
    // (`process_group(0)`, so pid == pgid), so signalling the group by its pid can only
    // reach this child and its own descendants, never a process outside the tree `run`
    // started.
    if unsafe { libc::killpg(pid, libc::SIGKILL) } == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            tracing::debug!(?error, pid, "could not kill child process group");
        }
    }
    match child.wait() {
        Ok(found) => *status = Some(found),
        Err(error) => {
            tracing::debug!(?error, pid, "could not reap child process");
        }
    }
}
