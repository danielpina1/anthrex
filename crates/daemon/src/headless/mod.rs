//! The headless session layer (decisions 24 to 27 and 49 to 54): every agent except the
//! orchestrator runs as `claude -p` (stream-json) or `codex exec --json`, with no
//! terminal. This module turns those processes' stdout into [`SessionEvent`]s, a
//! window status, and milestone 6.5's conversation inputs, and builds their argv.
//!
//! It sits outside `run/` because M9's scouts, sub-planners and deciders reuse it.
//!
//! Every file here except `session.rs` and its `session/pipes.rs` (M8a.17) is pure
//! (decision 2): no filesystem, process, thread, async runtime or wall-clock access,
//! which decision 2's grep checks. `codex_sandbox` also holds the one Codex version the
//! startup probe recorded (a `OnceLock`, set once, never I/O).

pub mod argv;
pub mod claude_stream;
pub mod codex_guard;
pub mod codex_sandbox;
pub mod codex_stream;
pub mod conversation;
pub mod failure;
pub mod session;
pub mod status;

use proto::{AgentRole, Effort, RunRef, Runtime, TokenUsage};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Everything needed to launch (and re-launch) one headless session. Persisted in the
/// window's opaque `WindowRecord.run` (decision 28), so a restored window can be resumed
/// with every flag re-passed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeadlessSpec {
    pub runtime: Runtime,
    /// Empty: the runtime's configured default (Codex only, decision 23).
    pub model: String,
    pub effort: Effort,
    pub cwd: PathBuf,
    /// `--append-system-prompt` (Claude) or `-c developer_instructions` (Codex).
    pub instructions: String,
    pub mcp: Option<McpTarget>,
    pub allowed_tools: Vec<String>,
    /// `None`: the flag is omitted.
    pub claude_permission_mode: Option<String>,
    /// `--disallowedTools`, empty: omitted. A reviewer's `Edit,Write,NotebookEdit` under
    /// `dontAsk` (M8a.1: plan mode blocks the reviewer's allowed MCP call in `-p`).
    pub claude_disallowed_tools: Vec<String>,
    /// Workers only, when `[orchestrator] worker_sandbox` (decision 54).
    pub claude_sandbox: Option<ClaudeSandbox>,
    pub codex_sandbox: String,
    pub codex_writable_roots: Vec<PathBuf>,
    /// Read-only although inside a writable root (a worker's denied git entries); only
    /// a dialect that can express it receives it (`headless::codex_sandbox`).
    #[serde(default)]
    pub codex_read_only: Vec<PathBuf>,
    pub env: Vec<(String, String)>,
    #[serde(with = "claude_auth_serde")]
    pub claude_auth: config::ClaudeAuth,
    pub api_key_helper: Option<String>,
    /// `WindowInfo.run`.
    pub run_ref: Option<RunRef>,
    /// Final fix batch F2 (C-I1): a Codex session's checkout must hold exactly this
    /// `.codex` before each of its processes starts ([`codex_guard`]). `None`: not
    /// checked (Claude, or a Codex CLI that does not load project config).
    #[serde(default)]
    pub codex_config_guard: Option<codex_guard::CodexConfigGuard>,
    /// Milestone 8b decision 28: a worker-like session's output filter, as the
    /// `PreToolUse` hook on Claude and, on Codex, as `codex_filter_note` after its
    /// contract (milestone 9.5 decision 28; `None`: no filter).
    #[serde(default)]
    pub output_filter: Option<crate::output_filter::FilterHook>,
}

/// Decision 54's sandbox block. The worktree (the session's cwd) is writable by default;
/// this adds the parts of the repository's git common directory a commit needs
/// (`run::role_launch::worker_git_roots`, completed by the driver at launch; final fix
/// batch F1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeSandbox {
    pub writable_roots: Vec<PathBuf>,
    /// `filesystem.denyWrite`: the checkout's protected agent-config paths the session
    /// may not write (final fix batch F2 round 2, `role_launch::protected_write_denials`).
    #[serde(default)]
    pub deny_write: Vec<PathBuf>,
    /// Milestone 9.6 (task M9.6.8, task 6's review a): paths the session may not read,
    /// each denied to the read tools as a `permissions.deny` rule `Read(/<path>/**)`
    /// (`//` is the filesystem root in Claude's rule syntax) and to commands as
    /// `sandbox.filesystem.denyRead`. A brainstormer's is the run's design folder.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deny_read: Vec<PathBuf>,
}

