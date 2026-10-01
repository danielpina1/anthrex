//! Milestone 9's ops, executed (task M9.13): the orchestrator's window
//! (`CreateOrchestrator`, `RestartOrchestrator`; decisions 5, 10, 11 and 14a), run scouts
//! and sub-planners on M8b's scout service (`StartScout`, `StartPlanner`; decisions 20,
//! 31, 32 and 34), and a review task's target (`ResolveTarget`, decision 36). Also
//! decision 11's run-live flag: set when the orchestrator starts, restarts or is
//! restored with a run that has not ended, cleared when a published run has.
//!
//! No lock is held across an await. Git, the OS random source and `otlp.addr` are read
//! on `spawn_blocking`; windows are made through the manager, whose own blocking phase
//! runs on `spawn_blocking` (AGENTS.md rules 2 and 10).

use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use proto::{RunsSnapshot, Runtime, TokenUsage};

use super::ops::{blocking, failed};
use super::{GitBudget, OpCtx, RunService, unix_now};
use crate::launch::role::{RoleLaunch, otlp_env};
use crate::run::engine::{EventKind, OpKind, OpResult, OrchEvent, ScoutEnd};
use crate::run::model::ClaudeAuth;
use crate::scout::planner::PlannerSpec;
use crate::scout::service::{ScoutHandle, ScoutOutcome};
use crate::scout::spec::ScoutSpec;

/// How long a session's end waits for its start op's result to reach the engine first.
const SETTLE_WAIT: Duration = Duration::from_secs(30);
const SETTLE_POLL: Duration = Duration::from_millis(50);

