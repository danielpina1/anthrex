//! `ScoutService` (milestone 8b decisions 12 to 14): starts scouts as read-only headless
//! windows, feeds their session events to the pure lifecycle machine, executes its
//! effects, and takes each scout's one `submit_scout_report`.
//!
//! **Locks.** The scout table is taken with `crate::lock` only to read or change it,
//! and every guard is dropped at the end of its block, before any manager call, any
//! `.await` and any file write: the effects a step returns are collected under the lock
//! and executed after it (AGENTS.md rule 2). Reports are written on `spawn_blocking`.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proto::{Route, RunReply, Runtime, ScoutInfo, ScoutReport, ScoutState, ToolCall};
use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;

use super::machine::{self, ScoutEffect, ScoutEvent, ScoutLimits, ScoutMachine};
use super::report::{self, build_report, report_path, state_label};
use super::spec::{self, ScoutContext, ScoutSpec};
use crate::headless::{SessionArg, SessionEvent};
use crate::manager::{WindowManager, WindowSignal, WindowSignalKind};
use crate::run::driver::unix_now;
use crate::run::journal::runs_dir;

/// The tool's answer to an accepted report.
pub const REPORT_ACCEPTED: &str = "Report recorded. You are done; end your turn now.";

/// How a scout ended. Sent once per scout, so the Interfaces' unboxed shape is kept.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoutOutcome {
    Report(ScoutReport),
    Failed { reason: String },
}

/// A started scout: its window, and where its outcome arrives.
#[derive(Debug)]
pub struct ScoutHandle {
    pub id: String,
    pub window_id: u32,
    pub outcome: oneshot::Receiver<ScoutOutcome>,
}

/// One scout the service knows.
struct Scout {
    spec: ScoutSpec,
    route: Route,
    window_id: Option<u32>,
    machine: ScoutMachine,
    outcome: Option<oneshot::Sender<ScoutOutcome>>,
    /// A report is being validated or written: a second one is refused meanwhile.
    storing: bool,
    report: Option<ScoutReport>,
    report_bytes: Option<u32>,
    ended_at: Option<u64>,
    /// Codex runs a process per turn: an exit after a turn ended in that process is not
    /// the session's end.
    turn_ended_pids: HashSet<u32>,
    kill_at: Option<Instant>,
    remove_at: Option<Instant>,
}

#[derive(Default)]
struct Table {
    scouts: HashMap<String, Scout>,
    by_window: HashMap<u32, String>,
    /// Headless windows that are not scouts, so their spec is looked up once.
    foreign: HashSet<u32>,
}

/// What one machine step left to do outside the lock.
struct Pending {
    window: Option<u32>,
    effects: Vec<ScoutEffect>,
    outcome: Option<(oneshot::Sender<ScoutOutcome>, ScoutOutcome)>,
}

/// See the module doc.
pub struct ScoutService {
    manager: Arc<WindowManager>,
    ctx: ScoutContext,
    table: Mutex<Table>,
    signals: Mutex<Option<broadcast::Receiver<WindowSignal>>>,
}

impl ScoutService {
    /// Subscribes to the manager's session feed at once, so no event of a scout started
    /// before [`ScoutService::spawn`] is lost.
    pub fn new(manager: Arc<WindowManager>, ctx: ScoutContext) -> Arc<Self> {
        let signals = manager.signals();
        Arc::new(ScoutService {
            manager,
            ctx,
            table: Mutex::new(Table::default()),
            signals: Mutex::new(Some(signals)),
        })
    }

