//! The headless session driver (decisions 26, 27 and 52): one agent process with piped
//! stdin, stdout and stderr and no terminal, in its own process group.
//!
//! With its `pipes` submodule (the threads), the only code under `headless/` that does
//! I/O. Everything here runs on dedicated threads, never on a tokio worker and never
//! under the manager lock (AGENTS.md rule 2):
//!
//! - a **writer** thread owns stdin and drains a bounded queue ([`WRITER_QUEUE_MAX`]
//!   lines), so [`HeadlessHandle::send_line`] only enqueues and never blocks on a full
//!   pipe;
//! - a **stdout** and a **stderr** reader, each cutting its lines;
//! - a **waiter** that sees the leader exit with `waitid(WNOWAIT)`, kills whatever is
//!   left of its group while the leader is still an unreaped zombie (so the group id
//!   cannot have been reused), then reaps it;
//! - one **dispatcher** that parses stdout lines and calls `on_event`, so a session's
//!   events reach the caller from one thread, in one order.
//!
//! **The ordering contract** (ruling T13-P1, relied on by `run::engine`'s signals): every
//! event carries the pid of the process that produced it; `ProcessStarted { pid }` is
//! the first event of each process; `ProcessExited` is its last, after every line its
//! stdout and stderr delivered, or after [`OUTPUT_GRACE`] when a process that escaped
//! the group still holds a pipe open (lines after that are dropped).

mod pipes;

use super::argv::InterruptMode;
use super::claude_stream;
use super::{SessionEvent, UNKNOWN_LINE_CHARS};
use crate::subprocess::scrub_git_env;
use anyhow::Context;
use pipes::{dispatch, read_stderr, read_stdout, wait_leader, write_lines};
use proto::Runtime;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// A stdout line longer than this is cut, and reported as `Unknown` with its first
/// [`UNKNOWN_LINE_CHARS`] characters; the rest of it is read and discarded.
pub const STDOUT_LINE_MAX: usize = 4 * 1024 * 1024;
/// Lines queued to a session's stdin writer thread. `send_line` fails once it is full.
pub const WRITER_QUEUE_MAX: usize = 256;
/// Inherited variables removed from every session's environment (decision 26), by
/// prefix and by name.
pub const SCRUB_PREFIXES: &[&str] = &["CLAUDE_CODE_"];
pub const SCRUB_NAMES: &[&str] = &["CLAUDECODE"];
/// A stderr line is cut to this many bytes (invented: stderr is only diagnosis).
pub const STDERR_LINE_MAX: usize = 4096;
/// How long output is still read after the process has exited and its group has been
/// killed, the same rule and value as the engine's own commands (`run::exec`).
pub use crate::run::exec::OUTPUT_GRACE;

/// M8a.1 item 4b: Claude's text, on stderr before `system/init`, when
/// `failIfUnavailable` stops it starting without its sandbox.
const SANDBOX_UNAVAILABLE: &str = "sandbox required but unavailable";
/// Bytes of a cut stdout line kept for its `Unknown` event: enough for
/// [`UNKNOWN_LINE_CHARS`] characters of any width.
const CUT_KEEP_BYTES: usize = UNKNOWN_LINE_CHARS * 4;

/// One session process, or none ([`HeadlessHandle::ended`]). Cheap to clone: every
/// clone drives the same process, so the manager can take one out from under its lock
/// and act on it after releasing it.
#[derive(Clone, Default)]
pub struct HeadlessHandle {
    process: Option<Arc<Process>>,
}

impl std::fmt::Debug for HeadlessHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HeadlessHandle")
            .field("pid", &self.spawned_pid())
            .field("ended", &self.is_ended())
            .finish()
    }
}

struct Process {
    pid: u32,
    /// `true` once the leader is reaped. Held across every signal and across the reap,
    /// so a signal can never reach a reused pid or group id.
    reaped: Mutex<bool>,
    /// Notified at the reap, for `kill`'s grace.
    exited: Condvar,
    /// `None` once stdin is closed.
    stdin: Mutex<Option<SyncSender<String>>>,
    /// `true` once the dispatcher has delivered `ProcessExited`, the process's last
    /// event: nothing it said can reach the caller after that.
    finished: Mutex<bool>,
    /// Notified when `finished` is set.
    finished_cv: Condvar,
}

impl Process {
    fn finish(&self) {
        *crate::lock(&self.finished) = true;
        self.finished_cv.notify_all();
    }
}

impl HeadlessHandle {
    /// A handle with no process: a restored window's (decision 28), or one whose
    /// process has not been started yet.
    pub fn ended() -> Self {
        Self::default()
    }

