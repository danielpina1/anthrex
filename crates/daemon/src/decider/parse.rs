//! Decision 16's answer extraction and decision 17's validation. No JSON-schema crate:
//! [`parse`] checks every limit of `schema.rs` by hand and names the failing path, for
//! example `tasks[2].size: expected S, M or L`. Pure.

use super::{
    BlockKind, DeciderAnswer, DeciderKind, DeciderRequest, SizeVerdict, TriageAnswer, TriageTask,
};
use crate::headless::SessionEvent;
use proto::{Scale, Size, TaskKind, TestMode};
use serde_json::{Map, Value};

/// The tool Claude's `--json-schema` makes the model call with its answer.
pub const STRUCTURED_OUTPUT_TOOL: &str = "StructuredOutput";

/// The answer in a decider session's events, in decision 16's order: a
/// `StructuredOutput` event (`result.structured_output`), else the input of a top-level
/// `StructuredOutput` tool use, else the last top-level assistant text parsed by
/// [`json_from_text`]. An error is the JSON error, or `the session gave no answer`.
pub fn answer_from_events(events: &[SessionEvent]) -> Result<Value, String> {
    let structured = events.iter().rev().find_map(|e| match e {
        SessionEvent::StructuredOutput { value } => Some(value),
        _ => None,
    });
    if let Some(value) = structured {
        return Ok(value.clone());
    }
    let tool = events.iter().rev().find_map(|e| match e {
        SessionEvent::ToolUse {
            name,
            input,
            parent: None,
            ..
        } if name == STRUCTURED_OUTPUT_TOOL => Some(input),
        _ => None,
    });
    if let Some(input) = tool {
        return Ok(input.clone());
    }
    let text = events.iter().rev().find_map(|e| match e {
        SessionEvent::AssistantText { text, parent: None } => Some(text),
        _ => None,
    });
    match text {
        Some(text) => json_from_text(text),
        None => Err("the session gave no answer".into()),
    }
}

/// `text` trimmed, with one surrounding fenced code block (and its info string, such as
/// `json`) removed, parsed as JSON. Used for assistant text and for a `result`'s text
/// (ruling R-T1-4: a model that answered in text before hitting `--max-turns`).
pub fn json_from_text(text: &str) -> Result<Value, String> {
    serde_json::from_str(strip_fence(text)).map_err(|e| e.to_string())
}

fn strip_fence(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(rest) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    let body = match rest.find('\n') {
        Some(newline) => &rest[newline + 1..],
        None => rest,
    };
    let body = body.trim_end();
    body.strip_suffix("```").unwrap_or(body).trim()
}

/// [`parse`], then, for a size check, only the verdicts of task ids `request` asked
/// about, the first per id (decision 17: ids not asked for are ignored). The call
/// (M8b.7) uses this one.
pub fn parse_for(request: &DeciderRequest, value: &Value) -> Result<DeciderAnswer, String> {
    let answer = parse(request.kind(), value)?;
    match (request, answer) {
        (DeciderRequest::SizeCheck(input), DeciderAnswer::SizeCheck(verdicts)) => {
            let mut kept: Vec<SizeVerdict> = Vec::new();
            for verdict in verdicts {
                let asked = input.tasks.iter().any(|t| t.id == verdict.id);
                if asked && !kept.iter().any(|k| k.id == verdict.id) {
                    kept.push(verdict);
                }
            }
            Ok(DeciderAnswer::SizeCheck(kept))
        }
        (_, answer) => Ok(answer),
    }
}

