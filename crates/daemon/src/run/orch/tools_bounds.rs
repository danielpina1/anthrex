//! Task M9.11, review finding 3: the Interfaces table's bounds on a tool call's
//! `plan_edit`, `plan_task` and `route` objects (Interfaces "MCP"), enforced on the
//! untrusted raw arguments before serde reads them. The MCP schemas are only what the
//! model sees; a forged call bypasses them. The schemas' objects are closed, so a field
//! they do not list is refused, except `plan_task`'s `budget`, which rule 7.1 refuses
//! with its own text (decision 23.1), and an edit whose `op` is not the schema's,
//! which serde refuses (decision 19).
//! Enums, integers, patterns, a task's `id`, `null` (an absent optional field) and
//! which keys each `op` needs are left to serde and the plan rules, as before. Pure.

use serde_json::{Map, Value};

/// What one field of an object may hold.
enum Rule {
    /// A string of `min` to `max` characters.
    Text(usize, usize),
    /// An array of `min` to `max` strings of 1 to `each` characters.
    List(usize, usize, usize),
    /// Checked by serde alone (an enum, an integer or a boolean).
    Serde,
    Task,
    /// An array of `min` to `max` tasks.
    Tasks(usize, usize),
    Route,
    /// `message`'s `to`: 1 to 20 task ids, or a string (its form is serde's).
    To,
}

const EDIT: &[(&str, Rule)] = &[
    ("op", Rule::Serde),
    ("task", Rule::Task),
    ("task_id", Rule::Text(1, 16)),
    ("into", Rule::Tasks(1, 12)),
    ("brief", Rule::Text(1, 8000)),
    ("acceptance", Rule::List(1, 20, 500)),
    ("route", Rule::Route),
    ("test_mode", Rule::Serde),
    ("test_mode_reason", Rule::Text(1, 300)),
    ("priority", Rule::Serde),
    ("size", Rule::Serde),
    ("deps", Rule::List(0, 20, 16)),
    ("dep", Rule::Text(1, 16)),
    ("text", Rule::Text(1, 8000)),
    ("to", Rule::To),
    ("kind", Rule::Serde),
];

const TASK: &[(&str, Rule)] = &[
    // Decision 19's structured `id` error is the plan rules' (M9.11 re-review 3).
    ("id", Rule::Serde),
    ("title", Rule::Text(1, 120)),
    ("epic", Rule::Text(1, 11)),
    ("kind", Rule::Serde),
    ("size", Rule::Serde),
    ("interface_change", Rule::Serde),
    ("test_mode", Rule::Serde),
    ("test_mode_reason", Rule::Text(1, 300)),
    ("owns", Rule::List(0, 20, 300)),
    ("deps", Rule::List(0, 20, 16)),
    ("priority", Rule::Serde),
    ("brief", Rule::Text(1, 8000)),
    ("acceptance", Rule::List(1, 20, 500)),
    ("test_to_write", Rule::Text(1, 300)),
    ("scout_refs", Rule::List(0, 20, 48)),
    ("route", Rule::Route),
    ("review_target", Rule::Text(1, 200)),
    // Not in the schema (decision 23.1), but M8a's `PlanTask` reads it and rule 7.1
    // refuses it with its own text, which tells the model what to do.
    ("budget", Rule::Serde),
];

const ROUTE: &[(&str, Rule)] = &[
    ("runtime", Rule::Serde),
    ("model", Rule::Text(0, 100)),
    ("strength", Rule::Serde),
    ("effort", Rule::Serde),
];

/// The `op`s the schema lists. An edit with any other is left to serde, whose
/// `unknown variant` refusal is decision 19's documented text for `override`.
const OPS: &[&str] = &[
    "add_task",
    "split_task",
    "cancel_task",
    "amend_task",
    "add_dep",
    "answer",
    "pause",
    "resume",
    "finish",
    "message",
    "refresh",
];

/// One `plan_edit`: `Err` holds `<path>: <problem>`, the path relative to the edit.
pub(super) fn check_edit(edit: &Value) -> Result<(), String> {
    let known = edit["op"].as_str().is_some_and(|op| OPS.contains(&op));
    if !known {
        return Ok(());
    }
    object(edit, EDIT, "")
}

fn object(value: &Value, rules: &[(&str, Rule)], path: &str) -> Result<(), String> {
    // A non-object is serde's to refuse, with its own text.
    let Some(map) = value.as_object() else {
        return Ok(());
    };
    fields(map, rules, path)
}

fn fields(map: &Map<String, Value>, rules: &[(&str, Rule)], path: &str) -> Result<(), String> {
    for (key, value) in map {
        let at = format!("{path}{key}");
        let Some((_, rule)) = rules.iter().find(|(name, _)| name == key) else {
            return Err(format!("{at}: unknown field"));
        };
        // `null` is absent, as serde's `Option` reads it; for a field that is not
        // optional, serde refuses it with its own text (M9.11 re-review 1).
        if value.is_null() {
            continue;
        }
        match rule {
            Rule::Serde => {}
            Rule::Text(min, max) => text(value, &at, *min, *max)?,
            Rule::List(min, max, each) => {
                for (i, item) in array(value, &at, *min, *max)?.iter().enumerate() {
                    text(item, &format!("{at}[{i}]"), 1, *each)?;
                }
            }
            Rule::Task => object(value, TASK, &format!("{at}: "))?,
            Rule::Tasks(min, max) => {
                for (i, item) in array(value, &at, *min, *max)?.iter().enumerate() {
                    object(item, TASK, &format!("{at}[{i}]: "))?;
                }
            }
            Rule::Route => object(value, ROUTE, &format!("{at}: "))?,
            Rule::To => {
                if value.is_array() {
                    for (i, item) in array(value, &at, 1, 20)?.iter().enumerate() {
                        text(item, &format!("{at}[{i}]"), 1, 16)?;
                    }
                }
            }
        }
    }
    Ok(())
}

fn text(value: &Value, at: &str, min: usize, max: usize) -> Result<(), String> {
    match value {
        Value::String(s) if (min..=max).contains(&s.chars().count()) => Ok(()),
        Value::String(_) => Err(format!("{at}: must be {min} to {max} characters")),
        _ => Err(format!("{at}: must be a string")),
    }
}

fn array<'a>(value: &'a Value, at: &str, min: usize, max: usize) -> Result<&'a [Value], String> {
    let Value::Array(items) = value else {
        return Err(format!("{at}: must be an array"));
    };
    if items.len() < min {
        let s = if min == 1 { "" } else { "s" };
        return Err(format!("{at}: at least {min} item{s}"));
    }
    if items.len() > max {
        return Err(format!("{at}: at most {max} items"));
    }
    Ok(items)
}