    /// Starts `program` in `cwd` as its own process group's leader, with every pipe
    /// connected and decision 26's environment: the inherited `CLAUDE_CODE_*` variables,
    /// `CLAUDECODE`, `ANTHREX_WINDOW_ID`, `ANTHREX_SOCKET` and AGENTS.md rule 11's git
    /// variables removed, then `env` set (the caller's window id and socket, and the
    /// profile's env). `on_event(pid, event)` receives every event of the process, from
    /// one thread, in the order the module doc describes.
    ///
    /// Blocking (a `fork`/`exec` and thread starts): call it from `spawn_blocking` or a
    /// thread, never from a tokio worker.
    pub fn spawn(
        runtime: Runtime,
        program: &OsStr,
        args: &[String],
        cwd: &Path,
        env: &[(String, String)],
        on_event: impl Fn(u32, SessionEvent) + Send + Sync + 'static,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            matches!(runtime, Runtime::Claude | Runtime::Codex),
            "a headless session runs claude or codex, not {}",
            runtime.label()
        );
        let mut command = Command::new(program);
        command
            .args(args)
            .current_dir(cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        session_env(&mut command, env);
        let mut child = command
            .spawn()
            .with_context(|| format!("could not start {}", program.to_string_lossy()))?;
        let pid = child.id();
        let (stdin, stdout, stderr) = (
            child.stdin.take().expect("piped"),
            child.stdout.take().expect("piped"),
            child.stderr.take().expect("piped"),
        );
        // The waiter reaps the leader itself (`waitpid`), so `child` is dropped
        // unwaited; dropping a `Child` neither waits nor kills.
        drop(child);

        let (queue, lines) = mpsc::sync_channel::<String>(WRITER_QUEUE_MAX);
        // Ruling T17-C1: `codex exec` (0.156.1) reads a piped stdin to EOF before it
        // starts its turn, so a Codex process's stdin is closed at once (the writer sees
        // no sender and drops the pipe). Claude keeps it open for stream-json input.
        let queue = (runtime == Runtime::Claude).then_some(queue);
        let process = Arc::new(Process {
            pid,
            reaped: Mutex::new(false),
            exited: Condvar::new(),
            stdin: Mutex::new(queue),
            finished: Mutex::new(false),
            finished_cv: Condvar::new(),
        });
        let (tx, rx) = mpsc::channel();
        let name = |role: &str| format!("headless-{role}-{pid}");
        let dispatched = process.clone();
        let started = std::thread::Builder::new()
            .name(name("dispatch"))
            .spawn(move || {
                dispatch(pid, runtime, rx, on_event);
                dispatched.finish();
            })
            .and_then(|_| {
                std::thread::Builder::new()
                    .name(name("stdin"))
                    .spawn(move || write_lines(stdin, lines))
            })
            .and_then(|_| {
                let tx = tx.clone();
                std::thread::Builder::new()
                    .name(name("stdout"))
                    .spawn(move || read_stdout(stdout, tx))
            })
            .and_then(|_| {
                let tx = tx.clone();
                std::thread::Builder::new()
                    .name(name("stderr"))
                    .spawn(move || read_stderr(stderr, tx))
            })
            .and_then(|_| {
                let process = process.clone();
                std::thread::Builder::new()
                    .name(name("wait"))
                    .spawn(move || wait_leader(&process, tx))
            });
        if let Err(error) = started {
            let handle = Self {
                process: Some(process.clone()),
            };
            handle.signal(libc::SIGKILL, true);
            // The dispatcher may never have started; nothing more will be delivered.
            process.finish();
            return Err(error).context("could not start a session thread");
        }
        Ok(Self {
            process: Some(process),
        })
    }

