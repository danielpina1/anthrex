//! The engine's own shell commands — `setup`, `check` and the proof runs (decisions 26,
//! 33 and 34). Blocking I/O: call only from `spawn_blocking` or a dedicated thread
//! (AGENTS.md rule 2).
//!
//! Every command runs as `/bin/sh -c "{ <command>\n} 2>&1"` in its own process group,
//! with stdin on `/dev/null`, and with both stdout and stderr on **one** pipe, so the
//! shell's own complaints (a command not found, a syntax error) land in order with the
//! command's output. The environment is decision 26's: every inherited `CLAUDE_CODE_*`
//! variable, `CLAUDECODE` and `ANTHREX_WINDOW_ID` are removed, as are AGENTS.md rule
//! 11's five git variables, and the profile's `env` is set on top.
//!
//! This is its own loop rather than `crate::subprocess::capture`, which it borrows its
//! `poll(2)` wait and non-blocking reads from, for three reasons that `capture` cannot
//! serve without changing what its git callers get: it needs the exit **code** (a
//! non-zero exit is `Outcome::Failed` there), it must kill the **whole group** whenever
//! it finishes, not only when the leader is still running (a check's background process
//! must not outlive it, and a timeout must kill the group even after the shell itself
//! has exited), and it reads one merged stream line by line into a bounded tail instead
//! of a byte buffer. To kill the group safely after the leader exits, the leader is
//! observed with `waitid(WNOWAIT)` and reaped only after the kill: while it is an
//! unreaped zombie its pid, which is the group id, cannot be reused.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use regex::Regex;

use crate::subprocess::{scrub_git_env, set_nonblocking, wait_readable};

/// Lines of output a [`ShellOutcome`] keeps (decision 34).
pub const CHECK_TAIL_LINES: usize = 200;
/// Lines of the tail a bounce message carries (decision 34).
pub const CHECK_SUMMARY_LINES: usize = 40;
/// Characters each kept line is cut to (decision 34).
pub const LINE_MAX_CHARS: usize = 300;
/// How long output is still read once the command's shell has exited, or once its group
/// has been killed, before the pipe is abandoned: a background process that keeps the
/// output open (or one that left the group with `setsid`) cannot hold the command past
/// it. So a command that times out returns within `timeout + OUTPUT_GRACE` (plus the
/// kill and reap), and any command within `timeout + 2 × OUTPUT_GRACE`.
pub const OUTPUT_GRACE: Duration = Duration::from_secs(1);

/// Bytes of one line held while it is read. `LINE_MAX_CHARS` characters are at most 4
/// bytes each, so this always holds the characters the tail keeps.
const TAIL_LINE_BYTES: usize = LINE_MAX_CHARS * 4;
/// Bytes of one line a `test_passed` pattern is matched against: a test runner's result
/// line is short, but the pattern must not be defeated by the tail's 300-character cut.
const MATCH_LINE_BYTES: usize = 64 * 1024;
/// Chunks of up to 64 KiB [`drain`] reads before it returns to the caller's checks.
const READS_PER_DRAIN: usize = 16;
/// The longest wait between checks on the shell, when its output pipe is quiet.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

/// What an engine command did (`OpResult::Check`'s fields).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShellOutcome {
    /// Exited 0 within the timeout.
    pub ok: bool,
    /// The shell's exit code; `None` on a timeout, a signal, or a command that never
    /// started.
    pub code: Option<i32>,
    pub timed_out: bool,
    /// The last [`CHECK_TAIL_LINES`] lines of stdout and stderr merged, each cut to
    /// [`LINE_MAX_CHARS`] characters, invalid UTF-8 replaced, joined by `\n` with no
    /// trailing newline.
    pub tail: String,
    /// Whole seconds from spawn to return.
    pub secs: u64,
}

/// Runs `command` in `dir` under decision 34's rules: its own process group, stdin
/// `/dev/null`, stderr merged into stdout, decision 26's environment plus `env`, and
/// `SIGKILL` for the whole group on timeout or once it has finished.
pub fn run_shell(
    dir: &Path,
    command: &str,
    env: &[(String, String)],
    timeout: Duration,
) -> ShellOutcome {
    run_matching(dir, command, env, timeout, None).0
}

