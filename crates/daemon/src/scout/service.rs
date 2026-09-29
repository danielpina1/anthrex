//! `ScoutService` (milestone 8b decisions 12 to 14): starts scouts as read-only headless
//! windows, feeds their session events to the pure lifecycle machine, executes its
//! effects, and takes each scout's one `submit_scout_report`.
//!
//! **Locks.** The scout table is taken with `crate::lock` only to read or change it,
//! and every guard is dropped at the end of its block, before any manager call, any
//! `.await` and any file write: the effects a step returns are collected under the lock
//! and executed after it (AGENTS.md rule 2). Reports are written on `spawn_blocking`.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use proto::{Route, RunReply, Runtime, ScoutInfo, ScoutReport, ScoutState, ToolCall};
use tokio::sync::{broadcast, oneshot};
use tokio_util::sync::CancellationToken;

use super::machine::{self, ScoutEffect, ScoutEvent, ScoutLimits, ScoutMachine};
use super::planner::PlannerTag;
use super::report::{self, build_report, report_path, state_label};
use super::spec::{self, ScoutContext, ScoutSpec};
use crate::headless::{SessionArg, SessionEvent};
use crate::manager::{WindowManager, WindowSignal, WindowSignalKind};
use crate::run::driver::unix_now;
use crate::run::journal::runs_dir;

/// The tool's answer to an accepted report.
/// The scout's one tool (`mcp::tools_scout`).
pub const SUBMIT_SCOUT_REPORT: &str = "submit_scout_report";

pub const REPORT_ACCEPTED: &str = "Report recorded. You are done; end your turn now.";

/// How a scout ended. Sent once per scout, so the Interfaces' unboxed shape is kept.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScoutOutcome {
    Report(ScoutReport),
    Failed {
        reason: String,
    },
    /// A sub-planner's epic was accepted by the engine (milestone 9 decision 22).
    Accepted,
}

/// A started scout: its window, and where its outcome arrives.
#[derive(Debug)]
pub struct ScoutHandle {
    pub id: String,
    pub window_id: u32,
    pub outcome: oneshot::Receiver<ScoutOutcome>,
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

    /// What every scout launch reads from the daemon.
    pub fn context(&self) -> &ScoutContext {
        &self.ctx
    }

    /// A scout's limits, or a sub-planner's own (milestone 9 decision 31).
    fn limits(&self, scout: &Scout) -> ScoutLimits {
        match &scout.planner {
            Some(tag) => tag.limits,
            None => ScoutLimits::new(&self.ctx.scouts, scout.route.runtime),
        }
    }

    /// Starts scout `spec` as window `scout/<id>` (decision 12), on the route this
    /// service gives every scout.
    pub async fn start(self: &Arc<Self>, spec: ScoutSpec) -> anyhow::Result<ScoutHandle> {
        let route = spec::scout_route(&self.ctx);
        self.start_on(spec, route).await
    }

    /// [`Self::start`] on `route`: a run scout's (M9.17 fix round 3), which the driver
    /// resolves over the run's installed runtimes.
    pub async fn start_on(
        self: &Arc<Self>,
        spec: ScoutSpec,
        route: Route,
    ) -> anyhow::Result<ScoutHandle> {
        anyhow::ensure!(spec::valid_id(&spec.id), "invalid scout id {:?}", spec.id);
        // Ruling M7: the run id is joined into the report's path. Run ids are decision
        // 15's slugs, whose alphabet and length `valid_id` covers.
        if let Some(run) = &spec.run_id {
            anyhow::ensure!(spec::valid_id(run), "invalid run id {run:?}");
        }
        let headless = spec::headless_spec_on(&spec, &self.ctx, &route);
        let name = format!("scout/{}", spec.id);
        self.launch(spec, route, headless, name, None).await
    }

