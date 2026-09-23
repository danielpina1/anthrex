//! Claude Code's transcript: `~/.claude/projects/<cwd>/<session>.jsonl`, where `<cwd>`
//! has every `/` and `.` replaced by `-`.
//!
//! Modelled on the capture in `tests/fixtures/transcripts/claude-2.1.278.jsonl`. Each line
//! is one envelope with a top-level `type` and a camelCase `sessionId`. Only three shapes
//! carry conversation:
//!
//! - `type:"user"` with a string `origin.kind`: a prompt. `"human"` is one a person
//!   typed. Claude 2.1.278 also injects prompts of its own that fire `UserPromptSubmit`
//!   (a background sub-agent's `"peer"` hand-back and its `"task-notification"`; see
//!   `claude-2.1.278-background-agent.jsonl`), so every kind opens a turn.
//! - `type:"assistant"`: one line per content block, `text` or `tool_use`.
//! - `type:"user"` whose `message.content` is a list of `tool_result` blocks: tool
//!   output. Several results may share one line.
//!
//! Everything else (`last-prompt`, `attachment`, `system`, `mode`, ...) is ignored.

use serde_json::{Map, Value};

use super::{Cursor, Record, TranscriptParser, Version, object};

pub(super) struct ClaudeParser;

const V1: Version = Version(1);

impl TranscriptParser for ClaudeParser {
    fn runtime(&self) -> proto::Runtime {
        proto::Runtime::Claude
    }

    /// The discriminator is a string `type` beside a top-level camelCase `sessionId`, on a
    /// line with no `payload`. Codex's envelope is `{timestamp, type, payload}` and never
    /// has a top-level `sessionId`. The first line itself is a weak signal — the capture
    /// starts with `last-prompt`, a bookkeeping record — so detect keys on the envelope
    /// every Claude record shares rather than on one record type, which would degrade a
    /// file the moment Claude reorders or adds bookkeeping records. The one exception the
    /// capture shows is `file-history-snapshot`, which has no `sessionId`; it is accepted
    /// by its `messageId` and `snapshot` object.
    fn detect(&self, first_line: &str) -> Option<Version> {
        let map = object(first_line)?;
        let kind = map.get("type")?.as_str()?;
        if map.contains_key("payload") {
            return None;
        }
        let envelope = map.get("sessionId").is_some_and(Value::is_string);
        let snapshot = kind == "file-history-snapshot"
            && map.get("messageId").is_some_and(Value::is_string)
            && map.get("snapshot").is_some_and(Value::is_object);
        (envelope || snapshot).then_some(V1)
    }

    fn record(&self, version: Version, line: &str, cursor: &mut Cursor) -> Vec<Record> {
        if version != V1 {
            return Vec::new();
        }
        let Some(map) = object(line) else {
            return Vec::new();
        };
        // A sub-agent's own exchange is not this conversation's turns.
        if map.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            return Vec::new();
        }
        let session_id = map
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let Some(content) = map.get("message").and_then(|m| m.get("content")) else {
            return Vec::new();
        };
        match map.get("type").and_then(Value::as_str) {
            Some("user") => user(&map, content, session_id, cursor),
            Some("assistant") => assistant(content, session_id, cursor),
            _ => Vec::new(),
        }
    }

    /// A `user` or `assistant` line is the only kind that carries conversation, and it
    /// always has a `message.content` that is a string or a list of blocks; one without
    /// is broken, where a bookkeeping line (`last-prompt`, `attachment`, ...) is not.
    fn malformed(&self, _version: Version, line: &str) -> bool {
        let Some(map) = object(line) else {
            return true;
        };
        match map.get("type").and_then(Value::as_str) {
            Some("user" | "assistant") => !matches!(
                map.get("message").and_then(|m| m.get("content")),
                Some(Value::String(_) | Value::Array(_))
            ),
            _ => false,
        }
    }
}

fn user(
    map: &Map<String, Value>,
    content: &Value,
    session_id: Option<String>,
    cursor: &mut Cursor,
) -> Vec<Record> {
    if let Some(kind) = origin_kind(map) {
        let human = kind == "human";
        // A compact summary never opens a turn. `isMeta` marks something Claude wrote
        // under the human origin (review F3); an injected origin carries it too (the
        // peer hand-back does) and still fired `UserPromptSubmit`, so it opens a turn.
        if flag(map, "isCompactSummary") || (human && flag(map, "isMeta")) {
            return Vec::new();
        }
        // A prompt line opens a turn whatever its content's shape, so a prompt with no
        // readable text can cost its own `UserText` but never shift a later ordinal
        // (review F2).
        let ordinal = cursor.next_prompt();
        let text = match content {
            Value::String(text) => Some(text.clone()),
            Value::Array(blocks) => joined_text(blocks),
            _ => None,
        };
        return text
            .map(|text| Record::UserText {
                session_id,
                ordinal,
                text,
                human,
            })
            .into_iter()
            .collect();
    }
    match content {
        Value::Array(blocks) => blocks.iter().filter_map(tool_result).collect(),
        _ => Vec::new(),
    }
}

fn flag(map: &Map<String, Value>, key: &str) -> bool {
    map.get(key).and_then(Value::as_bool) == Some(true)
}

/// The `text` of every `text` block, joined by newlines, or `None` when there is none.
fn joined_text(blocks: &[Value]) -> Option<String> {
    let texts: Vec<&str> = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    (!texts.is_empty()).then(|| texts.join("\n"))
}

/// The capture marks a typed prompt three ways (`origin.kind`, `promptSource`,
/// `turnOrigin`); `origin.kind` is the one that names its source outright, and the
/// injected prompts carry it too. A user message without it is something Claude wrote
/// itself (a tool result, say), not a turn.
fn origin_kind(map: &Map<String, Value>) -> Option<&str> {
    map.get("origin")?.get("kind")?.as_str()
}

fn tool_result(block: &Value) -> Option<Record> {
    if block.get("type")?.as_str()? != "tool_result" {
        return None;
    }
    let tool_use_id = block.get("tool_use_id")?.as_str()?.to_owned();
    let detail = match block.get("content") {
        Some(Value::String(text)) => Some(text.clone()),
        Some(Value::Array(parts)) => {
            let texts: Vec<&str> = parts
                .iter()
                .filter_map(|p| p.get("text").and_then(Value::as_str))
                .collect();
            (!texts.is_empty()).then(|| texts.join("\n"))
        }
        _ => None,
    };
    let ok = block.get("is_error").and_then(Value::as_bool).map(|e| !e);
    if detail.is_none() && ok.is_none() {
        return None;
    }
    Some(Record::ToolDetail {
        tool_use_id,
        input: None,
        detail,
        ok,
    })
}

fn assistant(content: &Value, session_id: Option<String>, cursor: &Cursor) -> Vec<Record> {
    let Some(blocks) = content.as_array() else {
        return Vec::new();
    };
    let turn = cursor.current_turn();
    blocks
        .iter()
        .filter_map(|block| match block.get("type")?.as_str()? {
            "text" => Some(Record::AssistantText {
                session_id: session_id.clone(),
                ordinal: turn?,
                text: block.get("text")?.as_str()?.to_owned(),
            }),
            "tool_use" => Some(Record::ToolDetail {
                tool_use_id: block.get("id")?.as_str()?.to_owned(),
                // The id alone is what joins a call to the timeline (review F10).
                input: block.get("input").cloned(),
                detail: None,
                ok: None,
            }),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
#[path = "claude_tests.rs"]
mod tests;