/// The last [`CHECK_SUMMARY_LINES`] lines of `tail`, what a bounce carries (decision 34;
/// the decider's summary is M8b).
pub fn summary(tail: &str) -> String {
    let lines: Vec<&str> = tail.split('\n').collect();
    let start = lines.len().saturating_sub(CHECK_SUMMARY_LINES);
    lines[start..].join("\n")
}

/// Removes decision 26's agent variables and AGENTS.md rule 11's git variables from
/// what `command` inherits, then sets `env`.
fn engine_env(command: &mut Command, env: &[(String, String)]) {
    scrub_git_env(command);
    for (key, _) in std::env::vars_os() {
        if key.as_bytes().starts_with(b"CLAUDE_CODE_") {
            command.env_remove(&key);
        }
    }
    command
        .env_remove("CLAUDECODE")
        .env_remove("ANTHREX_WINDOW_ID");
    for (key, value) in env {
        command.env(key, value);
    }
}

/// [`run_shell`], also reporting whether any whole output line matched `pattern` (the
/// proof's `test_passed`, decision 33). Every line is tested as it is read, so a match
/// early in a long output counts even though the tail no longer holds it.
pub(super) fn run_matching(
    dir: &Path,
    command: &str,
    env: &[(String, String)],
    timeout: Duration,
    pattern: Option<&Regex>,
) -> (ShellOutcome, bool) {
    let started = Instant::now();
    let not_started = |error: io::Error| ShellOutcome {
        ok: false,
        code: None,
        timed_out: false,
        tail: format!("could not run /bin/sh in {}: {error}", dir.display()),
        secs: started.elapsed().as_secs(),
    };
    let (mut reader, writer) = match io::pipe() {
        Ok(pipe) => pipe,
        Err(error) => return (not_started(error), false),
    };
    let spawned = {
        let stderr = match writer.try_clone() {
            Ok(stderr) => stderr,
            Err(error) => return (not_started(error), false),
        };
        let mut shell = Command::new("/bin/sh");
        shell
            .arg("-c")
            .arg(format!("{{ {command}\n}} 2>&1"))
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(writer)
            .stderr(stderr)
            .process_group(0);
        engine_env(&mut shell, env);
        shell.spawn()
        // `shell`, and with it this process's copies of the pipe's write end, drops
        // here, so the pipe reaches EOF once the command's own copies close.
    };
    let mut child = match spawned {
        Ok(child) => child,
        Err(error) => return (not_started(error), false),
    };
    let pid = child.id() as libc::pid_t;

    let mut sink = LineTail::new(pattern);
    let mut eof = match set_nonblocking(&reader) {
        Ok(()) => false,
        Err(error) => {
            tracing::debug!(
                ?error,
                "could not make an engine command's output nonblocking"
            );
            true
        }
    };
    let deadline = started + timeout;
    let mut exited_at: Option<Instant> = None;
    // `true` once `waitid` says the leader is not this process's child any more
    // (`ECHILD`: something else reaped it). Its pid, and so the group id, may then
    // already belong to another process, so neither the kill nor the reap may run.
    let mut gone = false;
    let mut timed_out = false;
    loop {
        let until = exited_at.map_or(deadline, |at| at + OUTPUT_GRACE);
        if !eof {
            eof = drain(&mut reader, &mut sink, until);
        }
        if exited_at.is_none() {
            match leader_state(pid) {
                Leader::Running => {}
                Leader::Exited => exited_at = Some(Instant::now()),
                Leader::Gone => {
                    gone = true;
                    exited_at = Some(Instant::now());
                }
            }
        }
        let now = Instant::now();
        let until = match exited_at {
            Some(_) if eof => break,
            Some(at) => at + OUTPUT_GRACE,
            None => deadline,
        };
        if now >= until {
            timed_out = exited_at.is_none();
            break;
        }
        let fd = reader.as_raw_fd();
        let open: &[std::os::fd::RawFd] = if eof { &[] } else { std::slice::from_ref(&fd) };
        wait_readable(
            open,
            until.saturating_duration_since(now).min(POLL_INTERVAL),
        );
    }

    let code = if gone {
        tracing::debug!(pid, "an engine command's shell was reaped elsewhere");
        None
    } else {
        // The leader is alive or an unreaped zombie, so its pid is still this group's
        // id.
        kill_group(pid);
        match child.wait() {
            Ok(status) => status.code(),
            Err(error) => {
                tracing::debug!(?error, pid, "could not reap an engine command");
                None
            }
        }
    };
    // A hard cap: a writer outside the group (`setsid`) that keeps the pipe full cannot
    // hold the command past it.
    let stop_reading = Instant::now() + OUTPUT_GRACE;
    while !eof {
        eof = drain(&mut reader, &mut sink, stop_reading);
        let left = stop_reading.saturating_duration_since(Instant::now());
        if eof || left.is_zero() {
            break;
        }
        wait_readable(&[reader.as_raw_fd()], left.min(POLL_INTERVAL));
    }
    let matched = sink.matched;
    let tail = sink.finish();
    let code = if timed_out { None } else { code };
    let outcome = ShellOutcome {
        ok: !timed_out && code == Some(0),
        code,
        timed_out,
        tail,
        secs: started.elapsed().as_secs(),
    };
    (outcome, matched)
}

