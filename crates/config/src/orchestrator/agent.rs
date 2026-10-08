//! Milestone 9's settings (Interfaces "`config`"): the orchestrator agent, its
//! sub-planners, waking, and the message and note limits. Top-level keys of
//! `[orchestrator]` plus the `[orchestrator.agent]` and `[orchestrator.planners]`
//! tables, read by one call, [`read`], from `orchestrator::read`; their unknown keys are
//! reported by `orchestrator/unknown.rs`.
//!
//! As everywhere in `[orchestrator]`, an invalid value is a [`Problem`] and keeps its
//! default. A non-empty `agent.model` is not checked against the roster here: `run
//! start --goal` refuses it with decision 6's text.

use proto::{Effort, Runtime};

use super::adapt::{read_effort, read_strength, sub_table};
use super::read_u32_in_range;
use super::roster::LegacyStrength;
use crate::{Problem, read_bool_key, read_u64_in_range};

pub(super) const KNOWN_AGENT_KEYS: &[&str] = &["runtime", "model", "effort"];
pub(super) const KNOWN_PLANNERS_KEYS: &[&str] = &[
    "runtime",
    "strength",
    "effort",
    "max_tool_calls",
    "timeout_secs",
    "max_rejections",
];

/// The orchestrator agent's settings. `config::Orchestrator.agent`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentSettings {
    pub planner_task_cap: u32,
    /// Per run.
    pub max_scouts: u32,
    pub wake_orchestrator: bool,
    pub wake_quiet_secs: u64,
    /// Zero turns messages off.
    pub message_max_per_turn: u32,
    /// Zero turns notes off.
    pub note_max_per_task: u32,
    pub agent: AgentConfig,
    pub planners: PlannerConfig,
}

/// `[orchestrator.agent]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentConfig {
    /// `None` means `orchestrator.default_runtime`.
    pub runtime: Option<Runtime>,
    /// Empty means decision 6's resolution.
    pub model: String,
    pub effort: Effort,
}

/// `[orchestrator.planners]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannerConfig {
    /// `None` means the orchestrator's runtime.
    pub runtime: Option<Runtime>,
    /// Read only to migrate the old keys (task M9.8.14).
    pub(crate) strength: LegacyStrength,
    pub effort: Effort,
    pub max_tool_calls: u32,
    pub timeout_secs: u64,
    pub max_rejections: u32,
}

impl Default for AgentSettings {
    fn default() -> Self {
        AgentSettings {
            planner_task_cap: 12,
            max_scouts: 12,
            wake_orchestrator: true,
            wake_quiet_secs: 5,
            message_max_per_turn: 3,
            note_max_per_task: 10,
            agent: AgentConfig::default(),
            planners: PlannerConfig::default(),
        }
    }
}

impl Default for AgentConfig {
    fn default() -> Self {
        AgentConfig {
            runtime: None,
            model: String::new(),
            effort: Effort::HIGH,
        }
    }
}

impl Default for PlannerConfig {
    fn default() -> Self {
        PlannerConfig {
            runtime: None,
            strength: LegacyStrength::Frontier,
            effort: Effort::HIGH,
            max_tool_calls: 200,
            timeout_secs: 2400,
            max_rejections: 5,
        }
    }
}

/// Reads every milestone 9 key of `[orchestrator]` (`table`). Never fails.
pub(crate) fn read(table: &toml::Table, problems: &mut Vec<Problem>) -> AgentSettings {
    let mut s = AgentSettings::default();
    let key = |k: &str| format!("orchestrator.{k}");
    for (k, range, field) in [
        ("planner_task_cap", 2..=50, &mut s.planner_task_cap),
        ("max_scouts", 1..=50, &mut s.max_scouts),
        ("message_max_per_turn", 0..=20, &mut s.message_max_per_turn),
        ("note_max_per_task", 0..=100, &mut s.note_max_per_task),
    ] {
        read_u32_in_range(table, k, &key(k), &range, field, problems);
    }
    read_bool_key(
        table,
        "wake_orchestrator",
        "orchestrator.wake_orchestrator",
        &mut s.wake_orchestrator,
        problems,
    );
    read_u64_in_range(
        table,
        "wake_quiet_secs",
        "orchestrator.wake_quiet_secs",
        &(1..=120),
        &mut s.wake_quiet_secs,
        problems,
    );
    if let Some(t) = sub_table(table, "agent", problems) {
        read_agent(t, &mut s.agent, problems);
    }
    if let Some(t) = sub_table(table, "planners", problems) {
        read_planners(t, &mut s.planners, problems);
    }
    s
}

fn read_agent(t: &toml::Table, a: &mut AgentConfig, problems: &mut Vec<Problem>) {
    let prefix = "orchestrator.agent";
    read_runtime(
        t,
        prefix,
        "orchestrator.default_runtime",
        &mut a.runtime,
        problems,
    );
    if let Some(v) = t.get("model") {
        match v.as_str() {
            Some(m) => a.model = m.to_string(),
            None => problems.push(Problem {
                key: format!("{prefix}.model"),
                message: "expected a string".to_string(),
                default: "unset".to_string(),
            }),
        }
    }
    read_effort(t, prefix, &mut a.effort, problems);
}

fn read_planners(t: &toml::Table, p: &mut PlannerConfig, problems: &mut Vec<Problem>) {
    let prefix = "orchestrator.planners";
    read_runtime(
        t,
        prefix,
        "the orchestrator's runtime",
        &mut p.runtime,
        problems,
    );
    read_strength(t, prefix, &mut p.strength, problems);
    read_effort(t, prefix, &mut p.effort, problems);
    let key = |k: &str| format!("{prefix}.{k}");
    for (k, range, field) in [
        ("max_tool_calls", 20..=2000, &mut p.max_tool_calls),
        ("max_rejections", 1..=20, &mut p.max_rejections),
    ] {
        read_u32_in_range(t, k, &key(k), &range, field, problems);
    }
    read_u64_in_range(
        t,
        "timeout_secs",
        "orchestrator.planners.timeout_secs",
        &(120..=14400),
        &mut p.timeout_secs,
        problems,
    );
}

/// `runtime`: `claude` or `codex`. `unset` names what an absent key means.
fn read_runtime(
    t: &toml::Table,
    prefix: &str,
    unset: &str,
    field: &mut Option<Runtime>,
    problems: &mut Vec<Problem>,
) {
    let Some(v) = t.get("runtime") else {
        return;
    };
    match v.as_str() {
        Some("claude") => *field = Some(Runtime::Claude),
        Some("codex") => *field = Some(Runtime::Codex),
        _ => problems.push(Problem {
            key: format!("{prefix}.runtime"),
            message: "must be claude or codex".to_string(),
            default: match field {
                Some(r) => r.to_string(),
                None => unset.to_string(),
            },
        }),
    }
}
