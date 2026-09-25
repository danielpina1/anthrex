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
//!
//! [`run_captured_head_tail`] (M8a.8 fix round 1) is `run_captured` for an output of
//! any size — a diff of a vendored data file — keeping its first and last bytes and
//! dropping the middle rather than failing over a cap. Both share one body, `capture`,
//! which waits for its pipes with `poll(2)` between drains instead of a fixed sleep,
//! so a large output is read at pipe speed.

mod head_tail;

pub use head_tail::{HeadTail, run_captured_head_tail};

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
    let mut sink = CappedSink {
        output: Vec::new(),
        max: max_output_bytes,
    };
    let (end, stderr, spawn_error) = capture(command, &mut sink, max_stderr_bytes, timeout);
    let output = sink.output;
    let outcome = match end {
        End::Failed => Outcome::Failed,
        End::Complete => Outcome::Complete(output),
        End::TimedOut => Outcome::TimedOut(output),
        End::OverCap => Outcome::Truncated(output),
    };
    Captured {
        outcome,
        stderr,
        spawn_error,
    }
}

/// Where [`capture`] puts stdout. `accept` returns `false` when the bytes would take
/// the sink over its cap, which ends the capture as [`End::OverCap`].
trait StdoutSink {
    fn accept(&mut self, bytes: &[u8]) -> bool;
}

struct CappedSink {
    output: Vec<u8>,
    max: usize,
}

impl StdoutSink for CappedSink {
    fn accept(&mut self, bytes: &[u8]) -> bool {
        if self.output.len() + bytes.len() > self.max {
            return false;
        }
        self.output.extend_from_slice(bytes);
        true
    }
}

/// How [`capture`] ended.
enum End {
    Failed,
    Complete,
    TimedOut,
    OverCap,
}

/// The shared body of [`run_captured`] and [`run_captured_head_tail`].
fn capture(
    command: &mut Command,
    sink: &mut impl StdoutSink,
    max_stderr_bytes: usize,
    timeout: Duration,
) -> (End, String, Option<io::ErrorKind>) {
    scrub_git_env(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            tracing::debug!(?error, program = ?command.get_program(), "could not spawn child process");
            return (End::Failed, String::new(), Some(error.kind()));
        }
    };

    let mut status = None;
    let (Some(mut stdout), Some(mut stderr)) = (child.stdout.take(), child.stderr.take()) else {
        tracing::debug!(program = ?command.get_program(), "child process had no stdout or stderr");
        terminate_unreaped(&mut child, &mut status);
        return (End::Failed, String::new(), None);
    };
    if let Err(error) = set_nonblocking(&stdout).and_then(|()| set_nonblocking(&stderr)) {
        tracing::debug!(
            ?error,
            program = ?command.get_program(),
            "could not make child process output nonblocking"
        );
        terminate_unreaped(&mut child, &mut status);
        return (End::Failed, String::new(), None);
    }

    let started = Instant::now();
    let mut stderr_bytes = Vec::new();
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    let end = loop {
        if !stdout_eof {
            match drain_into(&mut stdout, sink, started, timeout) {
                Ok(DrainState::Open) => {}
                Ok(DrainState::Eof) => stdout_eof = true,
                Ok(DrainState::Deadline) => {
                    tracing::debug!(program = ?command.get_program(), ?timeout, "child process timed out");
                    terminate_unreaped(&mut child, &mut status);
                    break End::TimedOut;
                }
                Ok(DrainState::OverCap) => {
                    tracing::debug!(
                        program = ?command.get_program(),
                        "child process output exceeded the cap"
                    );
                    terminate_unreaped(&mut child, &mut status);
                    break End::OverCap;
                }
                Err(error) => {
                    tracing::debug!(?error, program = ?command.get_program(), "could not read child process stdout");
                    terminate_unreaped(&mut child, &mut status);
                    break End::Failed;
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
                    break End::TimedOut;
                }
                Err(error) => {
                    tracing::debug!(?error, program = ?command.get_program(), "could not read child process stderr");
                    terminate_unreaped(&mut child, &mut status);
                    break End::Failed;
                }
            }
        }

        if status.is_none() {
            match child.try_wait() {
                Ok(found) => status = found,
                Err(error) => {
                    tracing::debug!(?error, program = ?command.get_program(), "could not poll child process status");
                    terminate_unreaped(&mut child, &mut status);
                    break End::Failed;
                }
            }
        }

        if let Some(found) = status
            && stdout_eof
            && stderr_eof
        {
            if found.success() {
                break End::Complete;
            }
            tracing::debug!(status = ?found, program = ?command.get_program(), "child process exited non-zero");
            break End::Failed;
        }

        if started.elapsed() >= timeout {
            tracing::debug!(program = ?command.get_program(), ?timeout, "child process timed out");
            terminate_unreaped(&mut child, &mut status);
            break End::TimedOut;
        }
        let remaining = timeout.saturating_sub(started.elapsed());
        let mut open = Vec::with_capacity(2);
        if !stdout_eof {
            open.push(stdout.as_raw_fd());
        }
        if !stderr_eof {
            open.push(stderr.as_raw_fd());
        }
        wait_readable(&open, POLL_INTERVAL.min(remaining));
    };
    (end, lossy_stderr(stderr_bytes), None)
}

