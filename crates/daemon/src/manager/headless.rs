//! Headless run sessions as windows with no terminal (decisions 27, 28, 49 and 52): the
//! manager side of `crate::headless`.
//!
//! A headless window is an ordinary [`Entry`] whose [`Process`] is
//! `Process::Headless(HeadlessWindow)`, so the machinery milestones 3 to 6.5 key by
//! window id keeps working: the list, sub-agent rows, persistence, the conversation view.
//! What differs:
//!
//! - its status comes from its stream (`headless::status::next`), never from hooks or
//!   the PTY rules;
//! - its conversation comes from its stream and its real hooks, through one
//!   `StreamCursor` kept for the window's whole life;
//! - every session event, and every `SubagentStart`/`SubagentStop` hook, is published on
//!   the engine's feed ([`WindowManager::signals`]).
//!
//! Only [`WindowManager::apply_session_event`]'s status, conversation and feed updates run
//! under the manager lock. Spawning, writing and killing happen after the guard is dropped
//! (AGENTS.md rule 2): every method here clones the window's [`HeadlessHandle`] out from
//! under the lock and acts on the clone.

use super::entry::{Entry, Process};
use super::{WindowManager, validate_name};
use crate::agent_state::AgentState;
use crate::headless::argv::{CLI_CAPS, InterruptMode, claude_args, codex_args};
use crate::headless::claude_stream::user_message;
use crate::headless::conversation::{self, ConversationInput, StreamCursor};
use crate::headless::session::HeadlessHandle;
use crate::headless::status::{self, HeadlessStatus};
use crate::headless::{HeadlessSpec, SessionArg, SessionEvent};
use crate::hooks::{HookKind, ParsedHook};
use proto::{ExitInfo, RunRef, Runtime, Status, WindowInfo, WindowSpec};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;
use tokio::sync::broadcast;

/// Capacity of the engine's feed. A receiver that lags logs a warning and continues
/// (decision 27); counts may then be low, which budgets tolerate.
pub const SIGNAL_CHANNEL_CAPACITY: usize = 4096;
/// The last unrecognised or diagnostic lines a headless window keeps (decision 27).
pub const DIAGNOSTIC_LINES: usize = 10;

/// One thing the engine hears about a window (decision 27).
///
/// `pid` is the process the event came from (`None` for a hook). The driver's ordering
/// contract (`headless::session`'s module doc) holds per pid: `ProcessStarted` first,
/// `ProcessExited` last, so the engine can tell a late event of a replaced process from
/// one of the process it is waiting on.
#[derive(Debug, Clone, PartialEq)]
pub struct WindowSignal {
    pub window_id: u32,
    pub pid: Option<u32>,
    pub kind: WindowSignalKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WindowSignalKind {
    Session(SessionEvent),
    /// A real `SubagentStart` or `SubagentStop` hook of a headless window.
    Hook {
        kind: HookKind,
        agent_id: Option<String>,
    },
}

/// What a headless window runs and remembers, in place of a PTY.
pub(super) struct HeadlessWindow {
    pub(super) handle: HeadlessHandle,
    pub(super) spec: HeadlessSpec,
    pub(super) status: HeadlessStatus,
    /// One per window for its whole life, across `--resume` processes, so its counts
    /// keep matching the conversation's (ruling T7-N1).
    pub(super) cursor: StreamCursor,
    pub(super) diagnostics: VecDeque<String>,
}

impl HeadlessWindow {
    pub(super) fn new(spec: HeadlessSpec) -> Self {
        HeadlessWindow {
            handle: HeadlessHandle::ended(),
            spec,
            status: HeadlessStatus::default(),
            cursor: StreamCursor::default(),
            diagnostics: VecDeque::new(),
        }
    }