    /// Records scout (or sub-planner, `planner`) `spec` and starts its session as
    /// headless window `name`.
    pub(super) async fn launch(
        self: &Arc<Self>,
        spec: ScoutSpec,
        route: Route,
        headless: crate::headless::HeadlessSpec,
        name: String,
        planner: Option<PlannerTag>,
    ) -> anyhow::Result<ScoutHandle> {
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
                    kill_on_bind: false,
                    installed: false,
                    kill_at: None,
                    remove_at: None,
                    planner,
                },
            );
        }
        self.drive(&id, ScoutEvent::Start { now: unix_now() });
        let uuid = (route.runtime == Runtime::Claude).then(|| {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos() as u64);
            crate::run::role_launch::session_uuid(&name, nanos)
        });
        let created = self
            .manager
            .create_headless(
                name,
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
        self.bind(&id, info.id, true);
        Ok(ScoutHandle {
            id,
            window_id: info.id,
            outcome,
        })
    }

    /// Binds scout `id` to its window. With `kill_owed`, once `create_headless` has
    /// installed the process, it also does a kill the machine asked for before there
    /// was a window to kill (ruling M4). Idempotent.
    fn bind(&self, id: &str, window_id: u32, kill_owed: bool) {
        let kill = {
            let mut table = crate::lock(&self.table);
            table.by_window.insert(window_id, id.to_string());
            match table.scouts.get_mut(id) {
                Some(scout) => {
                    scout.window_id = Some(window_id);
                    scout.installed |= kill_owed;
                    kill_owed && std::mem::take(&mut scout.kill_on_bind)
                }
                None => false,
            }
        };
        if kill && let Err(error) = self.manager.headless_kill(window_id) {
            tracing::debug!(window_id, %error, "scout kill at bind");
        }
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
        let target = self
            .manager
            .headless_spec(window_id)
            .and_then(|spec| spec.mcp);
        let mut table = crate::lock(&self.table);
        let scout_id = target.and_then(|t| match t.role {
            proto::AgentRole::Scout => t.scout_id,
            // Milestone 9 decision 31: a sub-planner's unbound session of its epic.
            proto::AgentRole::Planner => table
                .scouts
                .iter()
                .find(|(_, s)| {
                    s.window_id.is_none()
                        && s.planner.as_ref().is_some_and(|p| {
                            p.run_id == t.run_id && t.epic.as_ref() == Some(&p.epic)
                        })
                })
                .map(|(id, _)| id.clone()),
            _ => None,
        });
        let unbound = scout_id
            .as_ref()
            .and_then(|id| table.scouts.get(id))
            .is_some_and(|scout| scout.window_id.is_none());
        match scout_id {
            Some(id) if unbound => {
                drop(table);
                self.bind(&id, window_id, false);
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
            let runtime = scout.route.runtime;
            let texts = self.limits(scout).texts;
            let turns = &mut scout.turn_ended_pids;
            match machine::scout_event(&event, signal.pid, runtime, turns, &texts) {
                Some(scout_event) => scout_event,
                None => return,
            }
        };
        self.drive(&id, scout_event);
    }

    /// Steps scout `id`'s machine with `event` and executes what it asks.
    pub(super) fn drive(&self, id: &str, event: ScoutEvent) {
        let pending = {
            let mut table = crate::lock(&self.table);
            match table.scouts.get_mut(id) {
                Some(scout) => {
                    let limits = self.limits(scout);
                    step_locked(scout, event, &limits)
                }
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
    /// A removed window's scout is forgotten (ruling R-T9-2): its report is on disk.
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
            // Ruling R-T9-2: a finished scout leaves the table with its window.
            for window in &remove {
                if let Some(id) = table.by_window.remove(window) {
                    table.scouts.remove(&id);
                }
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
        RunReply::tool_result(ok, text)
    }

    async fn submit(&self, call: ToolCall) -> Result<String, String> {
        let id = call.scout_id.clone().unwrap_or_default();
        let (spec, route, machine) = self.claim(&call)?;
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
        match self.commit(&id, report, size) {
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

    /// The checks before a report is stored, under the lock; marks it in flight.
    fn claim(&self, call: &ToolCall) -> Result<(ScoutSpec, Route, ScoutMachine), String> {
        // Ruling M5: the scout's one tool, whatever a socket client names.
        if call.tool != SUBMIT_SCOUT_REPORT {
            return Err(format!(
                "tool {} is not available to the scout role",
                call.tool
            ));
        }
        let id = call.scout_id.clone().unwrap_or_default();
        let mut table = crate::lock(&self.table);
        let scout = table
            .scouts
            .get_mut(&id)
            .filter(|s| s.planner.is_none())
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
        Ok((
            scout.spec.clone(),
            scout.route.clone(),
            scout.machine.clone(),
        ))
    }

    /// After the report is written, under the lock: accepted only if the scout is still
    /// working (it may have failed meanwhile).
    fn commit(&self, id: &str, report: ScoutReport, size: u32) -> Result<Pending, String> {
        let mut table = crate::lock(&self.table);
        let Some(scout) = table.scouts.get_mut(id) else {
            return Err(format!("unknown scout {id}"));
        };
        scout.storing = false;
        if scout.machine.state != ScoutState::Working {
            return Err(format!(
                "scout {id} is {}",
                state_label(scout.machine.state)
            ));
        }
        scout.report = Some(report);
        scout.report_bytes = Some(size);
        let limits = self.limits(scout);
        Ok(step_locked(scout, ScoutEvent::ReportAccepted, &limits))
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

    /// The engine halted run scout `id` (`run cancel`, the `finish` edit): the machine
    /// kills the session with the engine's reason, as `stop_planner` does.
    pub fn halt(&self, id: &str, reason: &str) {
        let reason = reason.to_string();
        self.drive(id, ScoutEvent::Halt { reason });
    }

    /// The sub-planner session in window `window_id`, if it is one of ours.
    pub(super) fn planner_at(&self, window_id: u32) -> Option<String> {
        let table = crate::lock(&self.table);
        let id = table.by_window.get(&window_id)?;
        table.scouts.get(id)?.planner.as_ref()?;
        Some(id.clone())
    }

    pub fn info(&self, id: &str) -> Option<ScoutInfo> {
        let table = crate::lock(&self.table);
        table.scouts.get(id).map(info)
    }

    /// Runs `f` while holding the scout table's lock: a test's stand-in for a scout
    /// step that holds it (the M9.6 second review's lock-order test).
    #[cfg(test)]
    pub(crate) fn with_table_held<R>(&self, f: impl FnOnce() -> R) -> R {
        let _table = crate::lock(&self.table);
        f()
    }

    /// The scouts of run `run_id`, for its snapshot; its sub-planners are reported
    /// through `RunInfo.planners` (decision 33).
    pub fn run_scouts(&self, run_id: &str) -> Vec<ScoutInfo> {
        let table = crate::lock(&self.table);
        let mut scouts: Vec<ScoutInfo> = table
            .scouts
            .values()
            .filter(|s| s.spec.run_id.as_deref() == Some(run_id) && s.planner.is_none())
            .map(info)
            .collect();
        scouts.sort_by(|a, b| (a.started_at, &a.id).cmp(&(b.started_at, &b.id)));
        scouts
    }
}

#[path = "service_table.rs"]
mod table;
use table::{Pending, Scout, Table, info, step_locked};

#[cfg(test)]
#[path = "tests_service.rs"]
mod tests;
