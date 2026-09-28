//! The orchestrator's role in its PTY window (milestone 9 decisions 7–11): what a plain
//! window's launch gains when it is a run's orchestrator. Pure.
//!
//! Task M9.7 added the type the engine's `CreateOrchestrator` op carries and the tool
//! lists; task M9.10 builds the launch flags and the environment from it.

use proto::{Effort, RunRef, Runtime};
use serde::{Deserialize, Serialize};

use super::LaunchContext;
use crate::headless::McpTarget;
use crate::headless::argv::{CliCaps, effort, mcp_args, toml_array};
use crate::launch::codex::toml_string;
use serde_json::json;

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

/// Decision 7's middle block for a Claude orchestrator, between M3's `--settings` and
/// `--model`: the user-settings-only flags (decision 9), the anthrex MCP server, the tool
/// lists, `--permission-mode default` (M9.1 real-CLI ruling 1: the account's default is
/// auto mode, and every unlisted tool must ask the user), the contract, and the effort.
/// Every variadic flag is followed by another flag.
pub fn claude_role_args(role: &RoleLaunch, ctx: &LaunchContext<'_>, caps: &CliCaps) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    if let Some(flags) = caps.claude_user_settings_only {
        args.extend(flags.iter().map(|f| f.to_string()));
    }
    if let Some(mcp) = mcp_args(&role.mcp, ctx.window_id, ctx.socket_path) {
        let config = json!({"mcpServers": {"anthrex": {
            "type": "stdio",
            "command": ctx.exe.display().to_string(),
            "args": mcp,
        }}});
        args.extend(["--mcp-config".into(), config.to_string()]);
    }
    args.extend([
        "--allowedTools".into(),
        role.claude_allowed_tools.join(","),
        "--disallowedTools".into(),
        role.claude_disallowed_tools.join(","),
        "--permission-mode".into(),
        "default".into(),
        "--append-system-prompt".into(),
        role.instructions.clone(),
    ]);
    if caps.claude_effort_flag {
        args.extend(["--effort".into(), effort(role.effort).into()]);
    }
    args
}

/// Decision 8's block for a Codex orchestrator, between M3's hook block and `-m`: the
/// anthrex MCP server (`"approve"`, M9.1 ruling 3: `"auto"` asks per call), the contract
/// and effort, the project-config exclusion when the CLI has one, then `-s read-only -a
/// on-request`, which Codex takes ahead of `resume <id>` too. No project-trust flag
/// (ruling 7): Codex may show its own trust dialog, which the user answers.
pub fn codex_role_args(role: &RoleLaunch, ctx: &LaunchContext<'_>, caps: &CliCaps) -> Vec<String> {
    let mut args: Vec<String> = Vec::new();
    let mut config = |value: String| args.extend(["-c".into(), value]);
    if let Some(mcp) = mcp_args(&role.mcp, ctx.window_id, ctx.socket_path) {
        let exe = ctx.exe.display().to_string();
        config(format!("mcp_servers.anthrex.command={}", toml_string(&exe)));
        config(format!("mcp_servers.anthrex.args={}", toml_array(&mcp)));
        config("mcp_servers.anthrex.tool_timeout_sec=120".into());
        config(format!(
            "mcp_servers.anthrex.default_tools_approval_mode={}",
            toml_string("approve")
        ));
    }
    config(format!(
        "developer_instructions={}",
        toml_string(&role.instructions)
    ));
    config(format!(
        "model_reasoning_effort={}",
        toml_string(effort(role.effort))
    ));
    if let Some(flags) = caps.codex_user_config_only {
        args.extend(flags.iter().map(|f| f.to_string()));
    }
    args.extend(["-s", "read-only", "-a", "on-request"].map(String::from));
    args
}

/// Decision 10: what the role adds after M3's four variables: its own `env`, then, for
/// Claude only, `ENABLE_TOOL_SEARCH=false` last and once (`headless::session_vars`, the
/// rule every headless Claude session follows). No `MCP_TOOL_TIMEOUT` (M9.1 ruling 4).
pub fn launch_env(role: &RoleLaunch, runtime: Runtime) -> Vec<(String, String)> {
    crate::headless::session_vars(runtime, &role.env)
}

/// Decisions 10 and 14a: the variables the driver puts in a Claude orchestrator's
/// `RoleLaunch.env` when the OTLP receiver is up: `metering::orchestrator_env`'s, then the
/// run's token header.
pub fn otlp_env(addr: &str, run_id: &str, token: &str) -> Vec<(String, String)> {
    let mut env = crate::metering::orchestrator_env(addr, run_id);
    env.push((
        "OTEL_EXPORTER_OTLP_HEADERS".into(),
        format!("authorization=Bearer {token}"),
    ));
    env
}

#[cfg(test)]
#[path = "role_tests.rs"]
mod tests;