/// `value` checked against `kind`'s schema and turned into an answer.
pub fn parse(kind: DeciderKind, value: &Value) -> Result<DeciderAnswer, String> {
    match kind {
        DeciderKind::Triage => triage(value),
        DeciderKind::SizeCheck => {
            let root = object(value, "", &["tasks"])?;
            let tasks = array(&root["tasks"], "tasks", 0, 50)?;
            let mut verdicts = Vec::with_capacity(tasks.len());
            for (i, task) in tasks.iter().enumerate() {
                let path = format!("tasks[{i}]");
                let t = object(task, &path, &["id", "size", "reason"])?;
                let size = match one_of(&t["size"], &at(&path, "size"), &["S", "M", "L"])? {
                    "S" => Size::S,
                    "M" => Size::M,
                    _ => Size::L,
                };
                verdicts.push(SizeVerdict {
                    id: string(&t["id"], &at(&path, "id"), 1, 16)?,
                    size,
                    reason: string(&t["reason"], &at(&path, "reason"), 1, 500)?,
                });
            }
            Ok(DeciderAnswer::SizeCheck(verdicts))
        }
        DeciderKind::CheckSummary => {
            let root = object(value, "", &["lines"])?;
            let lines = array(&root["lines"], "lines", 1, 40)?;
            let lines = lines
                .iter()
                .enumerate()
                .map(|(i, line)| string(line, &format!("lines[{i}]"), 0, 300))
                .collect::<Result<Vec<_>, _>>()?;
            Ok(DeciderAnswer::CheckSummary { lines })
        }
        DeciderKind::BlockedReason => {
            let root = object(value, "", &["kind", "reason"])?;
            let kind = match one_of(
                &root["kind"],
                "kind",
                &["question", "mis_sized", "environment"],
            )? {
                "question" => BlockKind::Question,
                "mis_sized" => BlockKind::MisSized,
                _ => BlockKind::Environment,
            };
            Ok(DeciderAnswer::BlockedReason {
                kind,
                reason: string(&root["reason"], "reason", 1, 300)?,
            })
        }
        DeciderKind::CiSummary => super::ci::parse(value),
    }
}

fn triage(value: &Value) -> Result<DeciderAnswer, String> {
    let root = object(value, "", &["kinds", "scale", "reason", "task"])?;
    let kinds = array(&root["kinds"], "kinds", 1, 4)?
        .iter()
        .enumerate()
        .map(|(i, k)| {
            Ok(
                match one_of(
                    k,
                    &format!("kinds[{i}]"),
                    &["code", "docs", "research", "review"],
                )? {
                    "code" => TaskKind::Code,
                    "docs" => TaskKind::Docs,
                    "research" => TaskKind::Research,
                    _ => TaskKind::Review,
                },
            )
        })
        .collect::<Result<Vec<_>, String>>()?;
    let scale = match one_of(&root["scale"], "scale", &["single", "plan", "large"])? {
        "single" => Scale::Single,
        "plan" => Scale::Plan,
        _ => Scale::Large,
    };
    let reason = string(&root["reason"], "reason", 1, 500)?;
    // `task` is read only for `single`, and ignored otherwise.
    let task = match (scale, &root["task"]) {
        (Scale::Single, Value::Null) => return Err("task: required when scale is single".into()),
        (Scale::Single, task) => Some(triage_task(task)?),
        _ => None,
    };
    Ok(DeciderAnswer::Triage(TriageAnswer {
        kinds,
        scale,
        reason,
        task,
    }))
}

