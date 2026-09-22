//! Codex's rollout transcript: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`.
//!
//! Modelled on the capture in `tests/fixtures/transcripts/codex-0.155.0.jsonl`. Each line
//! is `{timestamp, ordinal, type, payload}`. Codex writes the conversation twice: once as
//! `response_item` lines and again as `event_msg`/`item_completed` lines. This parser
//! reads only the `response_item` stream, so nothing is counted twice:
//!
//! - `message` with `role:"user"`: a real prompt only for the content items whose
//!   parallel `content_item_kinds` entry is `"user.text"`. The AGENTS.md and environment
//!   context Codex injects is also `role:"user"`, with other kinds, and is never a prompt.
//! - `message` with `role:"assistant"`: prose, one record per `output_text` item.
//! - `custom_tool_call` and `custom_tool_call_output`, paired by `call_id`.
//!
//! The session id is stated once, by the first line's `session_meta`, and carried on the
//! cursor.

use serde_json::Value;

use super::{Cursor, Record, TranscriptParser, Version, object};

pub(super) struct CodexParser;

const V1: Version = Version(1);

impl TranscriptParser for CodexParser {
    fn runtime(&self) -> proto::Runtime {
        proto::Runtime::Codex
    }

    /// The discriminator is `type:"session_meta"` with a string `payload.id` and
    /// `payload.cli_version`. No Claude record has a `payload`, let alone this one.
    fn detect(&self, first_line: &str) -> Option<Version> {
        let map = object(first_line)?;
        let payload = map.get("payload")?;
        let meta = map.get("type")?.as_str()? == "session_meta"
            && payload.get("id").is_some_and(Value::is_string)
            && payload.get("cli_version").is_some_and(Value::is_string);
        meta.then_some(V1)
    }

    fn record(&self, version: Version, line: &str, cursor: &mut Cursor) -> Vec<Record> {
        if version != V1 {
            return Vec::new();
        }
        let Some(map) = object(line) else {
            return Vec::new();
        };
        let Some(payload) = map.get("payload") else {
            return Vec::new();
        };
        match map.get("type").and_then(Value::as_str) {
            Some("session_meta") => {
                if let Some(id) = payload.get("id").and_then(Value::as_str) {
                    cursor.session_id = Some(id.to_owned());
                }
                Vec::new()
            }
            Some("response_item") => response_item(payload, cursor),
            _ => Vec::new(),
        }
    }
}

fn response_item(payload: &Value, cursor: &mut Cursor) -> Vec<Record> {
    match payload.get("type").and_then(Value::as_str) {
        Some("message") => message(payload, cursor),
        Some("custom_tool_call") => call(payload).into_iter().collect(),
        Some("custom_tool_call_output") => output(payload).into_iter().collect(),
        _ => Vec::new(),
    }
}

fn message(payload: &Value, cursor: &mut Cursor) -> Vec<Record> {
    let Some(items) = payload.get("content").and_then(Value::as_array) else {
        return Vec::new();
    };
    match payload.get("role").and_then(Value::as_str) {
        Some("user") => {
            let kinds = payload
                .get("internal_chat_message_metadata_passthrough")
                .and_then(|m| m.get("content_item_kinds"))
                .and_then(Value::as_array);
            let Some(kinds) = kinds else {
                return Vec::new();
            };
            let texts: Vec<&str> = items
                .iter()
                .zip(kinds)
                .filter(|(_, kind)| kind.as_str() == Some("user.text"))
                .filter_map(|(item, _)| text_of(item, "input_text"))
                .collect();
            if texts.is_empty() {
                return Vec::new();
            }
            vec![Record::UserText {
                session_id: cursor.session_id.clone(),
                ordinal: cursor.next_prompt(),
                text: texts.join("\n"),
            }]
        }
        Some("assistant") => {
            let Some(ordinal) = cursor.current_turn() else {
                return Vec::new();
            };
            items
                .iter()
                .filter_map(|item| text_of(item, "output_text"))
                .map(|text| Record::AssistantText {
                    session_id: cursor.session_id.clone(),
                    ordinal,
                    text: text.to_owned(),
                })
                .collect()
        }
        _ => Vec::new(),
    }
}

fn text_of<'a>(item: &'a Value, kind: &str) -> Option<&'a str> {
    if item.get("type")?.as_str()? != kind {
        return None;
    }
    item.get("text")?.as_str()
}

/// Codex's tool input is a JS-like script string, not JSON, so it is carried verbatim
/// as a `Value::String`.
fn call(payload: &Value) -> Option<Record> {
    Some(Record::ToolDetail {
        tool_use_id: payload.get("call_id")?.as_str()?.to_owned(),
        input: Some(Value::String(payload.get("input")?.as_str()?.to_owned())),
        detail: None,
        ok: None,
    })
}

/// The capture's output is a list of `input_text` items: a "Script completed" banner,
/// then a JSON object carrying `exit_code` and the command's `output`. When that object
/// is present, `detail` is its `output` and `ok` is `exit_code == 0`. Without it the
/// items' text is kept as the detail and `ok` stays `None`: the banner alone does not
/// say whether the command succeeded.
fn output(payload: &Value) -> Option<Record> {
    let tool_use_id = payload.get("call_id")?.as_str()?.to_owned();
    let texts: Vec<&str> = payload
        .get("output")?
        .as_array()?
        .iter()
        .filter_map(|item| text_of(item, "input_text"))
        .collect();
    let (detail, ok) = match texts.iter().find_map(|t| exec_result(t)) {
        Some((output, exit_code)) => (output, Some(exit_code == 0)),
        None if texts.is_empty() => return None,
        None => (texts.concat(), None),
    };
    Some(Record::ToolDetail {
        tool_use_id,
        input: None,
        detail: Some(detail),
        ok,
    })
}

fn exec_result(text: &str) -> Option<(String, i64)> {
    let map = object(text)?;
    let exit_code = map.get("exit_code")?.as_i64()?;
    let output = map.get("output")?.as_str()?.to_owned();
    Some((output, exit_code))
}

#[cfg(test)]
#[path = "codex_tests.rs"]
mod tests;
