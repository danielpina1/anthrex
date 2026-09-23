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

    /// Every line is `{timestamp, type, payload}`; a `response_item`, the one kind this
    /// parser reads conversation from, is broken without an object `payload` naming its
    /// own `type`.
    fn malformed(&self, _version: Version, line: &str) -> bool {
        let Some(map) = object(line) else {
            return true;
        };
        match map.get("type").and_then(Value::as_str) {
            Some("response_item") => !map
                .get("payload")
                .and_then(|p| p.get("type"))
                .is_some_and(Value::is_string),
            _ => false,
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
            let Some(texts) = prompt_texts(payload, items) else {
                return Vec::new();
            };
            // A prompt opens a turn even when it has no text to offer, so an image-only
            // prompt never shifts a later ordinal (review F5).
            let ordinal = cursor.next_prompt();
            if texts.is_empty() {
                return Vec::new();
            }
            vec![Record::UserText {
                session_id: cursor.session_id.clone(),
                ordinal,
                text: texts.join("\n"),
                human: true,
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

/// The texts of a `role:"user"` message that is a real prompt, or `None` when it is
/// injected context. In the capture `content_item_kinds` runs parallel to `content`, and
/// a prompt's kinds are `user.*` while injected context's are not. A message is a prompt
/// when any kind starts with `user.`, or when it has no kinds at all (review F5). Its
/// text is the `user.text` items when the kinds line up with the content, and every
/// `input_text` item when they are missing or do not.
fn prompt_texts<'a>(payload: &'a Value, items: &'a [Value]) -> Option<Vec<&'a str>> {
    let kinds = payload
        .get("internal_chat_message_metadata_passthrough")
        .and_then(|m| m.get("content_item_kinds"))
        .and_then(Value::as_array);
    let all_text = || {
        items
            .iter()
            .filter_map(|i| text_of(i, "input_text"))
            .collect()
    };
    let Some(kinds) = kinds else {
        return Some(all_text());
    };
    let is_prompt = kinds
        .iter()
        .any(|k| k.as_str().is_some_and(|k| k.starts_with("user.")));
    if !is_prompt {
        return None;
    }
    if kinds.len() != items.len() {
        return Some(all_text());
    }
    Some(
        items
            .iter()
            .zip(kinds)
            .filter(|(_, kind)| kind.as_str() == Some("user.text"))
            .filter_map(|(item, _)| text_of(item, "input_text"))
            .collect(),
    )
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