fn triage_task(value: &Value) -> Result<TriageTask, String> {
    let keys = [
        "title",
        "brief",
        "acceptance",
        "owns",
        "size",
        "interface_change",
        "test_mode",
        "test_mode_reason",
        "test_to_write",
    ];
    let t = object(value, "task", &keys)?;
    let strings = |key: &str, max_items: usize, max_chars: usize| {
        let path = at("task", key);
        array(&t[key], &path, 1, max_items)?
            .iter()
            .enumerate()
            .map(|(i, s)| string(s, &format!("{path}[{i}]"), 1, max_chars))
            .collect::<Result<Vec<_>, String>>()
    };
    let size = match one_of(&t["size"], "task.size", &["S", "M"])? {
        "S" => Size::S,
        _ => Size::M,
    };
    let test_mode = match one_of(&t["test_mode"], "task.test_mode", &["tdd", "check", "none"])? {
        "tdd" => TestMode::Tdd,
        "check" => TestMode::Check,
        _ => TestMode::None,
    };
    let Value::Bool(interface_change) = t["interface_change"] else {
        return Err("task.interface_change: expected a boolean".into());
    };
    Ok(TriageTask {
        title: string(&t["title"], "task.title", 1, 120)?,
        brief: string(&t["brief"], "task.brief", 1, 4000)?,
        acceptance: strings("acceptance", 10, 500)?,
        owns: strings("owns", 10, 300)?,
        size,
        interface_change,
        test_mode,
        test_mode_reason: nullable_string(&t["test_mode_reason"], "task.test_mode_reason", 300)?,
        test_to_write: nullable_string(&t["test_to_write"], "task.test_to_write", 300)?,
    })
}

/// `path.key`, or `key` at the root.
fn at(path: &str, key: &str) -> String {
    if path.is_empty() {
        key.to_string()
    } else {
        format!("{path}.{key}")
    }
}

/// `path: problem`, or `problem` at the root.
fn problem(path: &str, problem: &str) -> String {
    if path.is_empty() {
        problem.to_string()
    } else {
        format!("{path}: {problem}")
    }
}

/// A closed object with exactly `keys`.
pub(super) fn object<'a>(
    value: &'a Value,
    path: &str,
    keys: &[&str],
) -> Result<&'a Map<String, Value>, String> {
    let Value::Object(map) = value else {
        return Err(problem(path, "expected an object"));
    };
    if let Some(key) = keys.iter().find(|k| !map.contains_key(**k)) {
        return Err(format!("{}: missing", at(path, key)));
    }
    if let Some(key) = map.keys().find(|k| !keys.contains(&k.as_str())) {
        return Err(format!("{}: not in the schema", at(path, key)));
    }
    Ok(map)
}

pub(super) fn array<'a>(
    value: &'a Value,
    path: &str,
    min: usize,
    max: usize,
) -> Result<&'a [Value], String> {
    let Value::Array(items) = value else {
        return Err(problem(path, "expected an array"));
    };
    let noun = |n: usize| if n == 1 { "item" } else { "items" };
    if items.len() < min {
        return Err(problem(
            path,
            &format!("must have at least {min} {}", noun(min)),
        ));
    }
    if items.len() > max {
        return Err(problem(
            path,
            &format!("must have at most {max} {}", noun(max)),
        ));
    }
    Ok(items)
}

/// A string of `min..=max` characters (JSON Schema counts characters, not bytes).
pub(super) fn string(value: &Value, path: &str, min: usize, max: usize) -> Result<String, String> {
    let Value::String(s) = value else {
        return Err(problem(path, "expected a string"));
    };
    let chars = s.chars().count();
    if chars < min {
        return Err(problem(path, "must not be empty"));
    }
    if chars > max {
        return Err(problem(path, &format!("must be at most {max} characters")));
    }
    Ok(s.clone())
}

fn nullable_string(value: &Value, path: &str, max: usize) -> Result<Option<String>, String> {
    match value {
        Value::Null => Ok(None),
        Value::String(_) => string(value, path, 0, max).map(Some),
        _ => Err(problem(path, "expected a string or null")),
    }
}

/// One of `options`, returned as the option itself.
pub(super) fn one_of<'a>(
    value: &Value,
    path: &str,
    options: &[&'a str],
) -> Result<&'a str, String> {
    value
        .as_str()
        .and_then(|s| options.iter().find(|o| **o == s).copied())
        .ok_or_else(|| problem(path, &format!("expected {}", or_list(options))))
}

/// `a, b or c`.
fn or_list(options: &[&str]) -> String {
    match options.split_last() {
        Some((last, [])) => (*last).to_string(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => String::new(),
    }
}
