//! The orchestrator's and sub-planners' tool calls, and a worker's `task_note`, parsed
//! into [`OrchCall`] (decision 15). The MCP server checks each schema (Interfaces
//! "MCP"); the daemon checks the limits again, since it does not trust a caller to
//! have. Every object is closed. Problems read `invalid arguments: <field>: <problem>`,
//! in M8a's worker-tool wording (`engine/tools.rs`). Pure.

use proto::{AgentRole, PlanEdit, TaskNoteKind};
use serde_json::{Map, Value};

use super::json::label;

/// `run_status`'s longest wait, in seconds.
pub const RUN_STATUS_MAX_WAIT: u64 = 50;
/// A `message` edit's text, in characters (decision 42a).
const MESSAGE_TEXT_MAX: usize = 4000;

/// One parsed call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrchCall {
    GetContext {
        scouts: Option<Vec<String>>,
    },
    SpawnScout {
        id: String,
        question: String,
        area: Vec<String>,
        web: bool,
    },
    SpawnSubplanner {
        epic: String,
        title: String,
        area: Vec<String>,
        brief: String,
        scout_refs: Vec<String>,
    },
    EditPlan {
        edits: Vec<PlanEdit>,
        submit: bool,
        summary: Option<String>,
        /// Milestone 9.3 decision 30: a round's request, alone in its call (the
        /// engine's check).
        iterate: Option<String>,
    },
    RunStatus {
        since: Option<u64>,
        wait_secs: u64,
    },
    TaskResult {
        task_id: String,
    },
    SubmitEpic {
        edits: Vec<PlanEdit>,
        note: Option<String>,
    },
    /// Milestone 9.3 decision 22: a next goal on the orchestrator's chain (the
    /// driver's, `chain_goal.rs`).
    StartGoal {
        goal: String,
    },
    /// A worker's (decision 42f).
    TaskNote {
        kind: TaskNoteKind,
        text: String,
    },
}

/// Parses `tool`'s `args` for `role`: the orchestrator's seven tools, a planner's
/// `get_context` and `submit_epic`, a worker's `task_note`. Any other pairing is
/// `tool <tool> is not available to the <role> role`.
pub fn parse_call(role: AgentRole, tool: &str, args: &Value) -> Result<OrchCall, String> {
    let allowed = match role {
        AgentRole::Orchestrator => matches!(
            tool,
            "get_context"
                | "spawn_scout"
                | "spawn_subplanner"
                | "edit_plan"
                | "run_status"
                | "task_result"
                | "start_goal"
        ),
        AgentRole::Planner => matches!(tool, "get_context" | "submit_epic"),
        // Milestone 9.5: a racer and a test writer have the worker's tools.
        AgentRole::Worker | AgentRole::Racer | AgentRole::TestWriter => tool == "task_note",
        AgentRole::Reviewer | AgentRole::Scout | AgentRole::Decider => false,
    };
    if !allowed {
        return Err(format!(
            "tool {tool} is not available to the {} role",
            label(&role)
        ));
    }
    parse(tool, args).map_err(|problem| format!("invalid arguments: {problem}"))
}

fn parse(tool: &str, args: &Value) -> Result<OrchCall, String> {
    match tool {
        "get_context" => {
            let map = object(args, &["scouts"])?;
            Ok(OrchCall::GetContext {
                scouts: list(map, "scouts", 0, 50, 48)?,
            })
        }
        "spawn_scout" => {
            let map = object(args, &["id", "question", "area", "web"])?;
            Ok(OrchCall::SpawnScout {
                id: id(map, "id", 31)?,
                question: required(text(map, "question", 2000)?, "question")?,
                area: required(list(map, "area", 1, 20, 300)?, "area")?,
                web: flag(map, "web")?,
            })
        }
        "spawn_subplanner" => {
            let map = object(args, &["epic", "title", "area", "brief", "scout_refs"])?;
            Ok(OrchCall::SpawnSubplanner {
                epic: id(map, "epic", 10)?,
                title: required(text(map, "title", 80)?, "title")?,
                area: required(list(map, "area", 1, 20, 300)?, "area")?,
                brief: required(text(map, "brief", 8000)?, "brief")?,
                scout_refs: list(map, "scout_refs", 0, 20, 48)?.unwrap_or_default(),
            })
        }
        "edit_plan" => {
            let map = object(args, &["edits", "submit", "summary", "iterate"])?;
            let iterate = text(map, "iterate", proto::GOAL_MAX_CHARS)?;
            // Milestone 9.3 decision 30: an iterate needs no `edits` array; and task
            // M9.3.7's fix round 1 (option (b)): no call does, a missing `edits` is an
            // empty batch, so the MCP schema can stay plain and exact.
            let edits = match map.get("edits") {
                None => Vec::new(),
                Some(_) => edits(map, 0)?,
            };
            Ok(OrchCall::EditPlan {
                edits,
                submit: flag(map, "submit")?,
                summary: text(map, "summary", 8000)?,
                iterate,
            })
        }
        "run_status" => {
            let map = object(args, &["since", "wait_secs"])?;
            let since = match map.get("since") {
                None => None,
                Some(v) => Some(
                    v.as_u64()
                        .ok_or("since: must be a non-negative integer".to_string())?,
                ),
            };
            let wait_secs = match map.get("wait_secs") {
                None => 0,
                Some(v) => v
                    .as_u64()
                    .filter(|w| *w <= RUN_STATUS_MAX_WAIT)
                    .ok_or(format!("wait_secs: must be 0 to {RUN_STATUS_MAX_WAIT}"))?,
            };
            Ok(OrchCall::RunStatus { since, wait_secs })
        }
        "task_result" => {
            let map = object(args, &["task_id"])?;
            Ok(OrchCall::TaskResult {
                task_id: required(text(map, "task_id", 16)?, "task_id")?,
            })
        }
        "start_goal" => {
            let map = object(args, &["goal"])?;
            Ok(OrchCall::StartGoal {
                goal: required(text(map, "goal", proto::GOAL_MAX_CHARS)?, "goal")?,
            })
        }
        "submit_epic" => {
            let map = object(args, &["edits", "note"])?;
            Ok(OrchCall::SubmitEpic {
                edits: edits(map, 1)?,
                note: text(map, "note", 2000)?,
            })
        }
        "task_note" => {
            let map = object(args, &["kind", "text"])?;
            let kind = match map.get("kind").and_then(Value::as_str) {
                Some("discovery") => TaskNoteKind::Discovery,
                Some("risk") => TaskNoteKind::Risk,
                Some("progress") => TaskNoteKind::Progress,
                None if !map.contains_key("kind") => return Err("kind: required".into()),
                _ => return Err("kind: must be one of discovery, risk, progress".into()),
            };
            Ok(OrchCall::TaskNote {
                kind,
                text: required(text(map, "text", 4000)?, "text")?,
            })
        }
        // `parse_call` lets through only the tools above.
        other => Err(format!("{other}: unknown tool")),
    }
}

