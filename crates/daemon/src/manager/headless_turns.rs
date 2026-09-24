//! Turns of a headless session (decisions 28, 29 and 52): starting a session process,
//! delivering an engine message as one new turn, interrupting it, and resuming an ended
//! session.
//!
//! Every method takes what it needs out from under the manager lock and drops the guard
//! before it spawns, writes, kills or awaits (AGENTS.md rule 2). Each turn is recorded in
//! the window's cursor (`headless::conversation::sent_turn`) before its message is
//! written to Claude's stdin or its `codex exec` process is spawned (M8a.7 fix round 2),
//! and what is recorded is exactly what is sent: the text after `run::messages::clamp`.

use super::WindowManager;
use super::entry::Process;
use super::headless::hooks_fire;
use crate::headless::argv::{CLI_CAPS, InterruptMode, claude_args, codex_args};
use crate::headless::claude_stream::user_message;
use crate::headless::conversation;
use crate::headless::session::{HeadlessHandle, OUTPUT_GRACE};
use crate::headless::{HeadlessSpec, SessionArg};
use crate::run::messages::clamp;
use crate::run::role_launch::jitter_ms;
use proto::{Runtime, Status, WindowInfo};
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::sync::oneshot;

/// How long a resumed process has to print its `Init` (invented). Past it the resume is
/// reported failed and the process killed, so a session that hangs before it starts
/// cannot hold its task forever; a real CLI starts in seconds.
pub const RESUME_START_TIMEOUT: Duration = Duration::from_secs(120);

/// Claude's control requests are numbered per daemon; the id only has to be unique.
static INTERRUPT_REQUESTS: AtomicU64 = AtomicU64::new(1);

/// Decision 26: the session's own window id and socket, exactly as `launch::plan` sets
/// them for a PTY window (`anthrex hook` reads both), after the profile's env so the
/// profile cannot point a session's hooks at another window.
fn session_env(
    window_id: u32,
    socket: &Path,
    profile: &[(String, String)],
) -> Vec<(String, String)> {
    let mut env = profile.to_vec();
    env.push(("ANTHREX_WINDOW_ID".into(), window_id.to_string()));
    env.push(("ANTHREX_SOCKET".into(), socket.display().to_string()));
    env
}

/// Decision 18's launch jitter for a new `codex exec resume` process of the session.
fn launch_jitter(spec: &HeadlessSpec) -> Duration {
    let ms = spec.run_ref.as_ref().map_or(0, |run| {
        jitter_ms(
            &run.run_id,
            run.task_id.as_deref().unwrap_or(""),
            run.session,
        )
    });
    Duration::from_millis(ms)
}

fn running(id: u32) -> anyhow::Error {
    anyhow::anyhow!("a turn is already running for window {id}")
}

fn ended(id: u32) -> anyhow::Error {
    anyhow::anyhow!("session for window {id} has ended; resume it")
}

/// A send or resume in progress on one window. Dropping it, however the operation
/// ended, frees the window for the next one and forgets an unanswered resume waiter.
struct TurnClaim<'a> {
    manager: &'a WindowManager,
    id: u32,
}

impl Drop for TurnClaim<'_> {
    fn drop(&mut self) {
        let mut inner = crate::lock(&self.manager.inner);
        if let Some(entry) = inner.entries.get_mut(&self.id)
            && let Process::Headless(window) = &mut entry.process
        {
            window.busy = false;
            window.start_waiter = None;
            window.start_failure = None;
        }
    }
}

impl WindowManager {
    /// The argv of one process of window `id`'s session: Claude's `claude_args` (the
    /// message goes on stdin), or Codex's `codex_args` with `message` last. Both carry
    /// every flag of the spec on every launch and resume (decisions 24, 25 and 53).
    pub(super) fn session_args(
        &self,
        spec: &HeadlessSpec,
        session: &SessionArg,
        message: &str,
        id: u32,
    ) -> Vec<String> {
        let (exe, socket) = (&self.config.exe, &self.config.socket_path);
        match spec.runtime {
            Runtime::Claude => claude_args(spec, session, exe, id, socket, &CLI_CAPS),
            _ => codex_args(spec, session, message, exe, id, socket, &CLI_CAPS),
        }
    }

