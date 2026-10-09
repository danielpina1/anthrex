//! Milestone 9.9 decision 15: `ask_user`'s arguments, checked again by the daemon with
//! the MCP schema's bounds (`question` 1 to 500 characters, at most nine `options` of 1
//! to 200, `context` 1 to 4000). The texts are the orchestrator's, untrusted where they
//! cross to the user; they are kept as given, and clients sanitise where they draw. Pure.

use serde_json::{Map, Value};

use super::{OrchCall, object};

const QUESTION_MAX: usize = 500;
const OPTIONS_MAX: usize = 9;
const OPTION_MAX: usize = 200;
const CONTEXT_MAX: usize = 4000;

pub(super) fn parse(args: &Value) -> Result<OrchCall, String> {
    let map = object(args, &["question", "options", "context"])?;
    let question = match map.get("question") {
        None => return Err("question: required".into()),
        Some(value) => one(value, "question", QUESTION_MAX)?,
    };
    let context = match map.get("context") {
        None => String::new(),
        Some(value) => one(value, "context", CONTEXT_MAX)?,
    };
    Ok(OrchCall::AskUser {
        question,
        options: options(map)?,
        context,
    })
}

fn options(map: &Map<String, Value>) -> Result<Vec<String>, String> {
    let Some(value) = map.get("options") else {
        return Ok(Vec::new());
    };
    let Value::Array(items) = value else {
        return Err("options: must be an array".into());
    };
    if items.len() > OPTIONS_MAX {
        return Err(format!("options: at most {OPTIONS_MAX}"));
    }
    items
        .iter()
        .enumerate()
        .map(|(i, v)| one(v, &format!("options[{i}]"), OPTION_MAX))
        .collect()
}

/// A string of 1 to `max` characters.
fn one(value: &Value, field: &str, max: usize) -> Result<String, String> {
    match value {
        Value::String(s) if s.is_empty() => Err(format!("{field}: empty")),
        Value::String(s) if s.chars().count() > max => {
            Err(format!("{field}: at most {max} characters"))
        }
        Value::String(s) => Ok(s.clone()),
        _ => Err(format!("{field}: must be a string")),
    }
}
