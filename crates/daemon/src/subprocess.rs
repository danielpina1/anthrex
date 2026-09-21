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
//!
//! [`run_captured`] is the same hardening plus the child's stderr, for a caller that
//! must tell the user *why* a command failed — milestone 5's worktree creation and
//! removal, whose only useful diagnostic is on git's stderr. `run` itself is untouched:
//! it keeps nulling stderr and reporting only `Outcome`.

use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
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

/// Like [`DrainState`], but for a stream that is capped without ever being treated as a
/// failure: bytes past the cap are read and discarded rather than stopping the drain, so
/// a chatty stream can neither block the child nor grow the buffer unboundedly.
enum CappedDrainState {
    Open,
    Eof,
    Deadline,
}

/// The stdout [`Outcome`] plus the child's stderr, for callers that must tell the user
/// why a command failed.
#[derive(Debug)]
pub struct Captured {
    pub outcome: Outcome,
    /// Lossy UTF-8, bounded by `max_stderr_bytes`. Empty when the child wrote none.
    pub stderr: String,
    /// `Some` only when the child never started. `ErrorKind::NotFound` means the
    /// program is not installed or not on PATH, which `Outcome::Failed` alone cannot
    /// tell apart from a non-zero exit.
    pub spawn_error: Option<io::ErrorKind>,
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
    scrub_git_env(command)
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

/// As [`run`], but stderr is piped and drained under the same deadline, so a chatty
/// stderr can neither block the child nor outlive the timeout. Stderr is bounded by its
/// own `max_stderr_bytes` cap, but exceeding that cap never changes the [`Outcome`]:
/// bytes past the cap are read and discarded so the pipe never backs up, and the cap
/// only bounds how much of that text `Captured::stderr` keeps.
///
/// The final `Outcome` is decided only once both streams have reached EOF (or the
/// deadline strikes, or stdout exceeds its own cap) — never the moment a non-zero exit
/// status is observed, unlike `run`. Deciding early would risk returning
/// `Outcome::Failed` with `stderr` still short of what the child actually wrote: the
/// exit status is often visible to `try_wait` before every byte already sitting in the
/// pipe has been drained.
pub fn run_captured(
    command: &mut Command,
    max_output_bytes: usize,
    max_stderr_bytes: usize,
    timeout: Duration,
) -> Captured {
    scrub_git_env(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            tracing::debug!(?error, program = ?command.get_program(), "could not spawn child process");
            return Captured {
                outcome: Outcome::Failed,
                stderr: String::new(),
                spawn_error: Some(error.kind()),
            };
        }
    };

