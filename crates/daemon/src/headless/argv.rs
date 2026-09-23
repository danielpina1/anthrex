//! The argv of every headless session (decisions 24, 25, 50, 53 and 54) and what the
//! installed CLIs can do (`CLI_CAPS`, set from M8a.1's findings). Pure.

use super::{ClaudeSandbox, HeadlessSpec, McpTarget, SessionArg};
use crate::launch;
use crate::launch::codex::toml_string;
use proto::{AgentRole, Effort};
use serde_json::{Map, Value, json};
use std::path::Path;

/// What the installed `claude` and `codex` accept, as M8a.1 found it (the "`CLI_CAPS`"
/// table in the milestone's implementation notes). Every argv builder takes one, so tests
/// can exercise each branch whatever the installed CLI does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CliCaps {
    /// `--output-format stream-json` under `-p` requires `--verbose`.
    pub claude_verbose: bool,
    pub claude_permission_prompts: bool,
    pub claude_effort_flag: bool,
    /// An explicit opt-out of `--bare` for `auth = "login"` (decision 50).
    pub claude_non_bare_flag: Option<&'static str>,
    pub claude_interrupt: InterruptMode,
    pub claude_hooks_fire_in_print: bool,
    /// Whether `codex exec resume` accepts `-s`; when not, `-c sandbox_mode=…`.
    pub codex_resume_takes_sandbox: bool,
    /// The flags that exclude project settings and `.mcp.json` (decision 53); `None`:
    /// impossible.
    pub claude_user_settings_only: Option<&'static [&'static str]>,
    pub claude_sandbox_keys: SandboxKeys,
    pub codex_loads_project_config: bool,
    pub codex_project_config_paths: &'static [&'static str],
    /// The flags that stop Codex loading project config; `None`: impossible.
    pub codex_user_config_only: Option<&'static [&'static str]>,
}

/// Decision 54's key names under `"sandbox"`. `write_allow` is a dotted path into
/// nested objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SandboxKeys {
    pub enabled: &'static str,
    pub allow_unsandboxed: &'static str,
    pub write_allow: &'static str,
    /// M8a.1 item 4b: without it a sandbox that cannot start only warns and runs
    /// commands unsandboxed.
    pub fail_if_unavailable: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InterruptMode {
    ControlRequest,
    Sigint,
}

/// M8a.1's findings against `claude` 2.1.278 and `codex-cli` 0.155.0.
pub const CLI_CAPS: CliCaps = CliCaps {
    claude_verbose: true,
    claude_permission_prompts: true,
    claude_effort_flag: true,
    claude_non_bare_flag: None,
    claude_interrupt: InterruptMode::ControlRequest,
    claude_hooks_fire_in_print: true,
    codex_resume_takes_sandbox: false,
    claude_user_settings_only: Some(&["--setting-sources", "user", "--strict-mcp-config"]),
    claude_sandbox_keys: SandboxKeys {
        enabled: "enabled",
        allow_unsandboxed: "allowUnsandboxedCommands",
        write_allow: "filesystem.allowWrite",
        fail_if_unavailable: "failIfUnavailable",
    },
    codex_loads_project_config: true,
    codex_project_config_paths: &[".codex/config.toml", ".codex/hooks.json"],
    codex_user_config_only: None,
};

/// M3's hook settings (`launch::claude::settings`, unchanged) plus, for a worker,
/// decision 54's sandbox block: enabled, unsandboxed commands disallowed, refusing to
/// start without a working sandbox (M8a.1 item 4b), and the git common dir writable.
pub fn claude_settings(
    exe: &Path,
    window_id: u32,
    sandbox: Option<&ClaudeSandbox>,
    caps: &CliCaps,
) -> Value {
    let mut settings = launch::claude::settings(exe, window_id);
    if let Some(sandbox) = sandbox {
        let keys = &caps.claude_sandbox_keys;
        let mut block = Map::new();
        block.insert(keys.enabled.into(), Value::Bool(true));
        block.insert(keys.allow_unsandboxed.into(), Value::Bool(false));
        block.insert(keys.fail_if_unavailable.into(), Value::Bool(true));
        let roots = sandbox
            .writable_roots
            .iter()
            .map(|p| Value::String(p.display().to_string()))
            .collect();
        insert_path(&mut block, keys.write_allow, Value::Array(roots));
        settings["sandbox"] = Value::Object(block);
    }
    settings
}

