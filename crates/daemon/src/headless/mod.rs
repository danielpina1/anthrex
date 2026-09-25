//! The headless session layer (decisions 24 to 27 and 49 to 54): every agent except the
//! orchestrator runs as `claude -p` (stream-json) or `codex exec --json`, with no
//! terminal. This module turns those processes' stdout into [`SessionEvent`]s, a
//! window status, and milestone 6.5's conversation inputs, and builds their argv.
//!
//! It sits outside `run/` because M9's scouts, sub-planners and deciders reuse it.
//!
//! Every file here except `session.rs` and its `session/pipes.rs` (M8a.17) is pure
//! (decision 2): no filesystem, process, thread, async runtime or wall-clock access,
//! which decision 2's grep checks.

pub mod argv;
pub mod claude_stream;
pub mod codex_stream;
pub mod conversation;
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
    pub env: Vec<(String, String)>,
    #[serde(with = "claude_auth_serde")]
    pub claude_auth: config::ClaudeAuth,
    pub api_key_helper: Option<String>,
    /// `WindowInfo.run`.
    pub run_ref: Option<RunRef>,
}

/// Decision 54's sandbox block. The worktree (the session's cwd) is writable by default;
/// this adds the parts of the repository's git common directory a commit needs
/// (`run::role_launch::worker_git_roots`, completed by the driver at launch; final fix
/// batch F1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClaudeSandbox {
    pub writable_roots: Vec<PathBuf>,
}

/// Who the session's `anthrex mcp` server speaks for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct McpTarget {
    pub role: AgentRole,
    pub run_id: String,
    pub task_id: Option<String>,
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
/// or reported by the session driver (`StderrLine`, `ProcessExited`).
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

/// Final fix batch F2 (review C, M2): the inherited API credentials a session's process
/// must not see. Every session loses them except a Claude session with `auth =
/// "api_key"`, which authenticates with them (decision 50); `claude -p` would otherwise
/// prefer a key the daemon inherited over the user's login, and a Codex session never
/// uses Anthropic's.
pub fn credential_scrub(spec: &HeadlessSpec) -> &'static [&'static str] {
    if spec.runtime == Runtime::Claude && spec.claude_auth == config::ClaudeAuth::ApiKey {
        &[]
    } else {
        config::reserved_env::API_CREDENTIALS
    }
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

#[cfg(test)]
mod test_support;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
