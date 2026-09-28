//! Milestone 9, the orchestrator and sub-planners
//! (`docs/milestones/M9-orchestrator-and-subplanners.md`). Pure.
//!
//! The model types here are those decision 23's rules read ([`rules`]): where an edit
//! batch comes from ([`EditSource`]), the run's epics ([`RunOrch`], [`EpicRecord`]) and
//! a task's integration-review mark ([`TaskOrch`]). Task M9.7 and later add the rest of
//! decision 1's types and fields, each `#[serde(default)]`.
//!
//! [`OrchLimits`] is `RunLimits.orch`: the orchestrator settings a run is frozen with at
//! start (ruling D-5), so a later config edit cannot change the rules of a live run. The
//! config crate has no serde, so `config::PlannerConfig` cannot be persisted; these are
//! its serde mirrors, of proto types.

use proto::{Effort, IntegrationState, Route, Runtime, Strength, TokenUsage};
use serde::{Deserialize, Serialize};

use super::model::OpId;

pub mod rules;

/// Who sent an edit batch. Plan files and the user's `run edit` are [`EditSource::User`]
/// and keep M8a's rules only; the orchestrator's `edit_plan` and a sub-planner's
/// `submit_epic` also meet decision 23's ([`rules::check`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditSource {
    User,
    Orchestrator,
    Planner { epic: String },
}

impl EditSource {
    /// `user`, `orchestrator` or `planner:<e>` (decision 40's edit-log source).
    pub fn label(&self) -> String {
        match self {
            EditSource::User => "user".into(),
            EditSource::Orchestrator => "orchestrator".into(),
            EditSource::Planner { epic } => format!("planner:{epic}"),
        }
    }
}

/// `Run.orch`: a run's milestone 9 state. Absent from an older run: empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunOrch {
    /// Every epic the orchestrator created with `spawn_subplanner`, in creation order.
    pub epics: Vec<EpicRecord>,
}

/// `Task.orch`: a task's milestone 9 state. Absent from an older run: empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskOrch {
    /// Decision 37: the epic an engine-made integration review task reviews.
    pub integration_of: Option<String>,
}

/// A sub-planner's phase (decision 32). `Queued` and `Planning` are live.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerPhase {
    #[default]
    Queued,
    Planning,
    Finished,
    Failed {
        reason: String,
    },
}

impl PlannerPhase {
    /// Queued or planning: the epic is its sub-planner's (decision 23.5).
    pub fn is_live(&self) -> bool {
        matches!(self, PlannerPhase::Queued | PlannerPhase::Planning)
    }
}

/// One epic and its sub-planner sessions (decisions 31–33, 37).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpicRecord {
    pub epic: String,
    pub title: String,
    pub area: Vec<String>,
    pub brief: String,
    pub scout_refs: Vec<String>,
    pub route: Route,
    pub phase: PlannerPhase,
    /// The live or last session's brief.
    pub request: String,
    pub sessions: Vec<PlannerSession>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub edits_accepted: u32,
    pub edits_rejected: u32,
    pub last_rejection: Option<String>,
    pub replans: Vec<String>,
    pub note: Option<String>,
    pub gate_hold: Option<String>,
    pub base: Option<String>,
    /// `(task id, merge commit)`.
    pub merges: Vec<(String, String)>,
    pub integration_state: IntegrationState,
    pub integration_rounds: u32,
}

/// One sub-planner session of an epic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerSession {
    pub session: u32,
    pub window_id: Option<u32>,
    pub op: Option<OpId>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub usage: TokenUsage,
}

#[cfg(test)]
impl EpicRecord {
    /// A test epic `epic` in `phase`, with no sessions.
    pub fn new(epic: &str, phase: PlannerPhase) -> EpicRecord {
        EpicRecord {
            epic: epic.into(),
            title: format!("Epic {epic}"),
            area: vec![format!("crates/{epic}/**")],
            brief: String::new(),
            scout_refs: Vec::new(),
            route: Route {
                runtime: Runtime::Claude,
                model: "claude-opus-5".into(),
                strength: Strength::Frontier,
                effort: Effort::High,
            },
            phase,
            request: String::new(),
            sessions: Vec::new(),
            started_at: 0,
            ended_at: None,
            edits_accepted: 0,
            edits_rejected: 0,
            last_rejection: None,
            replans: Vec::new(),
            note: None,
            gate_hold: None,
            base: None,
            merges: Vec::new(),
            integration_state: IntegrationState::default(),
            integration_rounds: 0,
        }
    }
}

/// `[orchestrator]`'s milestone 9 keys, as the run was built with them. Absent from a
/// run recorded before milestone 9: the config defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OrchLimits {
    pub planner_task_cap: u32,
    pub max_scouts: u32,
    pub wake_orchestrator: bool,
    pub wake_quiet_secs: u64,
    /// Zero turns messages off.
    pub message_max_per_turn: u32,
    /// Zero turns notes off.
    pub note_max_per_task: u32,
    pub planners: PlannerLimits,
}

/// `[orchestrator.planners]`. `runtime` `None` means the orchestrator's runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlannerLimits {
    pub runtime: Option<Runtime>,
    pub strength: Strength,
    pub effort: Effort,
    pub max_tool_calls: u32,
    pub timeout_secs: u64,
    pub max_rejections: u32,
}

impl OrchLimits {
    /// The limits `[orchestrator]` gives a run built now.
    pub fn from_config(config: &config::Orchestrator) -> OrchLimits {
        let a = &config.agent;
        let p = &a.planners;
        OrchLimits {
            planner_task_cap: a.planner_task_cap,
            max_scouts: a.max_scouts,
            wake_orchestrator: a.wake_orchestrator,
            wake_quiet_secs: a.wake_quiet_secs,
            message_max_per_turn: a.message_max_per_turn,
            note_max_per_task: a.note_max_per_task,
            planners: PlannerLimits {
                runtime: p.runtime,
                strength: p.strength,
                effort: p.effort,
                max_tool_calls: p.max_tool_calls,
                timeout_secs: p.timeout_secs,
                max_rejections: p.max_rejections,
            },
        }
    }
}

impl Default for OrchLimits {
    fn default() -> Self {
        OrchLimits::from_config(&config::Orchestrator::default())
    }
}

impl Default for PlannerLimits {
    fn default() -> Self {
        OrchLimits::default().planners
    }
}