/// Inserts `value` at the dotted `path`, creating the objects on the way.
fn insert_path(object: &mut Map<String, Value>, path: &str, value: Value) {
    match path.split_once('.') {
        None => {
            object.insert(path.into(), value);
        }
        Some((head, rest)) => {
            let child = object
                .entry(head)
                .or_insert_with(|| Value::Object(Map::new()));
            if !child.is_object() {
                *child = Value::Object(Map::new());
            }
            if let Value::Object(child) = child {
                insert_path(child, rest, value);
            }
        }
    }
}

/// The `anthrex mcp` argv of the CLI section.
pub fn mcp_args(target: &McpTarget, window_id: u32, socket: &Path) -> Vec<String> {
    let role = match target.role {
        AgentRole::Orchestrator => "orchestrator",
        AgentRole::Worker => "worker",
        AgentRole::Reviewer => "reviewer",
    };
    let mut args = vec![
        "mcp".to_string(),
        "--role".into(),
        role.into(),
        "--run".into(),
        target.run_id.clone(),
    ];
    if let Some(task) = &target.task_id {
        args.extend(["--task".into(), task.clone()]);
    }
    args.extend([
        "--window".into(),
        window_id.to_string(),
        "--socket".into(),
        socket.display().to_string(),
    ]);
    args
}

/// A Claude session's argv (decision 24), in this order: the stream flags, the
/// permission-prompt flag, the session argument, the user-settings-only flags (decision
/// 53), `--settings`, `--mcp-config`, `--allowedTools`, `--disallowedTools`,
/// `--append-system-prompt`, `--permission-mode`, `--model`, `--effort`, then the auth
/// flag (decision 50). Every variadic flag is followed by another flag. The prompt is
/// never on the argv: the first turn is a stream-json message on stdin.
pub fn claude_args(
    spec: &HeadlessSpec,
    session: &SessionArg,
    exe: &Path,
    window_id: u32,
    socket: &Path,
    caps: &CliCaps,
) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--input-format",
        "stream-json",
        "--output-format",
        "stream-json",
    ]
    .map(String::from)
    .to_vec();
    if caps.claude_verbose {
        args.push("--verbose".into());
    }
    if caps.claude_permission_prompts {
        args.extend(["--permission-prompts".into(), "none".into()]);
    }
    match session {
        SessionArg::New { uuid: Some(uuid) } => {
            args.extend(["--session-id".into(), uuid.clone()]);
        }
        SessionArg::New { uuid: None } => {}
        SessionArg::Resume { session_id } => {
            args.extend(["--resume".into(), session_id.clone()]);
        }
    }
    if let Some(flags) = caps.claude_user_settings_only {
        args.extend(flags.iter().map(|f| f.to_string()));
    }
    let mut settings = claude_settings(exe, window_id, spec.claude_sandbox.as_ref(), caps);
    if spec.claude_auth == config::ClaudeAuth::ApiKey
        && let Some(helper) = &spec.api_key_helper
    {
        settings["apiKeyHelper"] = Value::String(helper.clone());
    }
    args.extend(["--settings".into(), settings.to_string()]);
    if let Some(target) = &spec.mcp {
        let config = json!({"mcpServers": {"anthrex": {
            "type": "stdio",
            "command": exe.display().to_string(),
            "args": mcp_args(target, window_id, socket),
        }}});
        args.extend(["--mcp-config".into(), config.to_string()]);
    }
    if !spec.allowed_tools.is_empty() {
        args.extend(["--allowedTools".into(), spec.allowed_tools.join(",")]);
    }
    if !spec.claude_disallowed_tools.is_empty() {
        args.extend([
            "--disallowedTools".into(),
            spec.claude_disallowed_tools.join(","),
        ]);
    }
    args.extend(["--append-system-prompt".into(), spec.instructions.clone()]);
    if let Some(mode) = &spec.claude_permission_mode {
        args.extend(["--permission-mode".into(), mode.clone()]);
    }
    if !spec.model.is_empty() {
        args.extend(["--model".into(), spec.model.clone()]);
    }
    if caps.claude_effort_flag {
        args.extend(["--effort".into(), effort(spec.effort).into()]);
    }
    match spec.claude_auth {
        config::ClaudeAuth::ApiKey => args.push("--bare".into()),
        config::ClaudeAuth::Login => {
            if let Some(flag) = caps.claude_non_bare_flag {
                args.push(flag.into());
            }
        }
    }
    args
}

