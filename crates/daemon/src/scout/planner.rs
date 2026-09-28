//! Sub-planner sessions on M8b's scout machine (milestone 9 decision 31): the same
//! table, machine and session driving as a scout, with the planner's own limits and
//! texts. A planner never reports through `submit_scout_report`: its one write,
//! `submit_epic`, goes to the engine, which answers it and then asks this service to
//! retire the session ([`ScoutService::accept_planner`]) or to stop it
//! ([`ScoutService::stop_planner`]). I/O, like `service.rs`.

use std::path::PathBuf;
use std::sync::Arc;

use proto::{Route, ScoutKind};
use serde::{Deserialize, Serialize};

use super::machine::{PLANNER_TEXTS, ScoutEvent, ScoutLimits};
use super::service::{ScoutHandle, ScoutService};
use super::spec::{ScoutSpec, valid_id};
use crate::headless::HeadlessSpec;
use crate::run::orch::extract::ExtractSlot;

/// One sub-planner session to launch (decision 31), built by the engine
/// (`run::orch::launch::planner_spec`) and carried by `OpKind::StartPlanner`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerSpec {
    pub run_id: String,
    pub epic: String,
    pub session: u32,
    pub headless: HeadlessSpec,
    pub first_turn: String,
    pub project: PathBuf,
    pub cwd: PathBuf,
    pub route: Route,
    /// `[orchestrator.planners]` as the run was built with them (the service knows only
    /// `[orchestrator.scouts]`).
    pub max_tool_calls: u32,
    pub timeout_secs: u64,
    /// Decision 34: where the driver puts the epic's scout extract into `first_turn`.
    pub extract: Option<ExtractSlot>,
}

/// A table entry's mark as a sub-planner's session: whose, and its limits.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerTag {
    pub run_id: String,
    pub epic: String,
    pub session: u32,
    pub limits: ScoutLimits,
}

/// A run's `<h4>` (`Run::short`): the last four characters of its id.
fn short(run_id: &str) -> &str {
    let cut = run_id.len().saturating_sub(4);
    run_id.get(cut..).unwrap_or(run_id)
}

/// Decision 31's internal id `<h4>-plan-<e>-<n>` and window name `<h4>/plan-<e>.p<n>`.
pub fn planner_names(run_id: &str, epic: &str, session: u32) -> (String, String) {
    let h4 = short(run_id);
    (
        format!("{h4}-plan-{epic}-{session}"),
        format!("{h4}/plan-{epic}.p{session}"),
    )
}

impl ScoutService {
    /// Starts sub-planner session `spec` on the scout machine, with the planner's
    /// limits and texts; its window is `<h4>/plan-<e>.p<n>`.
    pub async fn start_planner(self: &Arc<Self>, spec: PlannerSpec) -> anyhow::Result<ScoutHandle> {
        let (id, name) = planner_names(&spec.run_id, &spec.epic, spec.session);
        anyhow::ensure!(valid_id(&id), "invalid sub-planner id {id:?}");
        let tag = PlannerTag {
            run_id: spec.run_id.clone(),
            epic: spec.epic.clone(),
            session: spec.session,
            limits: ScoutLimits {
                timeout_secs: spec.timeout_secs,
                max_tool_calls: spec.max_tool_calls,
                send_mid_turn: spec.route.runtime == proto::Runtime::Claude,
                texts: PLANNER_TEXTS,
            },
        };
        // The table's bookkeeping spec: a planner stores no report, so nothing but its
        // id, run and launch fields is ever read.
        let entry = ScoutSpec {
            id,
            kind: ScoutKind::Area,
            run_id: Some(spec.run_id.clone()),
            question: format!("plan epic {}", spec.epic),
            first_turn: spec.first_turn,
            cwd: spec.cwd,
            project: spec.project,
            web: false,
            codex_config: Vec::new(),
            base_sha: String::new(),
            repo_paths: Vec::new(),
        };
        self.launch(entry, spec.route, spec.headless, name, Some(tag))
            .await
    }

    /// The engine accepted the planner's epic (decision 22): the machine's
    /// `ReportAccepted` closes its stdin, kills it after the grace, and removes its
    /// window after `RETIRE_AFTER`.
    pub fn accept_planner(&self, window_id: u32) {
        if let Some(id) = self.planner_at(window_id) {
            self.drive(&id, ScoutEvent::ReportAccepted);
        }
    }

    /// The engine failed the planner (decision 22's `max_rejections`): the machine
    /// kills the session with the engine's reason.
    pub fn stop_planner(&self, window_id: u32, reason: &str) {
        if let Some(id) = self.planner_at(window_id) {
            let reason = reason.to_string();
            self.drive(&id, ScoutEvent::Halt { reason });
        }
    }
}
