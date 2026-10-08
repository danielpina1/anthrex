//! [`OrchLimits`], [`AgentLimits`], [`PlannerLimits`] and [`DesignLimits`]: the
//! `[orchestrator]` keys a run is frozen with, moved out of `orch/mod.rs` (milestone
//! 9.3, task 3) and re-exported there. Pure.

use proto::{Budget, Effort, Runtime, Strength};
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
    /// `[orchestrator.agent]`, frozen too (task M9.7): `run promote` and a promotion
    /// recorded before milestone 9 resolve the orchestrator's route from the run alone.
    pub agent: AgentLimits,
    /// Milestone 9.6: `[orchestrator.design]`, frozen with the run's mode
    /// (`Run.design_mode`). Not written while it is the default, so a run recorded
    /// before 9.6 is written back as it was read.
    #[serde(default, skip_serializing_if = "DesignLimits::is_default")]
    pub design: DesignLimits,
}

/// `[orchestrator.design]` without its `default` (the mode is decided once, at the
/// start, and frozen as `Run.design_mode`): where the documents are committed, the
/// questions, the orchestrator phases' budget and the design agents' budgets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesignLimits {
    /// Empty: the documents are never committed.
    pub docs_dir: String,
    pub commit_brainstorm: bool,
    pub max_questions: u32,
    pub phase_minutes: u32,
    pub brainstormer: Budget,
    pub doc_reviewer: Budget,
}

impl DesignLimits {
    pub fn from_config(design: &config::DesignConfig) -> DesignLimits {
        DesignLimits {
            docs_dir: design.docs_dir.clone(),
            commit_brainstorm: design.commit_brainstorm,
            max_questions: design.max_questions,
            phase_minutes: design.phase_minutes,
            brainstormer: design.budget.brainstormer,
            doc_reviewer: design.budget.doc_reviewer,
        }
    }

    fn is_default(&self) -> bool {
        *self == DesignLimits::default()
    }
}

impl Default for DesignLimits {
    fn default() -> Self {
        DesignLimits::from_config(&config::DesignConfig::default())
    }
}

/// `[orchestrator.agent]`: `runtime` `None` means `default_runtime`; an empty `model`
/// means decision 6's resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentLimits {
    pub runtime: Option<Runtime>,
    pub model: String,
    pub effort: Effort,
}

impl AgentLimits {
    /// The `config::AgentConfig` these limits were frozen from.
    pub fn config(&self) -> config::AgentConfig {
        config::AgentConfig {
            runtime: self.runtime,
            model: self.model.clone(),
            effort: self.effort.clone(),
        }
    }
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
                effort: p.effort.clone(),
                max_tool_calls: p.max_tool_calls,
                timeout_secs: p.timeout_secs,
                max_rejections: p.max_rejections,
            },
            agent: AgentLimits {
                runtime: a.agent.runtime,
                model: a.agent.model.clone(),
                effort: a.agent.effort.clone(),
            },
            design: DesignLimits::from_config(&config.design),
        }
    }
}

impl Default for OrchLimits {
    fn default() -> Self {
        OrchLimits::from_config(&config::Orchestrator::default())
    }
}

impl Default for AgentLimits {
    fn default() -> Self {
        OrchLimits::default().agent
    }
}

impl Default for PlannerLimits {
    fn default() -> Self {
        OrchLimits::default().planners
    }
}