    /// The signal listener and the 1-second ticker (timeouts, kills and removals due).
    pub fn spawn(self: &Arc<Self>, shutdown: CancellationToken) -> tokio::task::JoinHandle<()> {
        let signals = crate::lock(&self.signals).take();
        let service = self.clone();
        tokio::spawn(async move {
            let Some(mut signals) = signals else { return };
            let mut tick = tokio::time::interval(Duration::from_secs(1));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    _ = tick.tick() => service.tick(),
                    signal = signals.recv() => match signal {
                        Ok(signal) => service.on_signal(signal),
                        Err(broadcast::error::RecvError::Lagged(n)) => {
                            tracing::warn!(n, "the scout service lagged behind the session feed");
                        }
                        Err(broadcast::error::RecvError::Closed) => return,
                    },
                }
            }
        })
    }

    fn limits(&self) -> ScoutLimits {
        ScoutLimits {
            timeout_secs: self.ctx.scouts.timeout_secs,
            max_tool_calls: self.ctx.scouts.max_tool_calls,
        }
    }

    /// Starts scout `spec` as window `scout/<id>` (decision 12).
    pub async fn start(self: &Arc<Self>, spec: ScoutSpec) -> anyhow::Result<ScoutHandle> {
        anyhow::ensure!(spec::valid_id(&spec.id), "invalid scout id {:?}", spec.id);
        let headless = spec::headless_spec(&spec, &self.ctx);
        let route = spec::scout_route(&self.ctx);
        let id = spec.id.clone();
        let (sender, outcome) = oneshot::channel();
        {
            let mut table = crate::lock(&self.table);
            anyhow::ensure!(
                !table.scouts.contains_key(&id),
                "a scout {id} already exists"
            );
            table.scouts.insert(
                id.clone(),
                Scout {
                    spec: spec.clone(),
                    route: route.clone(),
                    window_id: None,
                    machine: ScoutMachine::default(),
                    outcome: Some(sender),
                    storing: false,
                    report: None,
                    report_bytes: None,
                    ended_at: None,
                    turn_ended_pids: HashSet::new(),
                    kill_at: None,
                    remove_at: None,
                },
            );
        }
        self.drive(&id, ScoutEvent::Start { now: unix_now() });
        let uuid = (route.runtime == Runtime::Claude).then(|| {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64);
            crate::run::role_launch::session_uuid(&format!("scout/{id}"), nanos)
        });
        let created = self
            .manager
            .create_headless(
                format!("scout/{id}"),
                headless,
                SessionArg::New { uuid },
                spec.first_turn.clone(),
                spec.project.clone(),
                spec.cwd.clone(),
            )
            .await;
        let info = match created {
            Ok(info) => info,
            Err(error) => {
                crate::lock(&self.table).scouts.remove(&id);
                return Err(error);
            }
        };
        {
            let mut table = crate::lock(&self.table);
            table.by_window.insert(info.id, id.clone());
            if let Some(scout) = table.scouts.get_mut(&id) {
                scout.window_id = Some(info.id);
            }
        }
        Ok(ScoutHandle {
            id,
            window_id: info.id,
            outcome,
        })
    }

    /// The scout window `window_id` belongs to, if it is one of ours. A window not yet
    /// bound (its first events can arrive before `create_headless` returns) is found by
    /// its spec's scout id.
    fn scout_of(&self, window_id: u32) -> Option<String> {
        {
            let table = crate::lock(&self.table);
            if let Some(id) = table.by_window.get(&window_id) {
                return Some(id.clone());
            }
            if table.foreign.contains(&window_id) {
                return None;
            }
        }
        let scout_id = self
            .manager
            .headless_spec(window_id)
            .and_then(|spec| spec.mcp)
            .filter(|target| target.role == proto::AgentRole::Scout)
            .and_then(|target| target.scout_id);
        let mut table = crate::lock(&self.table);
        let unbound = scout_id
            .as_ref()
            .and_then(|id| table.scouts.get(id))
            .is_some_and(|scout| scout.window_id.is_none());
        match scout_id {
            Some(id) if unbound => {
                table.by_window.insert(window_id, id.clone());
                if let Some(scout) = table.scouts.get_mut(&id) {
                    scout.window_id = Some(window_id);
                }
                Some(id)
            }
            _ => {
                if table.foreign.len() >= 4096 {
                    table.foreign.clear();
                }
                table.foreign.insert(window_id);
                None
            }
        }
    }

    /// One session event of one of our windows, as the machine's event.
    fn on_signal(&self, signal: WindowSignal) {
        let WindowSignalKind::Session(event) = signal.kind else {
            return;
        };
        if !matches!(
            event,
            SessionEvent::TurnEnded { .. }
                | SessionEvent::ToolUse { .. }
                | SessionEvent::ProcessExited { .. }
        ) {
            return;
        }
        let Some(id) = self.scout_of(signal.window_id) else {
            return;
        };
        let scout_event = {
            let mut table = crate::lock(&self.table);
            let Some(scout) = table.scouts.get_mut(&id) else {
                return;
            };
            match event {
                SessionEvent::TurnEnded { usage, .. } => {
                    scout.turn_ended_pids.extend(signal.pid);
                    ScoutEvent::TurnEnded { usage }
                }
                SessionEvent::ToolUse { .. } => ScoutEvent::ToolUse,
                SessionEvent::ProcessExited { code, .. } => {
                    let turn_over = signal
                        .pid
                        .is_some_and(|pid| scout.turn_ended_pids.contains(&pid));
                    if scout.route.runtime == Runtime::Codex && turn_over {
                        return;
                    }
                    ScoutEvent::Exited { code }
                }
                _ => return,
            }
        };
        self.drive(&id, scout_event);
    }

    /// Steps scout `id`'s machine with `event` and executes what it asks.
    fn drive(&self, id: &str, event: ScoutEvent) {
        let pending = {
            let mut table = crate::lock(&self.table);
            match table.scouts.get_mut(id) {
                Some(scout) => step_locked(scout, event, &self.limits()),
                None => return,
            }
        };
        self.execute(pending);
    }

    /// Executes a step's effects, with no lock held.
    fn execute(&self, pending: Pending) {
        if let Some((sender, outcome)) = pending.outcome {
            let _ = sender.send(outcome);
        }
        let Some(window) = pending.window else {
            return;
        };
        for effect in pending.effects {
            let done = match effect {
                ScoutEffect::Send(text) => {
                    let manager = self.manager.clone();
                    tokio::spawn(async move {
                        if let Err(error) = manager.headless_send(window, &text).await {
                            tracing::debug!(window, %error, "scout send");
                        }
                    });
                    Ok(())
                }
                ScoutEffect::Kill => self.manager.headless_kill(window),
                ScoutEffect::CloseStdin => self.manager.headless_retire(window),
                // Recorded as deadlines by `step_locked`; the ticker acts on them.
                ScoutEffect::KillAfter(_)
                | ScoutEffect::RemoveAfter(_)
                | ScoutEffect::Finished(_) => Ok(()),
            };
            if let Err(error) = done {
                tracing::debug!(window, %error, "scout effect");
            }
        }
    }

    /// Every second: the timeout of each working scout, then the kills and removals due.
    fn tick(&self) {
        let now = unix_now();
        let working: Vec<String> = {
            let table = crate::lock(&self.table);
            table
                .scouts
                .iter()
                .filter(|(_, s)| s.machine.state == ScoutState::Working)
                .map(|(id, _)| id.clone())
                .collect()
        };
        for id in working {
            self.drive(&id, ScoutEvent::Tick { now });
        }
        let at = Instant::now();
        let (kill, remove) = {
            let mut table = crate::lock(&self.table);
            let (mut kill, mut remove) = (Vec::new(), Vec::new());
            for scout in table.scouts.values_mut() {
                let Some(window) = scout.window_id else {
                    continue;
                };
                if scout.kill_at.is_some_and(|t| t <= at) {
                    scout.kill_at = None;
                    kill.push(window);
                }
                if scout.remove_at.is_some_and(|t| t <= at) {
                    scout.remove_at = None;
                    scout.kill_at = None;
                    remove.push(window);
                }
            }
            for window in &remove {
                table.by_window.remove(window);
            }
            (kill, remove)
        };
        for window in kill {
            // Decision 52, as the run driver retires a reviewer: only a session still
            // running at its deadline is killed.
            if self.manager.child_pid(window).ok().flatten().is_some()
                && let Err(error) = self.manager.headless_kill(window)
            {
                tracing::debug!(window, %error, "scout kill");
            }
        }
        for window in remove {
            if let Err(error) = self.manager.remove(window) {
                tracing::debug!(window, %error, "scout window removal");
            }
        }
    }

    /// `submit_scout_report` (decision 13), answered as `RunReply::ToolResult`.
    pub async fn tool(&self, call: ToolCall) -> RunReply {
        let (ok, text) = match self.submit(call).await {
            Ok(text) => (true, text),
            Err(text) => (false, text),
        };
        RunReply::ToolResult { ok, text }
    }

    async fn submit(&self, call: ToolCall) -> Result<String, String> {
        let id = call.scout_id.clone().unwrap_or_default();
        let draft = {
            let mut table = crate::lock(&self.table);
            let scout = table
                .scouts
                .get_mut(&id)
                .ok_or_else(|| format!("unknown scout {id}"))?;
            if scout.window_id != Some(call.window_id) {
                return Err(format!("this window is not scout {id}"));
            }
            if scout.report.is_some() || scout.storing {
                return Err(format!("a report for scout {id} was already recorded"));
            }
            if matches!(
                scout.machine.state,
                ScoutState::Failed | ScoutState::Reported
            ) {
                return Err(format!(
                    "scout {id} is {}",
                    state_label(scout.machine.state)
                ));
            }
            scout.storing = true;
            (
                scout.spec.clone(),
                scout.route.clone(),
                scout.machine.clone(),
            )
        };
        let (spec, route, machine) = draft;
        let args = match report::validate(&call.args, spec.kind) {
            Ok(args) => args,
            Err(problem) => {
                self.unclaim(&id);
                return Err(problem);
            }
        };
        let report = build_report(&spec, route, call.window_id, &machine, args, unix_now());
        let path = self.report_path(&spec);
        let bytes = serde_json::to_vec_pretty(&report).map_err(|e| e.to_string())?;
        let size = u32::try_from(bytes.len()).unwrap_or(u32::MAX);
        let file = path.clone();
        let written =
            tokio::task::spawn_blocking(move || crate::profile::store::write_atomic(&file, &bytes))
                .await;
        if let Err(error) = written
            .map_err(|e| e.to_string())
            .and_then(|r| r.map_err(|e| e.to_string()))
        {
            self.unclaim(&id);
            return Err(format!("could not store the report: {error}"));
        }
        let pending = {
            let mut table = crate::lock(&self.table);
            let Some(scout) = table.scouts.get_mut(&id) else {
                return Err(format!("unknown scout {id}"));
            };
            scout.storing = false;
            if scout.machine.state == ScoutState::Working {
                scout.report = Some(report);
                scout.report_bytes = Some(size);
                Ok(step_locked(
                    scout,
                    ScoutEvent::ReportAccepted,
                    &self.limits(),
                ))
            } else {
                Err(format!(
                    "scout {id} is {}",
                    state_label(scout.machine.state)
                ))
            }
        };
        match pending {
            Ok(pending) => {
                self.execute(pending);
                Ok(REPORT_ACCEPTED.to_string())
            }
            Err(refusal) => {
                // The scout failed while its report was written: the file goes too.
                let _ = tokio::task::spawn_blocking(move || std::fs::remove_file(path)).await;
                Err(refusal)
            }
        }
    }

    fn unclaim(&self, id: &str) {
        if let Some(scout) = crate::lock(&self.table).scouts.get_mut(id) {
            scout.storing = false;
        }
    }

    /// Where `spec`'s report is stored (decision 13).
    pub fn report_path(&self, spec: &ScoutSpec) -> std::path::PathBuf {
        let repo_dir = crate::profile::repo_dir(&self.ctx.data_dir, &spec.project);
        let run_dir = spec
            .run_id
            .as_ref()
            .map(|run| runs_dir(&self.ctx.data_dir).join(run));
        report_path(&repo_dir, run_dir.as_deref(), &spec.id)
    }

    /// Kills scout `id` and fails it with `stopped by the user`.
    pub fn stop(&self, id: &str) {
        self.drive(id, ScoutEvent::Stop);
    }

    pub fn info(&self, id: &str) -> Option<ScoutInfo> {
        let table = crate::lock(&self.table);
        table.scouts.get(id).map(info)
    }

    /// The scouts of run `run_id`, for its snapshot (none until milestone 9).
    pub fn run_scouts(&self, run_id: &str) -> Vec<ScoutInfo> {
        let table = crate::lock(&self.table);
        let mut scouts: Vec<ScoutInfo> = table
            .scouts
            .values()
            .filter(|s| s.spec.run_id.as_deref() == Some(run_id))
            .map(info)
            .collect();
        scouts.sort_by(|a, b| (a.started_at, &a.id).cmp(&(b.started_at, &b.id)));
        scouts
    }
}

