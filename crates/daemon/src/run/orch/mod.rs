//! Milestone 9, the orchestrator and sub-planners
//! (`docs/milestones/M9-orchestrator-and-subplanners.md`). Pure.
//!
//! [`OrchLimits`] is `RunLimits.orch`: the orchestrator settings a run is frozen with at
//! start (ruling D-5), so a later config edit cannot change the rules of a live run. The
//! config crate has no serde, so `config::PlannerConfig` cannot be persisted; these are
//! its serde mirrors, of proto types.

use proto::{Effort, Runtime, Strength};
use serde::{Deserialize, Serialize};

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