/// Decision 14a: 32 lowercase hex characters from the OS random source (blocking).
pub(super) fn fresh_token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| f.read_exact(&mut bytes))
        .map_err(|error| format!("could not read /dev/urandom: {error}"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// `config::ClaudeAuth` of a run's frozen auth, for the credential scrub.
fn auth(auth: ClaudeAuth) -> config::ClaudeAuth {
    match auth {
        ClaudeAuth::Login => config::ClaudeAuth::Login,
        ClaudeAuth::ApiKey => config::ClaudeAuth::ApiKey,
    }
}

/// Executes one of milestone 9's ops (`driver/ops.rs` dispatches here).
pub(super) async fn run(service: &Arc<RunService>, ctx: &OpCtx, kind: OpKind) -> OpResult {
    match kind {
        OpKind::CreateOrchestrator {
            spec,
            role,
            project,
        } => {
            service
                .create_orchestrator(ctx, *spec, *role, project)
                .await
        }
        OpKind::RestartOrchestrator { window_id } => {
            service.refresh_otlp(ctx, window_id).await;
            match service.manager.restart(window_id).await {
                Ok(()) => {
                    service.mark_live(&ctx.run_id, window_id);
                    OpResult::Restarted
                }
                Err(error) => failed(error.to_string()),
            }
        }
        OpKind::StartScout { spec } => service.start_scout(ctx, *spec).await,
        OpKind::StartPlanner { spec } => service.start_planner(ctx, *spec).await,
        OpKind::ResolveTarget {
            root,
            target,
            base_branch,
        } => {
            let budget = service.ctx.read_git;
            resolve_target(
                service.git(),
                ctx.git_timeout,
                budget,
                root,
                target,
                base_branch,
            )
            .await
        }
        other => failed(format!("{} is not an orchestrator op", other.name())),
    }
}

/// Decision 36's target, under one `DONE_CHECK_GIT_TIMEOUT` deadline for all its git
/// calls (each bounded by the run's `git_timeout_secs`, at most 10 s), on
/// `spawn_blocking`. Past the deadline the answer is the timeout; the abandoned read
/// ends on its own bound. `budget` is [`GitBudget::DONE_CHECK`] in the daemon; tests
/// pass their own.
pub(super) async fn resolve_target(
    git: std::ffi::OsString,
    git_timeout: Duration,
    budget: GitBudget,
    root: PathBuf,
    target: String,
    base_branch: String,
) -> OpResult {
    let each = budget.each(git_timeout);
    let read = tokio::task::spawn_blocking(move || {
        crate::run::git::resolve_target(&git, &root, &target, &base_branch, each)
    });
    match tokio::time::timeout(budget.deadline, read).await {
        Ok(Ok(Ok((base, head)))) => OpResult::Target { base, head },
        Ok(Ok(Err(error))) => failed(error),
        Ok(Err(error)) => failed(format!("a blocking step did not finish: {error}")),
        Err(_) => failed(format!(
            "git did not answer within {} s",
            budget.deadline.as_secs()
        )),
    }
}

/// M9.13 review, item 1: `env` with its OTLP variables (every name `otlp_env` sets)
/// taken out and, when `otlp` names a receiver as `(addr, run_id, token)`, set again for
/// it. The other variables keep their order.
pub(super) fn refreshed_otlp_env(
    env: &[(String, String)],
    otlp: Option<(&str, &str, &str)>,
) -> Vec<(String, String)> {
    let names: Vec<String> = otlp_env("", "", "").into_iter().map(|(k, _)| k).collect();
    let mut out: Vec<(String, String)> = env
        .iter()
        .filter(|(k, _)| !names.contains(k))
        .cloned()
        .collect();
    if let Some((addr, run_id, token)) = otlp {
        out.extend(otlp_env(addr, run_id, token));
    }
    out
}

impl RunService {
    /// The receiver's address as `otlp.addr` has it now, read off the runtime's threads;
    /// `None` when the receiver is not up.
    async fn otlp_addr(&self) -> Option<String> {
        let path = self.ctx.data_dir.join(crate::metering::server::ADDR_FILE);
        let addr = blocking(move || Ok(std::fs::read_to_string(path).ok())).await;
        addr.ok()
            .flatten()
            .map(|a| a.trim().to_string())
            .filter(|a| !a.is_empty())
    }

    /// M9.13 review, item 1, correcting decision 14a's "re-passes the same
    /// `RoleLaunch.env`": the receiver binds a new port with every daemon (`otlp_port =
    /// 0`), so before `RestartOrchestrator` the role's OTLP variables are set again from
    /// the `otlp.addr` of now, with the run's same token, or removed when no receiver is
    /// up (a stale endpoint would carry the token to whatever holds the old port). A
    /// Codex orchestrator is not metered this way and keeps none.
    async fn refresh_otlp(&self, ctx: &OpCtx, window_id: u32) {
        let claude = crate::lock(&self.state)
            .runs
            .get(&ctx.run_id)
            .and_then(|run| run.orch.orchestrator.as_ref())
            .is_some_and(|o| o.route.runtime == Runtime::Claude);
        let mut addr = None;
        let mut token = String::new();
        if claude {
            token = self.otlp_token(&ctx.run_id).await;
            if !token.is_empty() {
                addr = self.otlp_addr().await;
            }
        }
        let otlp = addr
            .as_deref()
            .map(|a| (a, ctx.run_id.as_str(), token.as_str()));
        self.manager
            .update_role_env(window_id, |env| refreshed_otlp_env(env, otlp));
    }

    /// M9.13 re-review, item 1: at the daemon's start, once the receiver is bound and
    /// before `serve` takes a client, every orchestrator window the manager restored
    /// has its role's OTLP variables set for `addr` (this daemon's receiver, `None`
    /// when metering is off) with its run's token, or removed; so every restart path
    /// (`run resume`'s op, a client's `anthrex restart`) launches with them. Only the
    /// engine's and the manager's locks, one at a time, and no I/O.
    pub fn refresh_orchestrator_otlp(&self, addr: Option<&str>) {
        let windows: Vec<(u32, String, Option<String>)> = crate::lock(&self.state)
            .runs
            .values()
            .filter_map(|run| {
                let o = run.orch.orchestrator.as_ref()?;
                let claude = o.route.runtime == Runtime::Claude && !o.otlp_token.is_empty();
                Some((
                    o.window_id?,
                    run.id.clone(),
                    claude.then(|| o.otlp_token.clone()),
                ))
            })
            .collect();
        for (window_id, run_id, token) in windows {
            let otlp = addr
                .zip(token.as_deref())
                .map(|(a, t)| (a, run_id.as_str(), t));
            self.manager
                .update_role_env(window_id, |env| refreshed_otlp_env(env, otlp));
        }
    }

    /// Decisions 5, 10 and 14a: the role's environment (for Claude, the OTLP variables
    /// with the run's token when the receiver is up) and the credential scrub, then the
    /// window, then its run-live flag.
    async fn create_orchestrator(
        self: &Arc<Self>,
        ctx: &OpCtx,
        spec: proto::WindowSpec,
        mut role: RoleLaunch,
        project: PathBuf,
    ) -> OpResult {
        let token = self.otlp_token(&ctx.run_id).await;
        let claude_auth = crate::lock(&self.state)
            .runs
            .get(&ctx.run_id)
            .map_or(ClaudeAuth::Login, |run| run.limits.claude_auth);
        if spec.runtime == Runtime::Claude
            && !token.is_empty()
            && let Some(addr) = self.otlp_addr().await
        {
            role.env.extend(otlp_env(&addr, &ctx.run_id, &token));
        }
        role.remove_env = crate::headless::credential_scrub_for(spec.runtime, auth(claude_auth))
            .into_iter()
            .map(str::to_string)
            .collect();
        match self.manager.create_run_window(spec, project, role).await {
            Ok(info) => {
                self.mark_live(&ctx.run_id, info.id);
                OpResult::Window {
                    window_id: info.id,
                    pid: self.manager.child_pid(info.id).ok().flatten(),
                }
            }
            Err(error) => failed(error.to_string()),
        }
    }

    /// Decision 14a: the run's token, drawn now (and sent to the engine, which keeps
    /// it in `run.json`, before this op's result) when its record has none. Empty when
    /// the random source cannot be read: the run is then not metered.
    async fn otlp_token(&self, run_id: &str) -> String {
        let kept = crate::lock(&self.state)
            .runs
            .get(run_id)
            .and_then(|run| run.orch.orchestrator.as_ref())
            .map(|o| o.otlp_token.clone())
            .unwrap_or_default();
        if !kept.is_empty() {
            return kept;
        }
        match blocking(fresh_token).await {
            Ok(token) => {
                self.send(EventKind::Orch(OrchEvent::OtlpToken {
                    run_id: run_id.to_string(),
                    token: token.clone(),
                }));
                token
            }
            Err(error) => {
                tracing::warn!(run = %run_id, %error, "no OTLP token; the orchestrator is not metered");
                String::new()
            }
        }
    }

    /// Decision 11: window `id` is the orchestrator of a run that has not ended.
    fn mark_live(&self, run_id: &str, id: u32) {
        let ended = crate::lock(&self.state)
            .runs
            .get(run_id)
            .is_none_or(|run| run.state.is_terminal());
        if !ended {
            self.manager.set_run_window_live(id, true);
        }
    }

    /// Decision 11, after a daemon restart: every restored orchestrator window of a run
    /// that has not ended is protected again, a dormant one included.
    pub(super) fn mark_restored_orchestrators_live(&self) {
        let windows: Vec<u32> = crate::lock(&self.state)
            .runs
            .values()
            .filter(|run| !run.state.is_terminal())
            .filter_map(|run| run.orch.orchestrator.as_ref()?.window_id)
            .collect();
        for id in windows {
            self.manager.set_run_window_live(id, true);
        }
    }

    /// Decisions 11 and 30, M9.13a review item 8: a run that has ended leaves its
    /// orchestrator window a plain one as soon as the step that ended it, under the
    /// engine lock. A client that sees the run ended through any request, which takes
    /// the engine lock, can then kill the window: before, the flag waited for the
    /// step's saves and its publish. Only a window that is still that run's is freed
    /// (re-review, item 5: a restart may give its id to another run's window).
    ///
    /// Lock order (re-review, item 6): this is the first place that takes the
    /// manager's lock inside the engine's. It is safe because no path takes them the
    /// other way: the manager never calls into the run service, its lock is held only
    /// inside its own methods, and neither lock is held across I/O or an `await`
    /// here (`end_run_window` changes one flag).
    pub(super) fn release_ended_orchestrators(&self, state: &crate::run::engine::EngineState) {
        for run in state.runs.values().filter(|run| run.state.is_terminal()) {
            if let Some(id) = run.orch.orchestrator.as_ref().and_then(|o| o.window_id) {
                self.manager.end_run_window(id, &run.id);
            }
        }
    }

    /// Decisions 11 and 30: a published run that has ended leaves its orchestrator
    /// window a plain one.
    pub(super) fn clear_ended_orchestrators(&self, snap: &RunsSnapshot) {
        for run in snap.runs.iter().filter(|run| run.state.is_terminal()) {
            if let Some(id) = run.orchestrator.as_ref().and_then(|o| o.window_id) {
                self.manager.end_run_window(id, &run.run_id);
            }
        }
    }

    /// Decision 20: a run scout on the scout service; its end is sent as
    /// `OrchEvent::ScoutEnded` after this op's result. Decision 43: its record, on the
    /// route the service gives it, is kept by the engine first.
    async fn start_scout(self: &Arc<Self>, ctx: &OpCtx, spec: ScoutSpec) -> OpResult {
        let Some(scouts) = self.adaptation.get().map(|a| a.scouts.clone()) else {
            return failed("the scout service is not running");
        };
        let scout_id = spec.id.clone();
        // M9.17 fix round 3: the route over the run's installed runtimes, the one its
        // record names.
        let (record, route) = {
            let state = crate::lock(&self.state);
            let run = state.runs.get(&ctx.run_id);
            let record =
                run.map(|run| crate::run::orch::roles::scout_record(run, &scout_id, unix_now()));
            let route = run.map(crate::run::orch::launch::scout_route_of);
            (record, route)
        };
        // Fix round 3, item 2: a run scout is routed from its run alone; with the run
        // gone there is nothing to route it from, and never the daemon's config.
        let Some(route) = route else {
            return failed("the run is gone");
        };
        // Review M-2: kept, and saved, before the session starts; a refusal (the scout
        // was stopped meanwhile, or the save failed) starts none (re-review 2, 3).
        if let Some(decision) = record
            && let Err(why) = self.keep_record(&ctx.run_id, decision).await
        {
            return failed(format!("the scout was not started: {why}"));
        }
        match scouts.start_on(spec, route).await {
            Ok(handle) => {
                let window_id = handle.window_id;
                let (service, run_id) = (self.clone(), ctx.run_id.clone());
                tokio::spawn(async move {
                    let (outcome, usage) = ended(&scouts, handle).await;
                    let id = scout_id.clone();
                    service
                        .settled(
                            &run_id,
                            move |k| matches!(k, OpKind::StartScout { spec } if spec.id == id),
                        )
                        .await;
                    service.send(EventKind::Orch(OrchEvent::ScoutEnded {
                        run_id,
                        scout_id,
                        outcome,
                        usage,
                    }));
                });
                OpResult::ScoutStarted { window_id }
            }
            Err(error) => failed(error.to_string()),
        }
    }

    /// Decisions 31, 32 and 34: a sub-planner on the scout machine, its first turn
    /// filled with its epic's scout extract first (as `CreateWindow` fills a worker's);
    /// its end is sent as `OrchEvent::PlannerEnded` after this op's result.
    async fn start_planner(self: &Arc<Self>, ctx: &OpCtx, mut spec: PlannerSpec) -> OpResult {
        let Some(scouts) = self.adaptation.get().map(|a| a.scouts.clone()) else {
            return failed("the scout service is not running");
        };
        let slot = spec.extract.take();
        let turn = std::mem::take(&mut spec.first_turn);
        spec.first_turn = self.fill_extract(ctx, slot, turn).await;
        let (epic, session) = (spec.epic.clone(), spec.session);
        match scouts.start_planner(spec).await {
            Ok(handle) => {
                let window_id = handle.window_id;
                let (service, run_id) = (self.clone(), ctx.run_id.clone());
                tokio::spawn(async move {
                    let (outcome, usage) = ended(&scouts, handle).await;
                    let e = epic.clone();
                    service
                        .settled(&run_id, move |k| {
                            matches!(k, OpKind::StartPlanner { spec }
                                if spec.epic == e && spec.session == session)
                        })
                        .await;
                    service.send(EventKind::Orch(OrchEvent::PlannerEnded {
                        run_id,
                        epic,
                        session,
                        outcome,
                        usage,
                    }));
                });
                OpResult::PlannerStarted { window_id }
            }
            Err(error) => failed(error.to_string()),
        }
    }

    /// Waits, at most [`SETTLE_WAIT`], until no pending op of `run_id` matches `op`: a
    /// session's end reaches the engine after its start's result. The engine's lock is
    /// taken for each look only.
    async fn settled(&self, run_id: &str, op: impl Fn(&OpKind) -> bool) {
        let deadline = tokio::time::Instant::now() + SETTLE_WAIT;
        loop {
            let pending = crate::lock(&self.state)
                .runs
                .get(run_id)
                .is_some_and(|run| run.pending_ops.values().any(|p| op(&p.kind)));
            if !pending || tokio::time::Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(SETTLE_POLL).await;
        }
    }
}

/// How a scout's or sub-planner's session ended, and what it spent.
async fn ended(
    scouts: &crate::scout::service::ScoutService,
    handle: ScoutHandle,
) -> (ScoutEnd, TokenUsage) {
    let spent = |id: &str| scouts.info(id).map(|i| i.usage).unwrap_or_default();
    match handle.outcome.await {
        Ok(ScoutOutcome::Report(report)) => (ScoutEnd::Reported, report.usage),
        Ok(ScoutOutcome::Accepted) => (ScoutEnd::Reported, spent(&handle.id)),
        Ok(ScoutOutcome::Failed { reason }) => (ScoutEnd::Failed { reason }, spent(&handle.id)),
        Err(_) => (
            ScoutEnd::Failed {
                reason: "the session ended without an outcome".to_string(),
            },
            spent(&handle.id),
        ),
    }
}

#[cfg(test)]
#[path = "orch_ops_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "live_flag_tests.rs"]
mod live_flag_tests;

#[cfg(test)]
#[path = "role_route_tests.rs"]
mod role_route_tests;
