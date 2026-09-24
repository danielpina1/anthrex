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
//! (AGENTS.md rule 2): every method clones the window's [`HeadlessHandle`] out from under
//! the lock and acts on the clone. Starting a session's processes, and its turns,
//! interrupts and resumes (M8a.18), are in `headless_turns.rs`.

use super::entry::{Entry, Process};
use super::{WindowManager, validate_name};
use crate::agent_state::AgentState;
use crate::headless::argv::CLI_CAPS;
use crate::headless::claude_stream::user_message;
use crate::headless::conversation::{self, ConversationInput, StreamCursor};
use crate::headless::session::HeadlessHandle;
use crate::headless::status::{self, HeadlessStatus};
use crate::headless::{HeadlessSpec, SessionArg, SessionEvent, TurnOutcome};
use crate::hooks::{HookKind, ParsedHook};
use proto::{ExitInfo, RunRef, Runtime, Status, WindowInfo, WindowSpec};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{broadcast, oneshot, watch};

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
    /// A session event that belongs to a turn Claude Code started by itself (a background
    /// sub-agent's notification) while a turn the engine delivered still waits to run:
    /// today only that turn's `TurnEnded`, as `headless::conversation` classified it by
    /// its prompt's text (ruling T7-N1). It is not the delivered turn's end: it must not
    /// close that turn, count toward its tool calls, or trigger the next delivery. Its
    /// usage is still the session's spend.
    Unprompted(SessionEvent),
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
    /// A turn ended in the current process (ruling T17-I1): only then is a Codex
    /// process's exit the normal end of its turn rather than the session's death.
    pub(super) turn_ended_in_process: bool,
    /// A send or resume is starting a process for this window (M8a.18): a second one is
    /// refused until it is done.
    pub(super) busy: bool,
    /// A resume waiting for its process's `Init` (`Ok`), or for its exit before one
    /// (`Err` with the reason): decision 28's failed-resume marker.
    pub(super) start_waiter: Option<oneshot::Sender<Result<(), String>>>,
    /// What the resuming process said went wrong before its `Init`: a failed `result`'s
    /// text, else its first stderr line.
    pub(super) start_failure: Option<String>,
    /// While `busy`: where a kill or interrupt that arrives before the new process is
    /// installed is recorded (ruling T18-I1). The send or resume stops at its next step
    /// and never installs a new process.
    pub(super) cancel: Option<watch::Sender<Option<Cancel>>>,
}

/// A kill or interrupt that reached a window between its old process and its new one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Cancel {
    Killed,
    Interrupted,
}

impl HeadlessWindow {
    pub(super) fn new(spec: HeadlessSpec) -> Self {
        HeadlessWindow {
            handle: HeadlessHandle::ended(),
            status: HeadlessStatus::default(),
            // A Claude window's real prompt hooks are fed to its cursor (ruling T7-N1),
            // so it is content-based from its first turn (M8a.7 re-review 2, m1).
            cursor: if hooks_fire(spec.runtime) {
                StreamCursor::fed()
            } else {
                StreamCursor::default()
            },
            spec,
            diagnostics: VecDeque::new(),
            turn_ended_in_process: false,
            busy: false,
            start_waiter: None,
            start_failure: None,
            cancel: None,
        }
    }

    /// Marks the window busy with a send or resume, and returns where a kill or
    /// interrupt meanwhile will be recorded.
    pub(super) fn claim(&mut self) -> watch::Receiver<Option<Cancel>> {
        self.busy = true;
        let (sender, receiver) = watch::channel(None);
        self.cancel = Some(sender);
        receiver
    }

    /// The kill or interrupt recorded during the current send or resume, if any.
    pub(super) fn cancelled(&self) -> Option<Cancel> {
        self.cancel.as_ref().and_then(|sender| *sender.borrow())
    }

    /// Decision 28's failed-resume marker, for a resume waiting on this process: its
    /// `Init` means it started; an exit before one means the resume failed, with the
    /// failed `result`'s text or else its first stderr line as the reason.
    fn observe_start(&mut self, event: &SessionEvent) {
        if self.start_waiter.is_none() {
            return;
        }
        match event {
            SessionEvent::Init { .. } => {
                self.start_failure = None;
                if let Some(waiter) = self.start_waiter.take() {
                    let _ = waiter.send(Ok(()));
                }
            }
            SessionEvent::TurnEnded {
                outcome: TurnOutcome::Failed { error, .. },
                ..
            } => self.start_failure = Some(error.clone()),
            SessionEvent::StderrLine { line } if self.start_failure.is_none() => {
                self.start_failure = Some(line.trim().to_string());
            }
            SessionEvent::ProcessExited { code, signal } => {
                let mut reason = format!(
                    "the process {} before it started",
                    exit_reason(*code, *signal)
                );
                if let Some(failure) = self.start_failure.take() {
                    reason = format!("{reason}: {failure}");
                }
                if let Some(waiter) = self.start_waiter.take() {
                    let _ = waiter.send(Err(reason));
                }
            }
            _ => {}
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
pub(super) fn hooks_fire(runtime: Runtime) -> bool {
    runtime == Runtime::Claude && CLI_CAPS.claude_hooks_fire_in_print
}

pub(super) fn exit_reason(code: Option<i32>, signal: Option<i32>) -> String {
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

        let args = self.session_args(&spec, &session, &first_turn, id);
        let line = (runtime == Runtime::Claude).then(|| {
            let session_id = match &session {
                SessionArg::New { uuid } => uuid.as_deref(),
                SessionArg::Resume { session_id } => Some(session_id.as_str()),
            };
            user_message(&first_turn, session_id)
        });
        let handle = match self.spawn_process(id, &spec, args, line).await {
            Ok(handle) => handle,
            Err(error) => {
                self.forget(id);
                return Err(error);
            }
        };
        let info = self.install(id, &handle)?;
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
        let input = conversation::map(runtime, hooks_fire(runtime), event, &mut window.cursor);
        // A background turn's `result` while a delivered turn waits (ruling T7-N1): the
        // delivered turn is still open, so the window's status stays as it is.
        let unprompted =
            matches!(event, SessionEvent::TurnEnded { .. }) && window.cursor.ended_unprompted();
        if current {
            window.observe_start(event);
        }
        let mut changed = false;
        if current && !unprompted {
            let mut next = status::next(&window.status, event);
            // Codex runs one process per turn: an exit after a turn ended in that
            // process is not the session's end, so the window keeps the turn's `Idle` or
            // `Attention`. An exit before any turn ended in it is a death (T17-I1).
            if runtime == Runtime::Codex
                && matches!(event, SessionEvent::ProcessExited { .. })
                && window.turn_ended_in_process
                && !window.status.turn_open
            {
                next.status = window.status.status;
            }
            match event {
                SessionEvent::ProcessStarted { .. } => window.turn_ended_in_process = false,
                SessionEvent::TurnEnded { .. } => window.turn_ended_in_process = true,
                _ => {}
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
        let kind = if unprompted {
            WindowSignalKind::Unprompted(event.clone())
        } else {
            WindowSignalKind::Session(event.clone())
        };
        let _ = self.signals.send(WindowSignal {
            window_id: id,
            pid: Some(pid),
            kind,
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
    pub(super) fn apply_input(
        &self,
        id: u32,
        entry: &mut Entry,
        input: ConversationInput,
        now: Instant,
    ) {
        for hook in &input.hooks {
            self.conversation_hook(id, entry, hook, now);
        }
        if !input.records.is_empty() {
            let changed = entry.conversations.enrich(&input.records, self.caps());
            self.notify_conversations(id, &entry.conversations, &changed);
        }
    }
}