    /// Starts one process of window `id`'s session on a blocking thread, then queues
    /// `line` (Claude's message) on its stdin. The process's events go to
    /// `apply_session_event` from the moment it starts. Takes no lock.
    pub(super) async fn spawn_process(
        self: &Arc<Self>,
        id: u32,
        spec: &HeadlessSpec,
        args: Vec<String>,
        line: Option<String>,
    ) -> anyhow::Result<HeadlessHandle> {
        let runtime = spec.runtime;
        let program = match runtime {
            Runtime::Claude => self.config.claude_bin.clone(),
            _ => self.config.codex_bin.clone(),
        };
        let env = session_env(id, &self.config.socket_path, &spec.env);
        let cwd = spec.cwd.clone();
        let weak = Arc::downgrade(self);
        let handle = tokio::task::spawn_blocking(move || {
            HeadlessHandle::spawn(
                runtime,
                program.as_ref(),
                &args,
                &cwd,
                &env,
                move |pid, event| {
                    if let Some(manager) = weak.upgrade() {
                        manager.apply_session_event(id, pid, &event);
                    }
                },
            )
        })
        .await
        .map_err(|error| anyhow::anyhow!("headless session start failed: {error}"))??;
        if let Some(line) = line
            && let Err(error) = handle.send_line(line)
        {
            handle.kill(self.config.kill_grace);
            return Err(error.context("could not write the turn's message"));
        }
        Ok(handle)
    }

    /// Installs a started process as window `id`'s current one, under the lock. A window
    /// that is gone, or a daemon shutting down, gets the process killed instead (after
    /// the guard is dropped).
    pub(super) fn install(&self, id: u32, handle: &HeadlessHandle) -> anyhow::Result<WindowInfo> {
        let mut inner = crate::lock(&self.inner);
        let shutting_down = inner.shutting_down;
        let installed = !shutting_down
            && match inner.entries.get_mut(&id) {
                Some(entry) => match &mut entry.process {
                    Process::Headless(window) => {
                        window.handle = handle.clone();
                        // The exit may already have been applied (the reap comes before
                        // it), so this reads the handle rather than assuming it lives.
                        entry.child_alive = handle.pid().is_some();
                        true
                    }
                    _ => false,
                },
                None => false,
            };
        if !installed {
            drop(inner);
            handle.kill(self.config.kill_grace);
            if shutting_down {
                anyhow::bail!("daemon is shutting down");
            }
            anyhow::bail!("window {id} was removed while its session started");
        }
        let info = inner.entries[&id].info(Instant::now());
        self.publish(&inner);
        Ok(info)
    }

    /// Records a turn the daemon is about to send, under the lock and before anything is
    /// written or spawned (M8a.7 fix round 2), and clears the window's handle for the
    /// process about to start: its events are then the window's from the first one. The
    /// previous process has delivered its last event by now ([`Self::retire`]), so none
    /// of its events can follow. With `wait_start`, returns the receiver a resume awaits.
    fn record_turn(
        &self,
        id: u32,
        text: &str,
        wait_start: bool,
    ) -> anyhow::Result<Option<oneshot::Receiver<Result<(), String>>>> {
        let mut inner = crate::lock(&self.inner);
        anyhow::ensure!(!inner.shutting_down, "daemon is shutting down");
        let entry = inner
            .entries
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        let Process::Headless(window) = &mut entry.process else {
            anyhow::bail!("window {id} is not a headless session");
        };
        let runtime = window.spec.runtime;
        window.handle = HeadlessHandle::ended();
        let started = wait_start.then(|| {
            let (tx, rx) = oneshot::channel();
            window.start_waiter = Some(tx);
            window.start_failure = None;
            rx
        });
        let input = conversation::sent_turn(runtime, hooks_fire(runtime), text, &mut window.cursor);
        self.apply_input(id, entry, input, Instant::now());
        Ok(started)
    }