    /// Queues one line (a newline is added) for the session's stdin. Never blocks: it
    /// fails once [`WRITER_QUEUE_MAX`] lines are waiting, when stdin is closed (always,
    /// for Codex), or when the process has ended.
    pub fn send_line(&self, line: String) -> anyhow::Result<()> {
        anyhow::ensure!(
            !line.contains('\n'),
            "a stdin line must not contain a newline"
        );
        let process = self.process.as_ref().context("the session has ended")?;
        // The writer thread may not have noticed the exit yet; the reap is the truth.
        anyhow::ensure!(!*crate::lock(&process.reaped), "the session has ended");
        let stdin = crate::lock(&process.stdin);
        let queue = stdin.as_ref().context("the session's stdin is closed")?;
        match queue.try_send(line) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => anyhow::bail!(
                "the session's stdin queue is full ({WRITER_QUEUE_MAX} lines waiting)"
            ),
            Err(TrySendError::Disconnected(_)) => {
                anyhow::bail!("the session's stdin is closed")
            }
        }
    }

    /// Closes stdin once the queued lines are written: Claude ends on EOF (decision 52).
    pub fn close_stdin(&self) {
        if let Some(process) = &self.process {
            crate::lock(&process.stdin).take();
        }
    }

    /// Interrupts the running turn: Claude's control request on stdin, or `SIGINT` to
    /// the leader alone (the group also holds the session's MCP server, which must
    /// survive the turn).
    pub fn interrupt(&self, mode: InterruptMode, request_id: u64) -> anyhow::Result<()> {
        anyhow::ensure!(!self.is_ended(), "the session has ended");
        match mode {
            InterruptMode::ControlRequest => {
                self.send_line(claude_stream::interrupt_request(request_id))
            }
            InterruptMode::Sigint => {
                anyhow::ensure!(self.signal(libc::SIGINT, false), "the session has ended");
                Ok(())
            }
        }
    }

    /// `SIGTERM` to the group now and `SIGKILL` after `grace` unless the leader has
    /// exited by then, on a thread of its own: returns at once. Stdin is closed too.
    pub fn kill(&self, grace: Duration) {
        let Some(process) = self.process.clone() else {
            return;
        };
        self.close_stdin();
        if !self.signal(libc::SIGTERM, true) {
            return;
        }
        let spawned = std::thread::Builder::new()
            .name(format!("headless-kill-{}", process.pid))
            .spawn(move || {
                let deadline = Instant::now() + grace;
                let mut reaped = crate::lock(&process.reaped);
                while !*reaped {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        signal_locked(process.pid, libc::SIGKILL, true);
                        return;
                    }
                    reaped = process
                        .exited
                        .wait_timeout(reaped, left)
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .0;
                }
            });
        if spawned.is_err() {
            self.signal(libc::SIGKILL, true);
        }
    }

    /// Sends `signal` to the whole group, unless the leader has been reaped. For the
    /// manager's generic window paths (`remove`, `shutdown`'s escalation).
    pub(crate) fn signal_group(&self, signal: i32) {
        self.signal(signal, true);
    }

    /// The running process's pid; `None` once it has been reaped, so the pid cannot be
    /// mistaken for a later process's.
    pub fn pid(&self) -> Option<u32> {
        let process = self.process.as_ref()?;
        (!*crate::lock(&process.reaped)).then_some(process.pid)
    }

    /// The pid the process was started with, reaped or not: what its events carry.
    pub fn spawned_pid(&self) -> Option<u32> {
        self.process.as_ref().map(|p| p.pid)
    }

    /// Waits until the process's last event (`ProcessExited`) has been delivered, at
    /// most `timeout`; `true` when it has, or when there is no process. After it, no event
    /// of this process can reach the caller: a replacement process may start on the same
    /// session (decision 52).
    ///
    /// Blocking: call it from `spawn_blocking` or a thread, never from a tokio worker.
    pub fn wait_finished(&self, timeout: Duration) -> bool {
        let Some(process) = &self.process else {
            return true;
        };
        let deadline = Instant::now() + timeout;
        let mut finished = crate::lock(&process.finished);
        while !*finished {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            finished = process
                .finished_cv
                .wait_timeout(finished, left)
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .0;
        }
        true
    }

    /// No process, or its leader has been reaped.
    pub fn is_ended(&self) -> bool {
        self.pid().is_none()
    }

    /// `true` when the signal was sent (the leader was not yet reaped).
    fn signal(&self, signal: i32, group: bool) -> bool {
        let Some(process) = &self.process else {
            return false;
        };
        let reaped = crate::lock(&process.reaped);
        if *reaped {
            return false;
        }
        signal_locked(process.pid, signal, group);
        true
    }
}

/// Decision 26's environment on `command`, then `env` on top.
fn session_env(command: &mut Command, env: &[(String, String)]) {
    scrub_git_env(command);
    for (key, _) in std::env::vars_os() {
        let bytes = key.as_bytes();
        if SCRUB_PREFIXES
            .iter()
            .any(|prefix| bytes.starts_with(prefix.as_bytes()))
        {
            command.env_remove(&key);
        }
    }
    for name in SCRUB_NAMES
        .iter()
        .chain(&["ANTHREX_WINDOW_ID", "ANTHREX_SOCKET"])
    {
        command.env_remove(name);
    }
    for (key, value) in env {
        command.env(key, value);
    }
}

/// Only called with the process's `reaped` lock held and `false`: the leader is alive or
/// an unreaped zombie, so its pid is still this session's, and so is its group.
fn signal_locked(pid: u32, signal: i32, group: bool) {
    let pid = pid as libc::pid_t;
    // SAFETY: plain signal syscalls on a pid (group) this process started and has not
    // reaped (see above).
    let result = unsafe {
        if group {
            libc::killpg(pid, signal)
        } else {
            libc::kill(pid, signal)
        }
    };
    if result == -1 {
        let error = std::io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::ESRCH) {
            tracing::debug!(?error, pid, signal, group, "could not signal a session");
        }
    }
}
