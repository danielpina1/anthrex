//! The session driver's threads (`super`'s module doc): the stdin writer, the stdout
//! and stderr line readers, the waiter that reaps the leader, and the dispatcher that
//! parses and delivers every event of one process in order.

use super::{
    CUT_KEEP_BYTES, OUTPUT_GRACE, Process, SANDBOX_UNAVAILABLE, STDERR_LINE_MAX, STDOUT_LINE_MAX,
    signal_locked,
};
use crate::headless::claude_stream::ClaudeStream;
use crate::headless::{FailureKind, SessionEvent, TurnOutcome, codex_stream};
use proto::Runtime;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::time::Instant;

/// What the readers and the waiter tell the dispatcher.
pub(super) enum Msg {
    Line(String),
    Cut(String),
    Stderr(String),
    StdoutDone,
    StderrDone,
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

pub(super) fn write_lines(mut stdin: std::process::ChildStdin, lines: Receiver<String>) {
    for line in lines {
        let written = stdin
            .write_all(line.as_bytes())
            .and_then(|()| stdin.write_all(b"\n"))
            .and_then(|()| stdin.flush());
        if let Err(error) = written {
            tracing::debug!(?error, "a session's stdin closed");
            return;
        }
    }
    // The queue's sender is gone (`close_stdin`): dropping `stdin` sends EOF.
}

/// Reads `\n`-separated lines, keeping at most `max` bytes of each (the rest is read
/// and dropped); calls `line(bytes, cut)` for each non-empty one, a last unterminated
/// line included. One trailing `\r` is dropped.
fn read_lines(reader: impl Read, max: usize, mut line: impl FnMut(&[u8], bool) -> bool) {
    let mut reader = BufReader::with_capacity(64 * 1024, reader);
    let mut current = Vec::new();
    let mut cut = false;
    loop {
        let (consumed, end) = match reader.fill_buf() {
            Ok([]) => break,
            Ok(buffer) => match buffer.iter().position(|&b| b == b'\n') {
                Some(at) => {
                    keep(&mut current, &mut cut, &buffer[..at], max);
                    (at + 1, true)
                }
                None => {
                    keep(&mut current, &mut cut, buffer, max);
                    (buffer.len(), false)
                }
            },
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) => {
                tracing::debug!(?error, "a session pipe failed");
                break;
            }
        };
        reader.consume(consumed);
        if end && !emit(&mut current, &mut cut, &mut line) {
            return;
        }
    }
    emit(&mut current, &mut cut, &mut line);
}

fn keep(current: &mut Vec<u8>, cut: &mut bool, bytes: &[u8], max: usize) {
    let room = max.saturating_sub(current.len());
    if bytes.len() > room {
        *cut = true;
    }
    current.extend_from_slice(&bytes[..bytes.len().min(room)]);
}

fn emit(current: &mut Vec<u8>, cut: &mut bool, line: &mut impl FnMut(&[u8], bool) -> bool) -> bool {
    if current.last() == Some(&b'\r') && !*cut {
        current.pop();
    }
    let go_on = current.iter().all(u8::is_ascii_whitespace) || line(current, *cut);
    current.clear();
    *cut = false;
    go_on
}

pub(super) fn read_stdout(stdout: std::process::ChildStdout, tx: Sender<Msg>) {
    read_lines(stdout, STDOUT_LINE_MAX, |bytes, cut| {
        let msg = if cut {
            Msg::Cut(
                String::from_utf8_lossy(&bytes[..bytes.len().min(CUT_KEEP_BYTES)]).into_owned(),
            )
        } else {
            Msg::Line(String::from_utf8_lossy(bytes).into_owned())
        };
        tx.send(msg).is_ok()
    });
    let _ = tx.send(Msg::StdoutDone);
}

pub(super) fn read_stderr(stderr: std::process::ChildStderr, tx: Sender<Msg>) {
    read_lines(stderr, STDERR_LINE_MAX, |bytes, _cut| {
        tx.send(Msg::Stderr(String::from_utf8_lossy(bytes).into_owned()))
            .is_ok()
    });
    let _ = tx.send(Msg::StderrDone);
}

