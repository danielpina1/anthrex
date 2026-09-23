//! `codex exec --json` lines to [`SessionEvent`]s (decision 27, the Interfaces table).
//! Pure and stateless: every Codex turn is its own process.
//!
//! Where M8a.1's recordings (`crates/daemon/tests/fixtures/headless/codex-0.155.0-*`)
//! differ from the working shapes, the recordings win:
//!
//! - `turn.completed.usage.input_tokens` includes `cached_input_tokens`, so
//!   `TokenUsage.input` is the difference and `cache_read` the cached part (decision 40
//!   bills cache reads apart). `cache_write_input_tokens` is `cache_write`.
//! - `item.type == "error"` items are notices ("Skill descriptions were shortened …",
//!   "Model metadata … not found"), not failures: `Other { kind: "item/error" }`.
//! - `turn.failed.error.message` is the API's JSON error as a string. The failure text
//!   is its inner `error.message` when it parses, and its `status` counts towards the
//!   rate-limit rule.
//! - A command that exits non-zero is not reported at all by Codex 0.155, and an
//!   interrupted turn ends with no `turn.*` line; the driver's `ProcessExited` covers it.

use super::{FailureKind, SessionEvent, TurnOutcome, bounded_text, unknown};
use proto::TokenUsage;
use serde_json::{Map, Value, json};

/// Zero or more events for one stdout line. Never panics: a line that is not a JSON
/// object, or whose type is not recognised, is `Unknown`.
pub fn parse_line(line: &str) -> Vec<SessionEvent> {
    let Ok(Value::Object(object)) = serde_json::from_str::<Value>(line) else {
        return vec![unknown(line)];
    };
    match object.get("type").and_then(Value::as_str).unwrap_or("") {
        "thread.started" => vec![SessionEvent::Init {
            session_id: string(&object, "thread_id").unwrap_or_default(),
            model: None,
            mcp_ok: None,
        }],
        "turn.started" => vec![SessionEvent::TurnStarted],
        "turn.completed" => vec![SessionEvent::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: object.get("usage").and_then(usage),
            denials: Vec::new(),
        }],
        "turn.failed" => {
            let message = object
                .get("error")
                .and_then(|e| e.get("message"))
                .and_then(Value::as_str)
                .unwrap_or("turn failed");
            let (error, kind) = classify(message);
            vec![SessionEvent::TurnEnded {
                outcome: TurnOutcome::Failed { error, kind },
                usage: None,
                denials: Vec::new(),
            }]
        }
        "error" => {
            let message = string(&object, "message").unwrap_or_default();
            tracing::warn!(
                message = %message.chars().take(super::UNKNOWN_LINE_CHARS).collect::<String>(),
                "codex reported an error"
            );
            vec![SessionEvent::Other {
                kind: "error".into(),
            }]
        }
        phase @ ("item.started" | "item.updated" | "item.completed") => {
            match object.get("item").and_then(Value::as_object) {
                Some(item) => item_events(phase, item),
                None => vec![unknown(line)],
            }
        }
        _ => vec![unknown(line)],
    }
}

