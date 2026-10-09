//! Reading a `[models]` table and rendering one (MR §3.2, §3.3). Pure.
//!
//! The shapes: `models.<row>` for `orchestrator`, `planner`, `test_writer`, `reviewer`
//! and `research`; `models.implementer.{small,medium,hub}`; `models.helpers`, whose
//! kind keys (`run_name`, …) are rows of their own; and `models.brainstorm` (`first`,
//! `second`, `effort`). A row has `model` (required), `effort` and `fallback`. A bad
//! row is one [`Problem`] and is skipped; an unknown key is one and is ignored; the
//! rest of the table is kept.

use proto::models::{
    BrainstormChoice, HelperKind, ModelRef, ModelTable, Role, RoleChoice, valid_effort,
};

use crate::{ENTRY_SKIPPED, Problem, not_a_table_problem, unknown_key_problem};

const ROW_KEYS: &[&str] = &["model", "effort", "fallback"];
const BRAINSTORM_KEYS: &[&str] = &["first", "second", "effort"];
const IMPLEMENTER: [(&str, Role); 3] = [
    ("small", Role::ImplementerSmall),
    ("medium", Role::ImplementerMedium),
    ("hub", Role::ImplementerHub),
];

/// `[models]` (`value`, the top-level key's value) into a table; a bad row or key is a
/// `Problem` keyed `models.<row>.<key>` and is skipped; the rest is kept. The problems
/// are pushed sorted by key.
pub(crate) fn read_table(value: &toml::Value, problems: &mut Vec<Problem>) -> ModelTable {
    let mut out = ModelTable::default();
    let Some(models) = value.as_table() else {
        problems.push(not_a_table_problem("models"));
        return out;
    };
    let mut found = Vec::new();
    for (key, value) in models {
        let path = format!("models.{key}");
        match key.as_str() {
            "orchestrator" | "planner" | "test_writer" | "reviewer" | "research" => {
                let role = Role::parse_key(key).expect("a row key is a role");
                if let Some(t) = table_at(&path, value, &mut found) {
                    report_unknown(&path, t, ROW_KEYS, &mut found);
                    if let Some(choice) = read_row(&path, t, &mut found) {
                        out.rows.insert(role, choice);
                    }
                }
            }
            "implementer" => read_implementer(&path, value, &mut out, &mut found),
            "helpers" => read_helpers(&path, value, &mut out, &mut found),
            "brainstorm" => out.brainstorm = read_brainstorm(&path, value, &mut found),
            _ => found.push(unknown_key_problem(&path)),
        }
    }
    found.sort_by(|a, b| a.key.cmp(&b.key));
    problems.extend(found);
    out
}

/// A whole file's text (the repository file), read as `[models]`. `Err` is the TOML
/// syntax error; a key other than `models` is an unknown-key problem.
pub fn parse_text(text: &str) -> Result<(ModelTable, Vec<Problem>), String> {
    let table: toml::Table = text.parse().map_err(|e: toml::de::Error| e.to_string())?;
    let mut problems = Vec::new();
    let mut out = ModelTable::default();
    for (key, value) in &table {
        if key == "models" {
            out = read_table(value, &mut problems);
        } else {
            problems.push(unknown_key_problem(key));
        }
    }
    Ok((out, problems))
}

/// `table` as `[models.<row>]` tables, rows in `Role::all()` order, then brainstorm,
/// one blank line between tables.
pub fn render(table: &ModelTable) -> String {
    let mut out = String::new();
    let header = |out: &mut String, name: &str| {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(&format!("[models.{name}]\n"));
    };
    for role in Role::all() {
        let Some(choice) = table.rows.get(&role) else {
            continue;
        };
        header(&mut out, &role.key());
        line(&mut out, "model", &choice.model.label());
        if let Some(effort) = &choice.effort {
            line(&mut out, "effort", effort);
        }
        if let Some(fallback) = &choice.fallback {
            line(&mut out, "fallback", &fallback.label());
        }
    }
    if let Some(b) = &table.brainstorm {
        header(&mut out, "brainstorm");
        line(&mut out, "first", &b.first.label());
        line(&mut out, "second", &b.second.label());
        if let Some(effort) = &b.effort {
            line(&mut out, "effort", effort);
        }
    }
    out
}

fn line(out: &mut String, key: &str, value: &str) {
    let value = toml::Value::String(value.to_string());
    out.push_str(&format!("{key} = {value}\n"));
}

fn read_implementer(
    path: &str,
    value: &toml::Value,
    out: &mut ModelTable,
    found: &mut Vec<Problem>,
) {
    let Some(t) = table_at(path, value, found) else {
        return;
    };
    let names: Vec<&str> = IMPLEMENTER.iter().map(|(name, _)| *name).collect();
    report_unknown(path, t, &names, found);
    for (name, role) in IMPLEMENTER {
        let sub = format!("{path}.{name}");
        let Some(row) = t.get(name).and_then(|v| table_at(&sub, v, found)) else {
            continue;
        };
        report_unknown(&sub, row, ROW_KEYS, found);
        if let Some(choice) = read_row(&sub, row, found) {
            out.rows.insert(role, choice);
        }
    }
}