    let mut status = None;
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        tracing::debug!(program = ?command.get_program(), "child process had no stdout or stderr");
        terminate_unreaped(&mut child, &mut status);
        return Captured {
            outcome: Outcome::Failed,
            stderr: String::new(),
            spawn_error: None,
        };
    };
    if let Err(error) = set_nonblocking(&stdout) {
        tracing::debug!(
            ?error,
            program = ?command.get_program(),
            "could not make child process stdout nonblocking"
        );
        terminate_unreaped(&mut child, &mut status);
        return Captured {
            outcome: Outcome::Failed,
            stderr: String::new(),
            spawn_error: None,
        };
    }
    if let Err(error) = set_nonblocking(&stderr) {
        tracing::debug!(
            ?error,
            program = ?command.get_program(),
            "could not make child process stderr nonblocking"
        );
        terminate_unreaped(&mut child, &mut status);
        return Captured {
            outcome: Outcome::Failed,
            stderr: String::new(),
            spawn_error: None,
        };
    }

    let started = Instant::now();
    let mut output = Vec::new();
    let mut stderr_bytes = Vec::new();
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    loop {
        if !stdout_eof {
            match drain_stdout(&mut stdout, &mut output, max_output_bytes, started, timeout) {
                Ok(DrainState::Open) => {}
                Ok(DrainState::Eof) => stdout_eof = true,
                Ok(DrainState::Deadline) => {
                    tracing::debug!(program = ?command.get_program(), ?timeout, "child process timed out");
                    terminate_unreaped(&mut child, &mut status);
                    return Captured {
                        outcome: Outcome::TimedOut(output),
                        stderr: lossy_stderr(stderr_bytes),
                        spawn_error: None,
                    };
                }
                Ok(DrainState::OverCap) => {
                    tracing::debug!(
                        program = ?command.get_program(),
                        max_output_bytes,
                        "child process output exceeded the cap"
                    );
                    terminate_unreaped(&mut child, &mut status);
                    return Captured {
                        outcome: Outcome::Truncated(output),
                        stderr: lossy_stderr(stderr_bytes),
                        spawn_error: None,
                    };
                }
                Err(error) => {
                    tracing::debug!(?error, program = ?command.get_program(), "could not read child process stdout");
                    terminate_unreaped(&mut child, &mut status);
                    return Captured {
                        outcome: Outcome::Failed,
                        stderr: lossy_stderr(stderr_bytes),
                        spawn_error: None,
                    };
                }
            }
        }

        if !stderr_eof {
            match drain_stderr_capped(
                &mut stderr,
                &mut stderr_bytes,
                max_stderr_bytes,
                started,
                timeout,
            ) {
                Ok(CappedDrainState::Open) => {}
                Ok(CappedDrainState::Eof) => stderr_eof = true,
                Ok(CappedDrainState::Deadline) => {
                    tracing::debug!(program = ?command.get_program(), ?timeout, "child process timed out");
                    terminate_unreaped(&mut child, &mut status);
                    return Captured {
                        outcome: Outcome::TimedOut(output),
                        stderr: lossy_stderr(stderr_bytes),
                        spawn_error: None,
                    };
                }
                Err(error) => {
                    tracing::debug!(?error, program = ?command.get_program(), "could not read child process stderr");
                    terminate_unreaped(&mut child, &mut status);
                    return Captured {
                        outcome: Outcome::Failed,
                        stderr: lossy_stderr(stderr_bytes),
                        spawn_error: None,
                    };
                }
            }
        }

        if status.is_none() {
            match child.try_wait() {
                Ok(found) => status = found,
                Err(error) => {
                    tracing::debug!(?error, program = ?command.get_program(), "could not poll child process status");
                    terminate_unreaped(&mut child, &mut status);
                    return Captured {
                        outcome: Outcome::Failed,
                        stderr: lossy_stderr(stderr_bytes),
                        spawn_error: None,
                    };
                }
            }
        }

        if let Some(found) = status
            && stdout_eof
            && stderr_eof
        {
            let outcome = if found.success() {
                Outcome::Complete(output)
            } else {
                tracing::debug!(status = ?found, program = ?command.get_program(), "child process exited non-zero");
                Outcome::Failed
            };
            return Captured {
                outcome,
                stderr: lossy_stderr(stderr_bytes),
                spawn_error: None,
            };
        }

        if started.elapsed() >= timeout {
            tracing::debug!(program = ?command.get_program(), ?timeout, "child process timed out");
            terminate_unreaped(&mut child, &mut status);
            return Captured {
                outcome: Outcome::TimedOut(output),
                stderr: lossy_stderr(stderr_bytes),
                spawn_error: None,
            };
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        std::thread::sleep(POLL_INTERVAL.min(remaining));
    }
}

fn lossy_stderr(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).into_owned()
}

fn scrub_git_env(command: &mut Command) -> &mut Command {
    command
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_PREFIX")
}

fn set_nonblocking(stream: &impl AsRawFd) -> io::Result<()> {
    let fd = stream.as_raw_fd();
    // SAFETY: the caller owns this live pipe descriptor for as long as this call runs.
    // fcntl changes only its status flags and neither transfers nor closes the
    // descriptor.
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

/// As [`drain_stdout`], but bytes past `max_stderr_bytes` are read and thrown away
/// instead of stopping the drain: unlike stdout, a stderr over its cap is never a
/// reason to fail or truncate the whole command, only a reason to stop remembering more
/// of it.
fn drain_stderr_capped(
    stderr: &mut ChildStderr,
    output: &mut Vec<u8>,
    max_stderr_bytes: usize,
    started: Instant,
    timeout: Duration,
) -> io::Result<CappedDrainState> {
    let mut buffer = [0; 4096];
    loop {
        if started.elapsed() >= timeout {
            return Ok(CappedDrainState::Deadline);
        }
        match stderr.read(&mut buffer) {
            Ok(0) => return Ok(CappedDrainState::Eof),
            Ok(count) => {
                if output.len() < max_stderr_bytes {
                    let keep = (max_stderr_bytes - output.len()).min(count);
                    output.extend_from_slice(&buffer[..keep]);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Ok(CappedDrainState::Open);
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