/// Waits for the leader to exit without reaping it, kills what is left of its group
/// while the group id is still this session's, then reaps it and reports the exit.
pub(super) fn wait_leader(process: &Process, tx: Sender<Msg>) {
    let pid = process.pid as libc::pid_t;
    let observed = loop {
        // SAFETY: an all-zero `siginfo_t` is a valid value for waitid to overwrite.
        let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
        // SAFETY: `pid` is this process's unreaped child; `WNOWAIT` leaves it waitable.
        let result = unsafe {
            libc::waitid(
                libc::P_PID,
                pid as libc::id_t,
                &mut info,
                libc::WEXITED | libc::WNOWAIT,
            )
        };
        if result == 0 {
            break true;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            // Not our child any more (`ECHILD`): something else reaped it, so its pid
            // may already be another process's. Neither signal nor reap it.
            tracing::debug!(?error, pid, "waitid on a session failed");
            break false;
        }
    };
    let mut reaped = crate::lock(&process.reaped);
    let (code, signal) = if observed {
        signal_locked(process.pid, libc::SIGKILL, true);
        let mut status = 0;
        // SAFETY: reaping the exited child observed above.
        let result = loop {
            let result = unsafe { libc::waitpid(pid, &mut status, 0) };
            if result != -1
                || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
            {
                break result;
            }
        };
        if result == pid {
            let status = std::process::ExitStatus::from_raw(status);
            (status.code(), status.signal())
        } else {
            (None, None)
        }
    } else {
        (None, None)
    };
    *reaped = true;
    process.exited.notify_all();
    drop(reaped);
    let _ = tx.send(Msg::Exited { code, signal });
}

/// Delivers a process's events in order: `ProcessStarted`, every stdout and stderr line
/// until the exit (and for at most [`OUTPUT_GRACE`] after it), then the sandbox failure
/// when there was one, then `ProcessExited`.
pub(super) fn dispatch(
    pid: u32,
    runtime: Runtime,
    rx: Receiver<Msg>,
    on_event: impl Fn(u32, SessionEvent),
) {
    on_event(pid, SessionEvent::ProcessStarted { pid });
    let mut claude = ClaudeStream::default();
    let mut saw_init = false;
    let mut sandbox: Option<String> = None;
    let mut open_pipes = 2;
    let mut exit: Option<(Option<i32>, Option<i32>)> = None;
    let mut grace_until: Option<Instant> = None;
    loop {
        let msg = match grace_until {
            None => rx.recv().ok(),
            Some(until) => {
                if open_pipes == 0 {
                    break;
                }
                match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
                    Ok(msg) => Some(msg),
                    Err(RecvTimeoutError::Timeout) => {
                        tracing::debug!(pid, "a session's pipes outlived it; abandoned");
                        break;
                    }
                    Err(RecvTimeoutError::Disconnected) => None,
                }
            }
        };
        let Some(msg) = msg else { break };
        match msg {
            Msg::Line(line) => {
                let events = match runtime {
                    Runtime::Claude => claude.parse_line(&line),
                    _ => codex_stream::parse_line(&line),
                };
                for event in events {
                    saw_init |= matches!(event, SessionEvent::Init { .. });
                    if let SessionEvent::Diagnostic { text } = &event {
                        tracing::warn!(pid, text = %text, "a headless session reported an error");
                    }
                    on_event(pid, event);
                }
            }
            Msg::Cut(prefix) => on_event(pid, crate::headless::unknown(&prefix)),
            Msg::Stderr(line) => {
                if !saw_init && sandbox.is_none() && line.contains(SANDBOX_UNAVAILABLE) {
                    sandbox = Some(line.trim().to_string());
                }
                on_event(pid, SessionEvent::StderrLine { line });
            }
            Msg::StdoutDone | Msg::StderrDone => open_pipes -= 1,
            Msg::Exited { code, signal } => {
                exit = Some((code, signal));
                grace_until = Some(Instant::now() + OUTPUT_GRACE);
            }
        }
        if exit.is_some() && open_pipes == 0 {
            break;
        }
    }
    if let Some(error) = sandbox.filter(|_| !saw_init) {
        on_event(
            pid,
            SessionEvent::TurnEnded {
                outcome: TurnOutcome::Failed {
                    error,
                    kind: FailureKind::SandboxUnavailable,
                },
                usage: None,
                denials: Vec::new(),
            },
        );
    }
    let (code, signal) = exit.unwrap_or((None, None));
    on_event(pid, SessionEvent::ProcessExited { code, signal });
}
