//! Claude Code's `-p --output-format stream-json` lines to [`SessionEvent`]s (decision 27,
//! the Interfaces table), and the two stdin envelopes M8a.1 pinned. Pure.
//!
//! Where M8a.1's recordings (`crates/daemon/tests/fixtures/headless/claude-2.1.278-*`)
//! differ from the working shapes, the recordings win:
//!
//! - `system/init` starts every turn, not only the session, so it yields `Init` and
//!   `TurnStarted` each time; a turn Claude Code starts by itself (a background
//!   sub-agent finishing) is then an ordinary turn.
//! - A failed API turn's category is on the synthetic `assistant` line before the
//!   `result` (`"error": "<category>"`), so the parser carries it to that `result`.
//! - A turn is interrupted when its `result` has `subtype: "error_during_execution"` and
//!   `terminal_reason` `aborted_tools` or `aborted_streaming`.
//! - `result.usage` is per turn and is passed through.
//! - The line types the stream meta lists as `unmodelled` (hook progress, thinking-token
//!   counts, task progress, `rate_limit_event`, `control_response`, …) are recognised
//!   and yield `Other`, so they never crowd real diagnostics out of the window's
//!   last-lines ring.

use super::{FailureKind, SessionEvent, TurnOutcome, bounded_text, unknown};
use proto::TokenUsage;
use serde::Serialize;
use serde_json::Value;

/// The parser for one Claude session's stdout. Start each session with
/// `ClaudeStream::default()` and pass it every line in order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClaudeStream {
    /// The category of the API error the current turn has reported, taken by its
    /// `result`.
    pending_failure: Option<FailureKind>,
}

impl ClaudeStream {
    /// Zero or more events for one stdout line. Never panics: a line that is not a JSON
    /// object, or whose type is not recognised, is `Unknown`.
    pub fn parse_line(&mut self, line: &str) -> Vec<SessionEvent> {
        let Ok(Value::Object(object)) = serde_json::from_str::<Value>(line) else {
            return vec![unknown(line)];
        };
        let kind = object.get("type").and_then(Value::as_str).unwrap_or("");
        let subtype = object.get("subtype").and_then(Value::as_str).unwrap_or("");
        let parent = string(&object, "parent_tool_use_id");
        match (kind, subtype) {
            ("system", "init") => vec![
                SessionEvent::Init {
                    session_id: string(&object, "session_id").unwrap_or_default(),
                    model: string(&object, "model"),
                    mcp_ok: mcp_ok(object.get("mcp_servers")),
                },
                SessionEvent::TurnStarted,
            ],
            ("system", "api_retry") => vec![SessionEvent::ApiRetry {
                error: string(&object, "error").unwrap_or_default(),
                attempt: number(&object, "attempt")
                    .and_then(|n| u32::try_from(n).ok())
                    .unwrap_or(0),
                delay_ms: number(&object, "retry_delay_ms").unwrap_or(0),
            }],
            ("system", "permission_denied") => vec![SessionEvent::PermissionDenied {
                tool: string(&object, "tool_name").unwrap_or_default(),
                reason: string(&object, "decision_reason")
                    .or_else(|| string(&object, "message"))
                    .unwrap_or_default(),
            }],
            ("system", "compact_boundary") => vec![SessionEvent::Compacted],
            ("system", other) if RECOGNISED_SYSTEM.contains(&other) => vec![SessionEvent::Other {
                kind: format!("system/{other}"),
            }],
            ("assistant", _) => {
                if let Some(category) = object.get("error").and_then(Value::as_str) {
                    self.pending_failure = Some(category_kind(category));
                }
                blocks(&object)
                    .iter()
                    .filter_map(|block| assistant_block(block, &parent))
                    .collect()
            }
            ("user", _) => blocks(&object)
                .iter()
                .filter_map(|block| user_block(block, &parent))
                .collect(),
            ("result", _) => vec![self.result(&object)],
            (other, _) if RECOGNISED_TYPES.contains(&other) => {
                vec![SessionEvent::Other { kind: other.into() }]
            }
            _ => vec![unknown(line)],
        }
    }