    /// A window restored from the state file: its process died with the daemon.
    pub(super) fn restored(spec: HeadlessSpec) -> Self {
        let mut window = Self::new(spec);
        window.status.status = Status::Exited;
        window
    }
}

/// Whether the runtime's own hooks build the conversation's turns (Claude, M8a.1), or
/// `headless::conversation` synthesises them from the stream (Codex).
fn hooks_fire(runtime: Runtime) -> bool {
    runtime == Runtime::Claude && CLI_CAPS.claude_hooks_fire_in_print
}

/// Decision 26: the session's own window id and socket, exactly as `launch::plan` sets
/// them for a PTY window (`anthrex hook` reads both), after the profile's env so the
/// profile cannot point a session's hooks at another window.
fn session_env(
    window_id: u32,
    socket: &std::path::Path,
    profile: &[(String, String)],
) -> Vec<(String, String)> {
    let mut env = profile.to_vec();
    env.push(("ANTHREX_WINDOW_ID".into(), window_id.to_string()));
    env.push(("ANTHREX_SOCKET".into(), socket.display().to_string()));
    env
}

fn exit_reason(code: Option<i32>, signal: Option<i32>) -> String {
    match (signal, code) {
        (Some(signal), _) => format!("killed by signal {signal}"),
        (None, Some(code)) => format!("exited with code {code}"),
        (None, None) => "exited".to_string(),
    }
}

/// Decision 49's answer to a client's `Subscribe` for a headless window.
pub fn subscribe_refusal(id: u32) -> String {
    format!("window {id} is a headless session; open its conversation with C-b m")
}

/// Decision 49's answer to a client's `Input`, `Kill`, `Remove` or `Restart` for a
/// headless window of `run` (`?` for a session that belongs to no run).
pub fn control_refusal(id: u32, run: Option<&RunRef>) -> String {
    let run = run.map_or("?", |run| run.run_id.as_str());
    format!(
        "window {id} is a headless session of run {run}; only the engine drives it. Use anthrex run cancel to stop it"
    )
}

/// Claude's control requests are numbered per daemon; the id only has to be unique.
static INTERRUPT_REQUESTS: AtomicU64 = AtomicU64::new(1);

impl WindowManager {
    /// The engine's feed of every headless window's session events and sub-agent hooks.
    pub fn signals(&self) -> broadcast::Receiver<WindowSignal> {
        self.signals.subscribe()
    }

    /// The spec a headless window was launched with; `None` for a PTY window.
    pub fn headless_spec(&self, id: u32) -> Option<HeadlessSpec> {
        let inner = crate::lock(&self.inner);
        match &inner.entries.get(&id)?.process {
            Process::Headless(window) => Some(window.spec.clone()),
            _ => None,
        }
    }

    /// `Some(run)` when `id` is a headless window (decision 49's refusals name the run).
    pub fn headless_run(&self, id: u32) -> Option<Option<RunRef>> {
        let inner = crate::lock(&self.inner);
        match &inner.entries.get(&id)?.process {
            Process::Headless(window) => Some(window.spec.run_ref.clone()),
            _ => None,
        }
    }