fn item_events(phase: &str, item: &Map<String, Value>) -> Vec<SessionEvent> {
    let id = string(item, "id").unwrap_or_default();
    let kind = item.get("type").and_then(Value::as_str).unwrap_or("");
    let started = phase == "item.started";
    let completed = phase == "item.completed";
    let tool_use = |name: String, input: Value| SessionEvent::ToolUse {
        id: id.clone(),
        name,
        input,
        parent: None,
    };
    let tool_result = |text: String, ok: bool| SessionEvent::ToolResult {
        id: id.clone(),
        text: bounded_text(&text),
        ok,
        parent: None,
    };
    match kind {
        "agent_message" if completed => vec![SessionEvent::AssistantText {
            text: string(item, "text").unwrap_or_default(),
            parent: None,
        }],
        "command_execution" if started => vec![tool_use(
            "Bash".into(),
            json!({"command": string(item, "command").unwrap_or_default()}),
        )],
        "command_execution" if completed => vec![tool_result(
            string(item, "aggregated_output").unwrap_or_default(),
            item.get("exit_code").and_then(Value::as_i64) == Some(0),
        )],
        "mcp_tool_call" if started => {
            let server = string(item, "server").unwrap_or_default();
            let tool = string(item, "tool").unwrap_or_default();
            vec![tool_use(
                format!("mcp__{server}__{tool}"),
                item.get("arguments").cloned().unwrap_or(Value::Null),
            )]
        }
        "mcp_tool_call" if completed => {
            let error = item.get("error").filter(|e| !e.is_null()).map(|e| {
                match e.get("message").and_then(Value::as_str) {
                    Some(message) => message.to_owned(),
                    None => e.to_string(),
                }
            });
            let failed = error.is_some() || string(item, "status").as_deref() == Some("failed");
            let text = error.unwrap_or_else(|| mcp_result_text(item.get("result")));
            vec![tool_result(text, !failed)]
        }
        "file_change" if completed => vec![
            tool_use(
                "apply_patch".into(),
                json!({"changes": item.get("changes").cloned().unwrap_or(Value::Null)}),
            ),
            tool_result(
                String::new(),
                string(item, "status").as_deref() == Some("completed"),
            ),
        ],
        "web_search" if completed => vec![
            tool_use(
                "WebSearch".into(),
                json!({"query": string(item, "query").unwrap_or_default()}),
            ),
            tool_result(String::new(), true),
        ],
        "" => vec![SessionEvent::Other {
            kind: "item".into(),
        }],
        other => vec![SessionEvent::Other {
            kind: format!("item/{other}"),
        }],
    }
}

/// An MCP call's result: its text content parts joined, else its JSON.
fn mcp_result_text(result: Option<&Value>) -> String {
    let Some(result) = result.filter(|r| !r.is_null()) else {
        return String::new();
    };
    let parts: Vec<&str> = result
        .get("content")
        .and_then(Value::as_array)
        .map(|c| {
            c.iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect()
        })
        .unwrap_or_default();
    if parts.is_empty() {
        result.to_string()
    } else {
        parts.join("\n")
    }
}

/// M8a.1 item 7's rule: `RateLimit` when the message, or its JSON `status`, matches
/// `429`, `rate limit`, `usage limit` or `too many requests`, ignoring case; else `Other`.
/// The text is the JSON error's inner message when the message is one.
fn classify(message: &str) -> (String, FailureKind) {
    let parsed: Option<Value> = serde_json::from_str(message).ok();
    let status = parsed
        .as_ref()
        .and_then(|v| v.get("status"))
        .and_then(Value::as_u64);
    let inner = parsed
        .as_ref()
        .and_then(|v| v.get("error"))
        .and_then(|e| e.get("message"))
        .and_then(Value::as_str);
    let lower = message.to_lowercase();
    let rate_limited = status == Some(429)
        || ["429", "rate limit", "usage limit", "too many requests"]
            .iter()
            .any(|p| lower.contains(p));
    let kind = if rate_limited {
        FailureKind::RateLimit
    } else {
        FailureKind::Other
    };
    (inner.unwrap_or(message).to_owned(), kind)
}

fn string(object: &Map<String, Value>, key: &str) -> Option<String> {
    object.get(key).and_then(Value::as_str).map(str::to_owned)
}

fn usage(value: &Value) -> Option<TokenUsage> {
    let object = value.as_object()?;
    let n = |key: &str| object.get(key).and_then(Value::as_u64).unwrap_or(0);
    let cached = n("cached_input_tokens");
    Some(TokenUsage {
        input: n("input_tokens").saturating_sub(cached),
        output: n("output_tokens"),
        cache_read: cached,
        cache_write: n("cache_write_input_tokens"),
    })
}

#[cfg(test)]
#[path = "codex_stream_tests.rs"]
mod tests;