/// Who the session's `anthrex mcp` server speaks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpTarget {
    pub role: AgentRole,
    /// Empty for a repository-level scout (M8b decision 15).
    pub run_id: String,
    #[serde(default)]
    pub task_id: Option<String>,
    /// A scout's id (`--scout`), M8b decision 15.
    #[serde(default)]
    pub scout_id: Option<String>,
    /// A sub-planner's epic (`--epic`), milestone 9 decision 31.
    #[serde(default)]
    pub epic: Option<String>,
    /// A chained orchestrator's chain (`--chain`), milestone 9.3 (KG §3.4). Not a wire
    /// type: it reaches the daemon as `proto::ToolCall.chain`.
    #[serde(default)]
    pub chain: Option<String>,
    /// A racer's lane (`--lane`), milestone 9.5. Not a wire type: it reaches the daemon
    /// as `proto::ToolCall.lane`.
    #[serde(default)]
    pub lane: Option<proto::RaceLane>,
    /// A design agent's label (`--agent-label`), milestone 9.6 ruling T1-O3: a
    /// brainstormer's `claude`, `codex`, `A` or `B`, a document reviewer's `<doc>-r<n>`.
    /// Only its session's argv names it (fake-agent keys its scripts by it); the daemon
    /// never reads it back, and it is no protocol field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_label: Option<String>,
}

/// Which session a launch starts or continues.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionArg {
    /// A new session. Claude takes `--session-id <uuid>`; Codex has no uuid (its session
    /// id is the first turn's `thread_id`).
    New {
        uuid: Option<String>,
    },
    Resume {
        session_id: String,
    },
}

/// One thing a headless session said, parsed from one line of its stdout (decision 27),
/// or reported by the session driver (`StderrLine`, `StartupFailed`, `ProcessExited`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SessionEvent {
    Init {
        session_id: String,
        model: Option<String>,
        mcp_ok: Option<bool>,
    },
    TurnStarted,
    /// Claude echoes of what was sent, when the CLI replays them.
    UserText {
        text: String,
    },
    AssistantText {
        text: String,
        parent: Option<String>,
    },
    /// The text of a top-level Claude `assistant` line that carries an API error
    /// category (`"error": "<category>"`): the synthetic message a failed API turn ends
    /// with (it also has `is_api_error_message`, which the parser does not check). Part
    /// of that failure, not the model's progress, so the engine does not see it as
    /// activity (M8a.24; decision 32's retry streak).
    ApiErrorText {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
        parent: Option<String>,
    },
    ToolResult {
        id: String,
        text: String,
        ok: bool,
        parent: Option<String>,
    },
    ApiRetry {
        error: String,
        attempt: u32,
        delay_ms: u64,
    },
    PermissionDenied {
        tool: String,
        reason: String,
    },
    Compacted,
    /// A Claude `result` line's `structured_output`: a decider's answer under
    /// `--json-schema` (M8b decision 16; M8b.1 item 1). Consumers other than the decider
    /// treat it as `Other`.
    StructuredOutput {
        value: serde_json::Value,
    },
    /// Recognised, nothing to act on (reasoning, thinking, rate-limit info, hook progress).
    Other {
        kind: String,
    },
    TurnEnded {
        outcome: TurnOutcome,
        usage: Option<TokenUsage>,
        denials: Vec<String>,
    },
    /// A runtime's own error notice (Codex's top-level `error` line), nothing to act on.
    /// The parsers are pure, so the session driver logs it and keeps it in the window's
    /// last-lines ring.
    Diagnostic {
        text: String,
    },
    /// The first [`UNKNOWN_LINE_CHARS`] characters of a line that did not parse.
    Unknown {
        line: String,
    },
    StderrLine {
        line: String,
    },
    /// From the driver, just before `ProcessExited`: the process failed (a non-zero
    /// code, or a signal) before its turn said anything, and its last stderr lines say
    /// why ([`failure::startup_failure`]; 2026-10-06, Claude without its sandbox).
    StartupFailed {
        reason: String,
    },
    /// From the driver, before any other event of the process `pid` (M8a.17, the
    /// ordering the engine relies on: see `run::engine::AgentSignal`).
    ProcessStarted {
        pid: u32,
    },
    ProcessExited {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnOutcome {
    Completed,
    Failed { error: String, kind: FailureKind },
    Interrupted,
}

/// Why a turn failed. `SandboxUnavailable` is M8a.1 item 4b's text, in a failed result.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FailureKind {
    RateLimit,
    Authentication,
    Billing,
    SandboxUnavailable,
    /// Ruling F-1: a deterministic client error (HTTP 400, 401, 403, 404 or 422, or an
    /// `invalid_request_error`, `not_found_error` or `permission_error`), which a
    /// continue cannot fix.
    ClientError,
    Other,
}