/// `[models.helpers]` is a row when it sets any row key; its kind keys are rows either
/// way.
fn read_helpers(path: &str, value: &toml::Value, out: &mut ModelTable, found: &mut Vec<Problem>) {
    let Some(t) = table_at(path, value, found) else {
        return;
    };
    let mut allowed: Vec<&str> = ROW_KEYS.to_vec();
    allowed.extend(HelperKind::ALL.iter().map(|k| k.key()));
    report_unknown(path, t, &allowed, found);
    if ROW_KEYS.iter().any(|k| t.contains_key(*k))
        && let Some(choice) = read_row(path, t, found)
    {
        out.rows.insert(Role::Helpers, choice);
    }
    for kind in HelperKind::ALL {
        let sub = format!("{path}.{}", kind.key());
        let Some(row) = t.get(kind.key()).and_then(|v| table_at(&sub, v, found)) else {
            continue;
        };
        report_unknown(&sub, row, ROW_KEYS, found);
        if let Some(choice) = read_row(&sub, row, found) {
            out.rows.insert(Role::Helper(kind), choice);
        }
    }
}

fn read_brainstorm(
    path: &str,
    value: &toml::Value,
    found: &mut Vec<Problem>,
) -> Option<BrainstormChoice> {
    let t = table_at(path, value, found)?;
    report_unknown(path, t, BRAINSTORM_KEYS, found);
    let first = required_model(path, "first", t, found);
    let second = required_model(path, "second", t, found);
    let effort = optional_effort(path, t, found);
    Some(BrainstormChoice {
        first: first?,
        second: second?,
        effort: effort?,
    })
}

/// The row's three keys; `None` (and a problem) when any is bad or `model` is missing.
fn read_row(path: &str, t: &toml::Table, found: &mut Vec<Problem>) -> Option<RoleChoice> {
    let model = required_model(path, "model", t, found);
    let effort = optional_effort(path, t, found);
    let fallback = match t.get("fallback") {
        None => Some(None),
        Some(v) => model_at(&format!("{path}.fallback"), v, found).map(Some),
    };
    Some(RoleChoice {
        model: model?,
        effort: effort?,
        fallback: fallback?,
    })
}

fn required_model(
    path: &str,
    name: &str,
    t: &toml::Table,
    found: &mut Vec<Problem>,
) -> Option<ModelRef> {
    let key = format!("{path}.{name}");
    match t.get(name) {
        Some(v) => model_at(&key, v, found),
        None => {
            found.push(skipped(&key, "is required (<runtime>:<model>)"));
            None
        }
    }
}

/// `Some(None)` without an effort, `None` (and a problem) for a bad one.
fn optional_effort(
    path: &str,
    t: &toml::Table,
    found: &mut Vec<Problem>,
) -> Option<Option<String>> {
    let Some(value) = t.get("effort") else {
        return Some(None);
    };
    let key = format!("{path}.effort");
    let name = string_at(&key, value, found)?;
    if !valid_effort(name) {
        found.push(skipped(
            &key,
            &format!("{name:?} is not an effort name (1 to 16 of a-z, 0-9, _ and -)"),
        ));
        return None;
    }
    Some(Some(name.to_string()))
}

fn model_at(key: &str, value: &toml::Value, found: &mut Vec<Problem>) -> Option<ModelRef> {
    let text = string_at(key, value, found)?;
    ModelRef::parse(text)
        .map_err(|e| found.push(skipped(key, &e)))
        .ok()
}

fn string_at<'a>(key: &str, value: &'a toml::Value, found: &mut Vec<Problem>) -> Option<&'a str> {
    let text = value.as_str();
    if text.is_none() {
        found.push(skipped(key, "expected a string"));
    }
    text
}

fn table_at<'a>(
    path: &str,
    value: &'a toml::Value,
    found: &mut Vec<Problem>,
) -> Option<&'a toml::Table> {
    let t = value.as_table();
    if t.is_none() {
        found.push(skipped(path, "expected a table"));
    }
    t
}

fn report_unknown(path: &str, t: &toml::Table, known: &[&str], found: &mut Vec<Problem>) {
    for key in t.keys() {
        if !known.contains(&key.as_str()) {
            found.push(unknown_key_problem(&format!("{path}.{key}")));
        }
    }
}

/// A problem that drops the row it is in.
fn skipped(key: &str, message: &str) -> Problem {
    Problem {
        key: key.to_string(),
        message: message.to_string(),
        default: ENTRY_SKIPPED.to_string(),
    }
}