/// Reads what is available into `sink`, but at most [`READS_PER_DRAIN`] chunks and
/// never past `until`, so writers that refill the pipe as fast as it is read cannot
/// keep the caller from its deadline (fix round 1, I1). `true` at EOF or on a read
/// error (after which nothing more can be read).
fn drain(reader: &mut io::PipeReader, sink: &mut LineTail<'_>, until: Instant) -> bool {
    let mut buffer = [0u8; 65536];
    for _ in 0..READS_PER_DRAIN {
        if Instant::now() >= until {
            return false;
        }
        match reader.read(&mut buffer) {
            Ok(0) => return true,
            Ok(count) => sink.push(&buffer[..count]),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return false,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                tracing::debug!(?error, "could not read an engine command's output");
                return true;
            }
        }
    }
    false
}

/// The group leader's state as `waitid` sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Leader {
    Running,
    /// Exited, and still an unreaped zombie of this process.
    Exited,
    /// Not this process's child (`ECHILD`, or any other `waitid` error): stop waiting,
    /// and neither signal nor reap it.
    Gone,
}

/// Whether the group leader has exited, without reaping it (`WNOWAIT`), so that
/// [`kill_group`] afterwards still addresses this command's group and no other.
fn leader_state(pid: libc::pid_t) -> Leader {
    // SAFETY: an all-zero `siginfo_t` is a valid value for waitid to overwrite.
    let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
    // SAFETY: `info` is a live, writable `siginfo_t`; `pid` is this process's own
    // unreaped child, and `WNOWAIT` leaves it waitable for `Child::wait`.
    let result = unsafe {
        libc::waitid(
            libc::P_PID,
            pid as libc::id_t,
            &mut info,
            libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
        )
    };
    if result == -1 {
        let error = io::Error::last_os_error();
        if error.kind() == io::ErrorKind::Interrupted {
            return Leader::Running;
        }
        tracing::debug!(?error, pid, "waitid on an engine command failed");
        return Leader::Gone;
    }
    if siginfo_pid(&info) != 0 {
        Leader::Exited
    } else {
        Leader::Running
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn siginfo_pid(info: &libc::siginfo_t) -> libc::pid_t {
    // SAFETY: waitid filled `info` for a child-state change (or left it zeroed).
    unsafe { info.si_pid() }
}

#[cfg(not(any(target_os = "linux", target_os = "android")))]
fn siginfo_pid(info: &libc::siginfo_t) -> libc::pid_t {
    info.si_pid
}

fn kill_group(pid: libc::pid_t) {
    // SAFETY: the command was started as its own group's leader (`process_group(0)`,
    // so pid == pgid) and has not been reaped, so the group id is still this command's.
    if unsafe { libc::killpg(pid, libc::SIGKILL) } == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            tracing::debug!(?error, pid, "could not kill an engine command's group");
        }
    }
}

/// The bounded line reader behind [`ShellOutcome::tail`]: the last
/// [`CHECK_TAIL_LINES`] lines, each cut to [`LINE_MAX_CHARS`] characters after lossy
/// decoding, holding at most one partial line's first bytes however long it runs.
struct LineTail<'a> {
    lines: VecDeque<String>,
    current: Vec<u8>,
    /// Whether any byte of the current line has been read (an empty line still counts).
    in_line: bool,
    keep: usize,
    pattern: Option<&'a Regex>,
    matched: bool,
}