/// One machine step under the table lock: the new machine, the deadlines, the outcome,
/// and the effects left to execute.
fn step_locked(scout: &mut Scout, event: ScoutEvent, limits: &ScoutLimits) -> Pending {
    let (next, effects) = machine::step(scout.machine.clone(), event, limits);
    scout.machine = next;
    let mut outcome = None;
    let now = Instant::now();
    for effect in &effects {
        match effect {
            ScoutEffect::KillAfter(after) => scout.kill_at = Some(now + *after),
            ScoutEffect::RemoveAfter(after) => scout.remove_at = Some(now + *after),
            ScoutEffect::Finished(result) => {
                scout.ended_at = Some(unix_now());
                let ended = match (result, &scout.report) {
                    (Ok(()), Some(report)) => ScoutOutcome::Report(report.clone()),
                    (Err(reason), _) => ScoutOutcome::Failed {
                        reason: reason.clone(),
                    },
                    (Ok(()), None) => ScoutOutcome::Failed {
                        reason: "the scout reported nothing".into(),
                    },
                };
                outcome = scout.outcome.take().map(|sender| (sender, ended));
            }
            _ => {}
        }
    }
    Pending {
        window: scout.window_id,
        effects,
        outcome,
    }
}

fn info(scout: &Scout) -> ScoutInfo {
    ScoutInfo {
        id: scout.spec.id.clone(),
        kind: scout.spec.kind,
        question: scout.spec.question.clone(),
        state: scout.machine.state,
        failure: scout.machine.failure.clone(),
        window_id: scout.window_id,
        route: scout.route.clone(),
        started_at: scout.machine.started_at,
        ended_at: scout.ended_at,
        tool_calls: scout.machine.tool_calls,
        report_bytes: scout.report_bytes,
        files: scout
            .report
            .as_ref()
            .map(|r| r.files.iter().map(|f| f.path.clone()).collect())
            .unwrap_or_default(),
        usage: scout.machine.usage,
    }
}