    /// Makes sure no process of the session is left before another starts (decision 52):
    /// waits until `old` has delivered its last event, killing it first when `kill` (or
    /// when it outstays its grace). Blocking waits run on `spawn_blocking`.
    async fn retire(&self, id: u32, old: &HeadlessHandle, kill: bool) -> anyhow::Result<()> {
        if old.spawned_pid().is_none() {
            return Ok(());
        }
        let grace = self.config.kill_grace;
        // The leader is killed at `grace` at the latest, and its output is read for at
        // most `OUTPUT_GRACE` after; one more second for the threads to finish.
        let bound = grace + OUTPUT_GRACE + Duration::from_secs(1);
        if kill {
            old.kill(grace);
        }
        let old = old.clone();
        let finished = tokio::task::spawn_blocking(move || {
            old.wait_finished(bound) || {
                old.kill(grace);
                old.wait_finished(bound)
            }
        })
        .await
        .map_err(|error| anyhow::anyhow!("waiting for a session process failed: {error}"))?;
        anyhow::ensure!(
            finished,
            "the previous process of window {id} did not stop; its session cannot run twice"
        );
        Ok(())
    }

    /// One new turn with `text`, clamped to `MESSAGE_MAX_BYTES` (decision 29):
    ///
    /// - Claude: one stream-json user message on the running process's stdin, through
    ///   its writer thread. An ended process is an error: the engine resumes it instead.
    /// - Codex: a new `codex exec resume <session> … -- <text>` process after the launch
    ///   jitter, once the previous turn's process has exited. A turn still running is an
    ///   error, not a queue: the engine never delivers into an open turn.
    pub async fn headless_send(self: &Arc<Self>, id: u32, text: &str) -> anyhow::Result<()> {
        let text = clamp(text);
        let (runtime, handle, spec, session_id) = {
            let mut inner = crate::lock(&self.inner);
            let entry = inner
                .entries
                .get_mut(&id)
                .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
            let session_id = entry.state.session_id.clone();
            let Process::Headless(window) = &mut entry.process else {
                anyhow::bail!("window {id} is not a headless session");
            };
            let runtime = window.spec.runtime;
            if runtime == Runtime::Claude {
                if window.handle.is_ended() {
                    return Err(ended(id));
                }
                if window.busy {
                    return Err(running(id));
                }
                let handle = window.handle.clone();
                let input = conversation::sent_turn(
                    runtime,
                    hooks_fire(runtime),
                    &text,
                    &mut window.cursor,
                );
                self.apply_input(id, entry, input, Instant::now());
                drop(inner);
                return handle.send_line(user_message(&text, session_id.as_deref()));
            }
            if window.status.status == Status::Exited {
                return Err(ended(id));
            }
            if window.busy || (!window.handle.is_ended() && !window.turn_ended_in_process) {
                return Err(running(id));
            }
            let session_id = session_id
                .ok_or_else(|| anyhow::anyhow!("session for window {id} has no session id yet"))?;
            window.busy = true;
            (
                runtime,
                window.handle.clone(),
                window.spec.clone(),
                session_id,
            )
        };
        let _claim = TurnClaim { manager: self, id };
        debug_assert_eq!(runtime, Runtime::Codex);
        // The previous turn has ended; its process only has to finish exiting.
        self.retire(id, &handle, false).await?;
        tokio::time::sleep(launch_jitter(&spec)).await;
        self.record_turn(id, &text, false)?;
        let session = SessionArg::Resume { session_id };
        let args = self.session_args(&spec, &session, &text, id);
        let handle = self.spawn_process(id, &spec, args, None).await?;
        self.install(id, &handle)?;
        Ok(())
    }