impl<'a> LineTail<'a> {
    fn new(pattern: Option<&'a Regex>) -> Self {
        LineTail {
            lines: VecDeque::with_capacity(CHECK_TAIL_LINES + 1),
            current: Vec::new(),
            in_line: false,
            keep: if pattern.is_some() {
                MATCH_LINE_BYTES
            } else {
                TAIL_LINE_BYTES
            },
            pattern,
            matched: false,
        }
    }

    fn push(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            let (part, newline) = match bytes.iter().position(|&b| b == b'\n') {
                Some(at) => (&bytes[..at], true),
                None => (bytes, false),
            };
            let room = self.keep.saturating_sub(self.current.len());
            self.current
                .extend_from_slice(&part[..room.min(part.len())]);
            self.in_line = true;
            if newline {
                self.end_line();
                bytes = &bytes[part.len() + 1..];
            } else {
                bytes = &[];
            }
        }
    }

    fn end_line(&mut self) {
        // One trailing `\r` goes (a CRLF runner's line end), before matching and storing,
        // so a `$`-anchored `test_passed` matches (fix round 1, M3).
        if self.current.last() == Some(&b'\r') {
            self.current.pop();
        }
        let text = String::from_utf8_lossy(&self.current);
        if let Some(pattern) = self.pattern
            && !self.matched
            && pattern.is_match(&text)
        {
            self.matched = true;
        }
        let cut: String = text.chars().take(LINE_MAX_CHARS).collect();
        self.lines.push_back(cut);
        if self.lines.len() > CHECK_TAIL_LINES {
            self.lines.pop_front();
        }
        self.current.clear();
        self.in_line = false;
    }

    fn finish(mut self) -> String {
        if self.in_line {
            self.end_line();
        }
        let lines: Vec<String> = self.lines.into_iter().collect();
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    fn tail_of(chunks: &[&[u8]]) -> String {
        let mut sink = LineTail::new(None);
        for chunk in chunks {
            sink.push(chunk);
        }
        sink.finish()
    }

    #[test]
    fn lines_split_across_reads_are_joined() {
        assert_eq!(tail_of(&[b"ab", b"c\nd", b"e\n"]), "abc\nde");
        assert_eq!(tail_of(&[b"a\n\nb"]), "a\n\nb");
        assert_eq!(tail_of(&[b""]), "");
    }

    #[test]
    fn a_character_split_across_reads_is_kept_whole() {
        let wide = "世".as_bytes();
        assert_eq!(tail_of(&[&wide[..1], &wide[1..], b"\n"]), "世");
    }

    #[test]
    fn a_match_beyond_the_cut_counts() {
        let pattern = Regex::new("PASS t").unwrap();
        let mut sink = LineTail::new(Some(&pattern));
        let mut line = vec![b'x'; 1000];
        line.extend_from_slice(b"PASS t\n");
        sink.push(&line);
        assert!(sink.matched);
        assert_eq!(sink.finish().len(), LINE_MAX_CHARS);
    }

    #[test]
    fn a_pid_that_is_not_our_child_is_gone() {
        // pid 1 is never this process's child: `waitid` gives `ECHILD`.
        assert_eq!(leader_state(1), Leader::Gone);
    }

    #[test]
    fn summary_of_an_empty_tail_is_empty() {
        assert_eq!(summary(""), "");
    }

    #[test]
    fn engine_env_is_applied_to_the_command() {
        let mut command = Command::new("env");
        engine_env(&mut command, &[("A".into(), "1".into())]);
        let envs: Vec<(&OsStr, Option<&OsStr>)> = command.get_envs().collect();
        assert!(envs.contains(&(OsStr::new("CLAUDECODE"), None)));
        assert!(envs.contains(&(OsStr::new("ANTHREX_WINDOW_ID"), None)));
        assert!(envs.contains(&(OsStr::new("GIT_DIR"), None)));
        assert!(envs.contains(&(OsStr::new("A"), Some(OsStr::new("1")))));
    }
}