fn object<'a>(args: &'a Value, fields: &[&str]) -> Result<&'a Map<String, Value>, String> {
    let Some(map) = args.as_object() else {
        return Err("arguments: must be an object".into());
    };
    if let Some(key) = map.keys().find(|k| !fields.contains(&k.as_str())) {
        return Err(format!("{key}: unknown field"));
    }
    Ok(map)
}

fn required<T>(value: Option<T>, field: &str) -> Result<T, String> {
    value.ok_or_else(|| format!("{field}: required"))
}

/// A string of 1 to `max` characters; `None` when absent.
fn text(map: &Map<String, Value>, field: &str, max: usize) -> Result<Option<String>, String> {
    match map.get(field) {
        None => Ok(None),
        Some(value) => one_text(value, field, max).map(Some),
    }
}

fn one_text(value: &Value, field: &str, max: usize) -> Result<String, String> {
    match value {
        Value::String(s) if (1..=max).contains(&s.chars().count()) => Ok(s.clone()),
        Value::String(_) => Err(format!("{field}: must be 1 to {max} characters")),
        _ => Err(format!("{field}: must be a string")),
    }
}

/// A required id matching `^[a-z0-9][a-z0-9-]{0,<rest>}$`.
fn id(map: &Map<String, Value>, field: &str, rest: usize) -> Result<String, String> {
    let value = match map.get(field) {
        None => return Err(format!("{field}: required")),
        Some(Value::String(s)) => s,
        Some(_) => return Err(format!("{field}: must be a string")),
    };
    let first = value
        .bytes()
        .next()
        .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    let all = value
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if first && all && value.len() <= rest + 1 {
        Ok(value.clone())
    } else {
        Err(format!(
            "{field}: must match ^[a-z0-9][a-z0-9-]{{0,{rest}}}$"
        ))
    }
}

fn flag(map: &Map<String, Value>, field: &str) -> Result<bool, String> {
    match map.get(field) {
        None => Ok(false),
        Some(Value::Bool(b)) => Ok(*b),
        Some(_) => Err(format!("{field}: must be a boolean")),
    }
}

/// An array of `min` to `max` strings of 1 to `each` characters; `None` when absent.
fn list(
    map: &Map<String, Value>,
    field: &str,
    min: usize,
    max: usize,
    each: usize,
) -> Result<Option<Vec<String>>, String> {
    let Some(value) = map.get(field) else {
        return Ok(None);
    };
    let items = array(value, field, min, max)?;
    items
        .iter()
        .enumerate()
        .map(|(i, v)| one_text(v, &format!("{field}[{i}]"), each))
        .collect::<Result<Vec<_>, _>>()
        .map(Some)
}

fn array<'a>(value: &'a Value, field: &str, min: usize, max: usize) -> Result<&'a [Value], String> {
    let Value::Array(items) = value else {
        return Err(format!("{field}: must be an array"));
    };
    if items.len() < min {
        let s = if min == 1 { "" } else { "s" };
        return Err(format!("{field}: at least {min} item{s}"));
    }
    if items.len() > max {
        return Err(format!("{field}: at most {max} items"));
    }
    Ok(items)
}

/// `edits`: required, `min` to 60 plan edits, each within the Interfaces table's
/// nested bounds with no field the schema lacks (`tools_bounds.rs`), then M8a's
/// `PlanEdit` serde shape
/// (decision 19: `{"op": "override"}` fails here), a `message`'s text at most 4000
/// characters (decision 42a).
fn edits(map: &Map<String, Value>, min: usize) -> Result<Vec<PlanEdit>, String> {
    let value = required(map.get("edits"), "edits")?;
    array(value, "edits", min, 60)?
        .iter()
        .enumerate()
        .map(|(i, v)| {
            bounds::check_edit(v).map_err(|e| format!("edits[{i}]: {e}"))?;
            let edit: PlanEdit =
                serde_json::from_value(v.clone()).map_err(|e| format!("edits[{i}]: {e}"))?;
            if let PlanEdit::Message { text, .. } = &edit
                && text.chars().count() > MESSAGE_TEXT_MAX
            {
                return Err(format!(
                    "edits[{i}]: message: text: at most {MESSAGE_TEXT_MAX} characters"
                ));
            }
            Ok(edit)
        })
        .collect()
}

#[path = "tools_bounds.rs"]
mod bounds;

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tools_tests_bounds.rs"]
mod tests_bounds;