    fn result(&mut self, object: &serde_json::Map<String, Value>) -> SessionEvent {
        let pending = self.pending_failure.take();
        let is_error = object.get("is_error").and_then(Value::as_bool) == Some(true);
        let subtype = object.get("subtype").and_then(Value::as_str).unwrap_or("");
        let terminal = object.get("terminal_reason").and_then(Value::as_str);
        let outcome = if subtype == "error_during_execution"
            && matches!(terminal, Some("aborted_tools" | "aborted_streaming"))
        {
            TurnOutcome::Interrupted
        } else if subtype == "success" && !is_error {
            TurnOutcome::Completed
        } else {
            let error = failure_text(object, subtype);
            let kind = if error.contains(SANDBOX_UNAVAILABLE) {
                FailureKind::SandboxUnavailable
            } else {
                pending.unwrap_or(FailureKind::Other)
            };
            TurnOutcome::Failed { error, kind }
        };
        SessionEvent::TurnEnded {
            outcome,
            usage: object.get("usage").and_then(usage),
            denials: object
                .get("permission_denials")
                .and_then(Value::as_array)
                .map(|denials| {
                    denials
                        .iter()
                        .filter_map(|d| d.get("tool_name").and_then(Value::as_str))
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default(),
        }
    }
}

/// M8a.1 item 4b: the start of the error Claude Code prints when `failIfUnavailable` is
/// set and the sandbox cannot start.
const SANDBOX_UNAVAILABLE: &str = "sandbox required but unavailable";

/// `system` subtypes seen in M8a.1's recordings that carry nothing to act on.
const RECOGNISED_SYSTEM: &[&str] = &[
    "hook_started",
    "hook_response",
    "thinking_tokens",
    "task_started",
    "task_updated",
    "task_notification",
    "background_tasks_changed",
    "vcs_state_changed",
];

/// Top-level types seen in M8a.1's recordings that carry nothing to act on.
const RECOGNISED_TYPES: &[&str] = &["rate_limit_event", "control_response"];

/// The user message that starts one turn, exactly M8a.1's accepted envelope (item 2),
/// in its key order. `session_id` is omitted when unknown.
pub fn user_message(text: &str, session_id: Option<&str>) -> String {
    #[derive(Serialize)]
    struct Envelope<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        message: Message<'a>,
        parent_tool_use_id: Option<&'a str>,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<&'a str>,
    }
    #[derive(Serialize)]
    struct Message<'a> {
        role: &'static str,
        content: [Content<'a>; 1],
    }
    #[derive(Serialize)]
    struct Content<'a> {
        #[serde(rename = "type")]
        kind: &'static str,
        text: &'a str,
    }
    serde_json::to_string(&Envelope {
        kind: "user",
        message: Message {
            role: "user",
            content: [Content { kind: "text", text }],
        },
        parent_tool_use_id: None,
        session_id,
    })
    .expect("serializing strings cannot fail")
}

/// The interrupt control request M8a.1 found accepted (item 2); `request_id` is a string.
pub fn interrupt_request(request_id: u64) -> String {
    // In the recorded key order; the id is digits only, so it needs no escaping.
    format!(
        r#"{{"type":"control_request","request_id":"{request_id}","request":{{"subtype":"interrupt"}}}}"#
    )
}

fn string(object: &serde_json::Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn number(object: &serde_json::Map<String, Value>, key: &str) -> Option<u64> {
    object.get(key).and_then(Value::as_u64)
}

fn blocks(object: &serde_json::Map<String, Value>) -> Vec<Value> {
    object
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

/// `Some(connected)` for the `anthrex` server; `None` when the session has none.
fn mcp_ok(servers: Option<&Value>) -> Option<bool> {
    servers?
        .as_array()?
        .iter()
        .find(|s| s.get("name").and_then(Value::as_str) == Some("anthrex"))
        .map(|s| s.get("status").and_then(Value::as_str) == Some("connected"))
}

fn assistant_block(block: &Value, parent: &Option<String>) -> Option<SessionEvent> {
    match block.get("type")?.as_str()? {
        "text" => Some(SessionEvent::AssistantText {
            text: block.get("text")?.as_str()?.to_owned(),
            parent: parent.clone(),
        }),
        "tool_use" => Some(SessionEvent::ToolUse {
            id: block.get("id")?.as_str()?.to_owned(),
            name: block.get("name")?.as_str()?.to_owned(),
            input: block.get("input").cloned().unwrap_or(Value::Null),
            parent: parent.clone(),
        }),
        other => Some(SessionEvent::Other { kind: other.into() }),
    }
}

fn user_block(block: &Value, parent: &Option<String>) -> Option<SessionEvent> {
    match block.get("type")?.as_str()? {
        "tool_result" => Some(SessionEvent::ToolResult {
            id: block.get("tool_use_id")?.as_str()?.to_owned(),
            text: bounded_text(&content_text(block.get("content"))),
            ok: block.get("is_error").and_then(Value::as_bool) != Some(true),
            parent: parent.clone(),
        }),
        "text" => Some(SessionEvent::UserText {
            text: block.get("text")?.as_str()?.to_owned(),
        }),
        other => Some(SessionEvent::Other { kind: other.into() }),
    }
}

/// A `tool_result`'s content: the string itself, or its text parts joined by newlines.
fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// The error categories decision 32 acts on; every other category is `Other`.
fn category_kind(category: &str) -> FailureKind {
    match category {
        "rate_limit" => FailureKind::RateLimit,
        "authentication_failed" => FailureKind::Authentication,
        "billing_error" => FailureKind::Billing,
        _ => FailureKind::Other,
    }
}

/// A failed turn's text: `result`, else its `errors` joined, else its subtype.
fn failure_text(object: &serde_json::Map<String, Value>, subtype: &str) -> String {
    if let Some(text) = object
        .get("result")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return text.to_owned();
    }
    let errors: Vec<&str> = object
        .get("errors")
        .and_then(Value::as_array)
        .map(|e| e.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if errors.is_empty() {
        subtype.to_owned()
    } else {
        errors.join("; ")
    }
}

fn usage(value: &Value) -> Option<TokenUsage> {
    let object = value.as_object()?;
    let n = |key: &str| object.get(key).and_then(Value::as_u64).unwrap_or(0);
    Some(TokenUsage {
        input: n("input_tokens"),
        output: n("output_tokens"),
        cache_read: n("cache_read_input_tokens"),
        cache_write: n("cache_creation_input_tokens"),
    })
}

#[cfg(test)]
#[path = "claude_stream_tests.rs"]
mod tests;