/// One Codex turn's argv (decision 25): `exec --json` for the first turn, `exec resume
/// <id> --json` for every later one, then the project-config exclusion when the CLI has
/// one, the MCP server, the instructions, effort and approval policy, the sandbox, a
/// worker's writable roots, the model when named, `--`, and the turn's message. `exec
/// resume` rejects `-s` (M8a.1 item 6), so a resume passes `-c sandbox_mode=…` unless
/// the caps say otherwise. Codex's default writable `/tmp` is left alone. Every TOML
/// string comes from `launch::codex::toml_string`.
pub fn codex_args(
    spec: &HeadlessSpec,
    session: &SessionArg,
    message: &str,
    exe: &Path,
    window_id: u32,
    socket: &Path,
    caps: &CliCaps,
) -> Vec<String> {
    let mut args = vec!["exec".to_string()];
    let resuming = match session {
        SessionArg::Resume { session_id } => {
            args.extend(["resume".into(), session_id.clone()]);
            true
        }
        SessionArg::New { .. } => false,
    };
    args.push("--json".into());
    if let Some(flags) = caps.codex_user_config_only {
        args.extend(flags.iter().map(|f| f.to_string()));
    }
    let mut config = |value: String| args.extend(["-c".into(), value]);
    if let Some(target) = &spec.mcp {
        let mcp = mcp_args(target, window_id, socket);
        config(format!(
            "mcp_servers.anthrex.command={}",
            toml_string(&exe.display().to_string())
        ));
        config(format!("mcp_servers.anthrex.args={}", toml_array(&mcp)));
        config("mcp_servers.anthrex.tool_timeout_sec=120".into());
        config(format!(
            "mcp_servers.anthrex.default_tools_approval_mode={}",
            toml_string("auto")
        ));
    }
    config(format!(
        "developer_instructions={}",
        toml_string(&spec.instructions)
    ));
    config(format!(
        "model_reasoning_effort={}",
        toml_string(effort(spec.effort))
    ));
    config(format!("approval_policy={}", toml_string("never")));
    if resuming && !caps.codex_resume_takes_sandbox {
        config(format!("sandbox_mode={}", toml_string(&spec.codex_sandbox)));
    } else {
        args.extend(["-s".into(), spec.codex_sandbox.clone()]);
    }
    if !spec.codex_writable_roots.is_empty() {
        let roots: Vec<String> = spec
            .codex_writable_roots
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        args.extend([
            "-c".into(),
            format!(
                "sandbox_workspace_write.writable_roots={}",
                toml_array(&roots)
            ),
        ]);
    }
    if !spec.model.is_empty() {
        args.extend(["-m".into(), spec.model.clone()]);
    }
    args.extend(["--".into(), message.to_owned()]);
    args
}

/// A TOML array of basic strings: `[` + the strings joined by `,` + `]`.
fn toml_array(items: &[String]) -> String {
    let strings: Vec<String> = items.iter().map(|s| toml_string(s)).collect();
    format!("[{}]", strings.join(","))
}

fn effort(effort: Effort) -> &'static str {
    match effort {
        Effort::Low => "low",
        Effort::Medium => "medium",
        Effort::High => "high",
    }
}

#[cfg(test)]
#[path = "argv_tests.rs"]
mod tests;