/// Waits up to `wait` for any of `fds` to be readable (or hung up), or simply sleeps
/// when none is open. Waking as soon as a pipe has data, rather than after a fixed
/// sleep, is what lets [`capture`] read a large output at pipe speed: a sleep after
/// every pipe-full caps it at a few MB/s.
pub(crate) fn wait_readable(fds: &[std::os::fd::RawFd], wait: Duration) {
    if fds.is_empty() {
        std::thread::sleep(wait);
        return;
    }
    let mut polled: Vec<libc::pollfd> = fds
        .iter()
        .map(|&fd| libc::pollfd {
            fd,
            events: libc::POLLIN,
            revents: 0,
        })
        .collect();
    let millis = wait.as_millis().clamp(1, 1000) as libc::c_int;
    // SAFETY: `polled` is a live, correctly sized array of `pollfd`s over descriptors
    // the caller owns for the duration of this call; poll only writes `revents`.
    unsafe { libc::poll(polled.as_mut_ptr(), polled.len() as libc::nfds_t, millis) };
}

/// As [`drain_stdout`], into a [`StdoutSink`].
fn drain_into(
    stdout: &mut ChildStdout,
    sink: &mut impl StdoutSink,
    started: Instant,
    timeout: Duration,
) -> io::Result<DrainState> {
    let mut buffer = [0; 65536];
    loop {
        if started.elapsed() >= timeout {
            return Ok(DrainState::Deadline);
        }
        match stdout.read(&mut buffer) {
            Ok(0) => return Ok(DrainState::Eof),
            Ok(count) => {
                if !sink.accept(&buffer[..count]) {
                    return Ok(DrainState::OverCap);
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                return Ok(DrainState::Open);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

fn lossy_stderr(bytes: Vec<u8>) -> String {
    String::from_utf8_lossy(&bytes).into_owned()
}

/// AGENTS.md rule 11's five variables go, and (ruling T14-R3) git reads no
/// `refs/replace` objects, so a replacement a worker writes into the shared repository
/// cannot make one commit read as another to a gate, a merge or a check.
///
/// Rule 11 removes an **inherited** value. One the daemon set on `command` itself, on
/// purpose, is kept: M8a final fix batch F1c (C1) runs every engine git command in a
/// task checkout with `GIT_INDEX_FILE` naming an engine-owned copy of its index
/// ([`crate::worktree::engine_index`]), so no engine write ever goes through an index
/// file the worker can replace with a symbolic link. That is the only such use.
pub(crate) fn scrub_git_env(command: &mut Command) -> &mut Command {
    scrub_git_location_env(command).env("GIT_NO_REPLACE_OBJECTS", "1")
}

/// Rule 11's inherited location variables removed, as in [`scrub_git_env`], without
/// `GIT_NO_REPLACE_OBJECTS`: for a process that runs the project's or an agent's own
/// commands (a check, a proof, `setup`, a headless session), whose git is the user's
/// to configure (final fix batch F4, T14-P1). The engine's own git calls go through
/// [`scrub_git_env`].
pub(crate) fn scrub_git_location_env(command: &mut Command) -> &mut Command {
    for key in config::reserved_env::GIT_LOCATION_VARS {
        let set_here = command
            .get_envs()
            .any(|(name, value)| name == key && value.is_some());
        if !set_here {
            command.env_remove(key);
        }
    }
    command
}

pub(crate) fn set_nonblocking(stream: &impl AsRawFd) -> io::Result<()> {
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