/// How much of an unparsed line `SessionEvent::Unknown` keeps.
pub const UNKNOWN_LINE_CHARS: usize = 300;

/// `SessionEvent::Unknown` for `line`: its first [`UNKNOWN_LINE_CHARS`] characters.
pub(crate) fn unknown(line: &str) -> SessionEvent {
    SessionEvent::Unknown {
        line: line.chars().take(UNKNOWN_LINE_CHARS).collect(),
    }
}

/// A tool result's text cut to `proto::conversation::TOOL_RESULT_SUMMARY_MAX` bytes on a
/// character boundary, the bound `anthrex hook` puts on a real hook's result.
pub(crate) fn bounded_text(text: &str) -> String {
    proto::conversation::truncate_to_char_boundary(
        text,
        proto::conversation::TOOL_RESULT_SUMMARY_MAX,
    )
}

/// Final fix batch F2 (review C, M2; round 2, N2): the inherited API credentials (and,
/// by the user's rulings of 2026-09-29, `ANTHROPIC_BASE_URL` and `OPENAI_BASE_URL`) a
/// session's process must not see. Every session loses the OpenAI and Codex ones
/// (anthrex's Codex sessions use the user's `codex login`), and the Anthropic ones
/// unless it is a Claude session with `auth = "api_key"`, which authenticates with them
/// (decision 50); `claude -p` would otherwise prefer a key the daemon inherited over the
/// user's login.
pub fn credential_scrub(spec: &HeadlessSpec) -> Vec<&'static str> {
    credential_scrub_for(spec.runtime, spec.claude_auth)
}

/// [`credential_scrub`] for a runtime and auth without a spec: a decider (M8b decision
/// 16) passes `(runtime, ClaudeAuth::Login)`, so every API credential is removed.
pub fn credential_scrub_for(runtime: Runtime, auth: config::ClaudeAuth) -> Vec<&'static str> {
    use config::reserved_env::{API_CREDENTIALS, OPENAI_CREDENTIALS};
    let mut names = OPENAI_CREDENTIALS.to_vec();
    if !(runtime == Runtime::Claude && auth == config::ClaudeAuth::ApiKey) {
        names.extend(API_CREDENTIALS);
    }
    names
}

/// The variables a session's process gets on top of what it inherits: `env` (the
/// caller's), then, for a Claude session, `ENABLE_TOOL_SEARCH=false`
/// ([`config::reserved_env::CLAUDE_TOOL_SEARCH`]) last and only once, so neither the
/// daemon's own environment nor `env` can turn `ToolSearch` back on. Every headless
/// session (worker, reviewer, scout, decider) starts through
/// [`session::HeadlessHandle::spawn`], which applies this; the user's own PTY windows
/// never do.
pub fn session_vars(runtime: Runtime, env: &[(String, String)]) -> Vec<(String, String)> {
    let (name, value) = config::reserved_env::CLAUDE_TOOL_SEARCH;
    let mut vars = env.to_vec();
    if runtime == Runtime::Claude {
        vars.retain(|(key, _)| key != name);
        vars.push((name.to_string(), value.to_string()));
    }
    vars
}

/// `config::ClaudeAuth` has no serde derive (the config crate does not depend on serde),
/// so the spec spells it as decision 50's own config strings.
mod claude_auth_serde {
    use config::ClaudeAuth;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(auth: &ClaudeAuth, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(match auth {
            ClaudeAuth::Login => "login",
            ClaudeAuth::ApiKey => "api_key",
        })
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<ClaudeAuth, D::Error> {
        match String::deserialize(d)?.as_str() {
            "login" => Ok(ClaudeAuth::Login),
            "api_key" => Ok(ClaudeAuth::ApiKey),
            other => Err(serde::de::Error::custom(format!(
                "unknown claude auth '{other}' (expected login or api_key)"
            ))),
        }
    }
}

/// Milestone 9.6 rulings T8-4 and T8-7: a design agent's session is not saved, so it is
/// never resumed; a resume, or a Codex design agent's next turn, is refused before
/// anything changes.
pub fn never_resumed(id: u32, spec: &HeadlessSpec) -> anyhow::Result<()> {
    if crate::headless::argv::unsaved(spec) {
        anyhow::bail!("window {id} is a design agent's session, which is never resumed");
    }
    Ok(())
}

#[cfg(test)]
mod test_support;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