    /// Registers a headless window and starts its session (decisions 24 to 26 and 49).
    /// The first turn is `first_turn`: on Claude's stdin as a stream-json user message,
    /// or as Codex's last argument. Only the engine calls this; no client message can.
    ///
    /// Waits for the launch gate first, outside the lock, as `create` does. The window is
    /// listed from before the spawn, so no event of the new process can arrive for a
    /// window that does not exist yet; a spawn that fails removes it again. The turn is
    /// recorded in the window's cursor before its message is written (M8a.7's ordering
    /// requirement).
    pub async fn create_headless(
        self: &Arc<Self>,
        name: String,
        spec: HeadlessSpec,
        session: SessionArg,
        first_turn: String,
        project: PathBuf,
        worktree: PathBuf,
    ) -> anyhow::Result<WindowInfo> {
        let runtime = spec.runtime;
        anyhow::ensure!(
            matches!(runtime, Runtime::Claude | Runtime::Codex),
            "a headless session runs claude or codex, not {}",
            runtime.label()
        );
        let name = validate_name(&name)?;
        self.config.launch_gate.wait().await;

        let id = self.admit_headless(name, &spec, &first_turn, project, worktree)?;

        let config = self.config.clone();
        let (program, args) = match runtime {
            Runtime::Claude => (
                config.claude_bin.clone(),
                claude_args(
                    &spec,
                    &session,
                    &config.exe,
                    id,
                    &config.socket_path,
                    &CLI_CAPS,
                ),
            ),
            _ => (
                config.codex_bin.clone(),
                codex_args(
                    &spec,
                    &session,
                    &first_turn,
                    &config.exe,
                    id,
                    &config.socket_path,
                    &CLI_CAPS,
                ),
            ),
        };
        let env = session_env(id, &config.socket_path, &spec.env);
        let cwd = spec.cwd.clone();
        let weak = Arc::downgrade(self);
        let spawned = tokio::task::spawn_blocking(move || {
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
        .map_err(|error| anyhow::anyhow!("headless session start failed: {error}"))
        .and_then(|spawned| spawned);
        let handle = match spawned {
            Ok(handle) => handle,
            Err(error) => {
                self.forget(id);
                return Err(error);
            }
        };
        if runtime == Runtime::Claude {
            let session_id = match &session {
                SessionArg::New { uuid } => uuid.clone(),
                SessionArg::Resume { session_id } => Some(session_id.clone()),
            };
            if let Err(error) = handle.send_line(user_message(&first_turn, session_id.as_deref())) {
                handle.kill(config.kill_grace);
                self.forget(id);
                return Err(error.context("could not write the first turn"));
            }
        }

        let mut inner = crate::lock(&self.inner);
        let installed = !inner.shutting_down
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
            handle.kill(config.kill_grace);
            anyhow::bail!("daemon is shutting down");
        }
        let info = inner.entries[&id].info(Instant::now());
        self.publish(&inner);
        drop(inner);
        tracing::info!(id, name = %info.name, runtime = %runtime, "headless window created");
        Ok(info)
    }

    /// Phase A of `create_headless`, under the lock and with no I/O: spend an id, list
    /// the window as `Starting` with no process, and record the first turn.
    fn admit_headless(
        &self,
        name: String,
        spec: &HeadlessSpec,
        first_turn: &str,
        project: PathBuf,
        worktree: PathBuf,
    ) -> anyhow::Result<u32> {
        let mut inner = crate::lock(&self.inner);
        anyhow::ensure!(!inner.shutting_down, "daemon is shutting down");
        let id = inner.next_id;
        let next_id = id.checked_add(1).ok_or_else(|| {
            anyhow::anyhow!(
                "no window ids remain; remove a window holding a high id to free one up"
            )
        })?;
        if inner.entries.values().any(|e| e.name == name) || inner.reserved_names.contains(&name) {
            anyhow::bail!("a window named '{name}' already exists");
        }
        inner.next_id = next_id;
        let now = Instant::now();
        let runtime = spec.runtime;
        let mut window = HeadlessWindow::new(spec.clone());
        let input =
            conversation::sent_turn(runtime, hooks_fire(runtime), first_turn, &mut window.cursor);
        let entry = Entry {
            id,
            name: name.clone(),
            spec: WindowSpec {
                name: Some(name),
                runtime,
                cwd: spec.cwd.clone(),
                worktree_branch: None,
                model: (!spec.model.is_empty()).then(|| spec.model.clone()),
                initial_prompt: None,
            },
            project: Some(project),
            worktree: Some(worktree),
            managed: None,
            removing: false,
            restarting: false,
            status: Status::Starting,
            state: AgentState::default(),
            viewers: 0,
            since: now,
            last_output: now,
            created_at: std::time::SystemTime::now(),
            exit: None,
            child_alive: false,
            process: Process::Headless(Box::new(window)),
            run: serde_json::to_value(spec).ok(),
            conversations: crate::conversation::ConversationSet::new(id, runtime),
            conversation_viewers: 0,
            transcript: Default::default(),
        };
        inner.entries.insert(id, entry);
        let entry = inner.entries.get_mut(&id).expect("inserted above");
        self.apply_input(id, entry, input, now);
        self.publish(&inner);
        Ok(id)
    }

    /// Unlists a headless window whose session never started.
    fn forget(&self, id: u32) {
        let mut inner = crate::lock(&self.inner);
        if inner.entries.remove(&id).is_some() {
            self.notify_window_gone(id);
            self.publish(&inner);
        }
    }

    /// Applies one session event of process `pid` under the lock (decision 27): the
    /// status, the tool, the session id, the conversation inputs, then the feed send. No
    /// I/O happens here; a broadcast send never blocks.
    ///
    /// Status follows the window's current process only: a late event of a process the
    /// window has already replaced (a killed Claude process before its `--resume`) does
    /// not move it, though it still reaches the feed with its own pid.
    pub fn apply_session_event(&self, id: u32, pid: u32, event: &SessionEvent) {
        let now = Instant::now();
        let mut inner = crate::lock(&self.inner);
        let Some(entry) = inner.entries.get_mut(&id) else {
            return;
        };
        let Process::Headless(window) = &mut entry.process else {
            return;
        };
        let runtime = window.spec.runtime;
        if let SessionEvent::Unknown { line }
        | SessionEvent::StderrLine { line }
        | SessionEvent::Diagnostic { text: line } = event
        {
            if window.diagnostics.len() >= DIAGNOSTIC_LINES {
                window.diagnostics.pop_front();
            }
            window.diagnostics.push_back(line.clone());
        }
        let current = window.handle.spawned_pid().is_none_or(|p| p == pid);
        let mut changed = false;
        if current {
            let mut next = status::next(&window.status, event);
            // Codex runs one process per turn: an exit after its turn ended is not the
            // session's end, so the window keeps the turn's `Idle` or `Attention`.
            if runtime == Runtime::Codex
                && matches!(event, SessionEvent::ProcessExited { .. })
                && !window.status.turn_open
            {
                next.status = window.status.status;
            }
            window.status = next;
            if entry.status != window.status.status {
                entry.status = window.status.status;
                entry.since = now;
                changed = true;
            }
            if entry.state.tool != window.status.tool {
                entry.state.tool.clone_from(&window.status.tool);
                changed = true;
            }
        }
        let input = conversation::map(runtime, hooks_fire(runtime), event, &mut window.cursor);
        match event {
            SessionEvent::Init { session_id, .. }
                if current && entry.state.session_id.as_ref() != Some(session_id) =>
            {
                entry.state.session_id = Some(session_id.clone());
                changed = true;
            }
            SessionEvent::ProcessStarted { .. } if current => entry.child_alive = true,
            SessionEvent::ProcessExited { code, signal } if current => {
                entry.child_alive = false;
                if entry.status == Status::Exited {
                    entry.exit = Some(ExitInfo {
                        code: *code,
                        reason: exit_reason(*code, *signal),
                    });
                }
                changed |= entry.state.subagents.child_exited(now);
            }
            _ => {}
        }
        entry.last_output = now;
        self.apply_input(id, entry, input, now);
        let _ = self.signals.send(WindowSignal {
            window_id: id,
            pid: Some(pid),
            kind: WindowSignalKind::Session(event.clone()),
        });
        if changed {
            self.publish(&inner);
        }
    }

    /// `handle_hook`'s branch for a headless window, under the lock (decision 27): a real
    /// hook updates the conversation and the sub-agent rows, never the status. Its prompt
    /// goes to the window's cursor right after the conversation has it (ruling T7-N1), and
    /// a `SubagentStart` or `SubagentStop` reaches the feed. Returns whether the listed
    /// window changed.
    pub(super) fn headless_hook(
        &self,
        id: u32,
        entry: &mut Entry,
        hook: &ParsedHook,
        now: Instant,
    ) -> bool {
        let runtime = entry.spec.runtime;
        let outcome = entry.state.on_hook(runtime, hook, now);
        self.conversation_hook(id, entry, hook, now);
        if hooks_fire(runtime)
            && let Process::Headless(window) = &mut entry.process
        {
            let input = conversation::observe_hook(runtime, hook, &mut window.cursor);
            self.apply_input(id, entry, input, now);
        }
        if matches!(hook.kind, HookKind::SubagentStart | HookKind::SubagentStop) {
            let _ = self.signals.send(WindowSignal {
                window_id: id,
                pid: None,
                kind: WindowSignalKind::Hook {
                    kind: hook.kind,
                    agent_id: hook.agent_id.clone(),
                },
            });
        }
        outcome.changed
    }

    /// Milestone 6.5's two inputs, in order: hooks through `conversation_hook`, records
    /// through `enrich`, then the notification.
    fn apply_input(&self, id: u32, entry: &mut Entry, input: ConversationInput, now: Instant) {
        for hook in &input.hooks {
            self.conversation_hook(id, entry, hook, now);
        }
        if !input.records.is_empty() {
            let changed = entry.conversations.enrich(&input.records, self.caps());
            self.notify_conversations(id, &entry.conversations, &changed);
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

    /// One new turn with `text` (decision 29). Claude's is one stream-json line on the
    /// running process's stdin, recorded in the cursor first. Codex's `exec resume` is
    /// M8a.18's.
    pub async fn headless_send(&self, id: u32, text: &str) -> anyhow::Result<()> {
        let (handle, line) = {
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
            anyhow::ensure!(
                runtime == Runtime::Claude,
                "a Codex turn is a new exec resume process (M8a.18)"
            );
            anyhow::ensure!(
                !window.handle.is_ended(),
                "session for window {id} has ended; resume it"
            );
            let handle = window.handle.clone();
            let input =
                conversation::sent_turn(runtime, hooks_fire(runtime), text, &mut window.cursor);
            self.apply_input(id, entry, input, Instant::now());
            (handle, user_message(text, session_id.as_deref()))
        };
        handle.send_line(line)
    }

    /// Interrupts the running turn: Claude by `CLI_CAPS.claude_interrupt`, Codex by
    /// `SIGINT`.
    pub fn headless_interrupt(&self, id: u32) -> anyhow::Result<()> {
        let (handle, runtime) = self.headless_handle(id)?;
        let mode = match runtime {
            Runtime::Claude => CLI_CAPS.claude_interrupt,
            _ => InterruptMode::Sigint,
        };
        handle.interrupt(mode, INTERRUPT_REQUESTS.fetch_add(1, Ordering::Relaxed))
    }

    /// Resumes an ended session with a message (decision 28): M8a.18's.
    pub async fn headless_resume(
        self: &Arc<Self>,
        id: u32,
        session_id: &str,
        message: &str,
    ) -> anyhow::Result<()> {
        let _ = (session_id, message);
        self.headless_handle(id)?;
        anyhow::bail!("resuming a headless session is not implemented yet (M8a.18)")
    }

    /// `SIGTERM` to the session's group, `SIGKILL` after the kill grace (decision 52).
    pub fn headless_kill(&self, id: u32) -> anyhow::Result<()> {
        let (handle, _) = self.headless_handle(id)?;
        handle.kill(self.config.kill_grace);
        Ok(())
    }
}