    /// Resumes session `session_id` in window `id` with `message` (decision 28), after
    /// `jitter` (the op's decision 18 launch jitter):
    ///
    /// - A live process of the window is stopped first, and its last event delivered,
    ///   so one session never runs in two processes (decision 52).
    /// - Claude: `claude_args` with `--resume <id>` and every flag re-passed, then the
    ///   message on stdin. Codex: `codex exec resume <id> … -- <message>`, stdin closed.
    /// - It returns once the process prints its `Init`. An exit before one is an error
    ///   naming why (M8a.1's `No conversation found with session ID: <id>`, or the exit),
    ///   which the driver reports as `ResumeFailed`.
    pub async fn headless_resume(
        self: &Arc<Self>,
        id: u32,
        session_id: &str,
        message: &str,
        jitter: Duration,
    ) -> anyhow::Result<()> {
        let text = clamp(message);
        let (old, spec) = {
            let mut inner = crate::lock(&self.inner);
            let entry = inner
                .entries
                .get_mut(&id)
                .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
            let Process::Headless(window) = &mut entry.process else {
                anyhow::bail!("window {id} is not a headless session");
            };
            if window.busy {
                return Err(running(id));
            }
            window.busy = true;
            (window.handle.clone(), window.spec.clone())
        };
        let _claim = TurnClaim { manager: self, id };
        self.retire(id, &old, true).await?;
        tokio::time::sleep(jitter).await;
        let started = self
            .record_turn(id, &text, true)?
            .expect("a resume waits for its start");
        let session = SessionArg::Resume {
            session_id: session_id.to_owned(),
        };
        let args = self.session_args(&spec, &session, &text, id);
        let line = (spec.runtime == Runtime::Claude).then(|| user_message(&text, Some(session_id)));
        let handle = self.spawn_process(id, &spec, args, line).await?;
        self.install(id, &handle)?;
        match tokio::time::timeout(RESUME_START_TIMEOUT, started).await {
            Ok(Ok(Ok(()))) => Ok(()),
            Ok(Ok(Err(reason))) => {
                anyhow::bail!("could not resume session {session_id}: {reason}")
            }
            Ok(Err(_)) => anyhow::bail!("window {id} was removed while its session resumed"),
            Err(_) => {
                handle.kill(self.config.kill_grace);
                anyhow::bail!(
                    "could not resume session {session_id}: it did not start within {}s",
                    RESUME_START_TIMEOUT.as_secs()
                )
            }
        }
    }

    /// The window's handle and runtime, cloned out from under the lock.
    fn headless_handle(&self, id: u32) -> anyhow::Result<(HeadlessHandle, Runtime)> {
        let inner = crate::lock(&self.inner);
        let entry = inner
            .entries
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("no window with id {id}"))?;
        match &entry.process {
            Process::Headless(window) => Ok((window.handle.clone(), window.spec.runtime)),
            _ => anyhow::bail!("window {id} is not a headless session"),
        }
    }

    /// Interrupts the running turn: Claude by `CLI_CAPS.claude_interrupt` (a control
    /// request on stdin, or `SIGINT`), Codex always by `SIGINT` to the turn's process.
    pub fn headless_interrupt(&self, id: u32) -> anyhow::Result<()> {
        let (handle, runtime) = self.headless_handle(id)?;
        let mode = match runtime {
            Runtime::Claude => CLI_CAPS.claude_interrupt,
            _ => InterruptMode::Sigint,
        };
        handle.interrupt(mode, INTERRUPT_REQUESTS.fetch_add(1, Ordering::Relaxed))
    }

    /// `SIGTERM` to the session's group, `SIGKILL` after the kill grace (decision 52).
    pub fn headless_kill(&self, id: u32) -> anyhow::Result<()> {
        let (handle, _) = self.headless_handle(id)?;
        handle.kill(self.config.kill_grace);
        Ok(())
    }
}
