//! The orchestrator's role in its PTY window (milestone 9 decisions 7–11): what a plain
//! window's launch gains when it is a run's orchestrator. Pure.
//!
//! Task M9.7 adds the type the engine's `CreateOrchestrator` op carries and the tool
//! lists; task M9.10 builds the launch flags from it.

use proto::{Effort, RunRef};
use serde::{Deserialize, Serialize};

use crate::headless::McpTarget;

/// The orchestrator's whole role: its run reference, its `anthrex mcp` target, its
/// contract, its effort, its Claude tool lists, and the environment it adds and removes
/// (decision 10; the driver fills `env` and `remove_env`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RoleLaunch {
    pub run_ref: RunRef,
    pub mcp: McpTarget,
    pub instructions: String,
    pub effort: Effort,
    pub claude_allowed_tools: Vec<String>,
    pub claude_disallowed_tools: Vec<String>,
    pub env: Vec<(String, String)>,
    #[serde(default)]
    pub remove_env: Vec<String>,
}

/// Decision 7: the orchestrator's six anthrex tools and its read-only tools.
pub const ORCHESTRATOR_ALLOWED_TOOLS: &[&str] = &[
    "mcp__anthrex__get_context",
    "mcp__anthrex__spawn_scout",
    "mcp__anthrex__spawn_subplanner",
    "mcp__anthrex__edit_plan",
    "mcp__anthrex__run_status",
    "mcp__anthrex__task_result",
    "Read",
    "Glob",
    "Grep",
];

/// Decision 7, with the M9.1 real-CLI rulings 1 and 2: the writing tools, the sub-agent
/// tool under both its names, and the outward-acting tools the checks observed.
pub const ORCHESTRATOR_DISALLOWED_TOOLS: &[&str] = &[
    "Edit",
    "Write",
    "NotebookEdit",
    "Bash",
    "Agent",
    "Task",
    "Artifact",
    "CronCreate",
    "CronDelete",
    "RemoteTrigger",
    "PushNotification",
    "SendMessage",
    "Workflow",
    "WebFetch",
    "WebSearch",
];
