//! Decision 51's shape test: `fake-agent`'s headless output against M8a.1's recorded
//! fixtures, in both directions (ruling T20-I2).

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::Path;

use serde_json::Value;

/// Decision 51's event type: `type`, plus `subtype` for Claude's `system` and `result`
/// lines, or `item.type` for a Codex item line.
pub fn event_type(event: &Value) -> String {
    let kind = event["type"].as_str().unwrap_or("").to_string();
    if let Some(subtype) = event["subtype"].as_str() {
        return format!("{kind}/{subtype}");
    }
    if let Some(item) = event["item"]["type"].as_str() {
        return format!("{kind}/{item}");
    }
    kind
}

/// Keys whose values are free-form data rather than a fixed shape: a tool's arguments,
/// and maps keyed by a model or agent-type name. Only their JSON type is checked.
const OPAQUE: &[&str] = &["input", "arguments", "tool_input", "modelUsage", "by_type"];

/// Every recorded sample of one event type, merged: the JSON types seen at each path,
/// the keys an object may have, the keys every sample's object has (the required ones;
/// a key some sample lacks is optional), and one schema per array element kind (an
/// element's `type` string, or `""`).
#[derive(Default, Debug)]
struct Schema {
    types: BTreeSet<&'static str>,
    keys: BTreeMap<String, Schema>,
    required: Option<BTreeSet<String>>,
    items: BTreeMap<String, Schema>,
    opaque: bool,
}

fn kind(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn element_kind(value: &Value) -> String {
    value["type"].as_str().unwrap_or("").to_string()
}

impl Schema {
    fn merge(&mut self, value: &Value) {
        self.types.insert(kind(value));
        if self.opaque {
            return;
        }
        match value {
            Value::Object(object) => {
                let present: BTreeSet<String> = object.keys().cloned().collect();
                self.required = Some(match self.required.take() {
                    None => present,
                    Some(required) => required.intersection(&present).cloned().collect(),
                });
                for (key, child) in object {
                    let schema = self.keys.entry(key.clone()).or_default();
                    schema.opaque = OPAQUE.contains(&key.as_str());
                    schema.merge(child);
                }
            }
            Value::Array(items) => {
                for item in items {
                    self.items
                        .entry(element_kind(item))
                        .or_default()
                        .merge(item);
                }
            }
            _ => {}
        }
    }

    /// Every problem with `ours` at `path`: a JSON type no sample had there, a key no
    /// sample has, a key every sample has that `ours` lacks, or an array element kind
    /// no sample has.
    fn check(&self, ours: &Value, path: &str, problems: &mut Vec<String>) {
        if !self.types.contains(kind(ours)) {
            let recorded = &self.types;
            problems.push(format!(
                "{path}: {} where recorded {recorded:?}",
                kind(ours)
            ));
            return;
        }
        if self.opaque {
            return;
        }
        match ours {
            Value::Object(object) => {
                for (key, value) in object {
                    match self.keys.get(key) {
                        None => problems.push(format!("{path}.{key}: not recorded")),
                        Some(child) => child.check(value, &format!("{path}.{key}"), problems),
                    }
                }
                for key in self.required.iter().flatten() {
                    if !object.contains_key(key) {
                        problems.push(format!("{path}.{key}: in every recorded sample, missing"));
                    }
                }
            }
            Value::Array(items) => {
                for item in items {
                    let element = element_kind(item);
                    match self.items.get(&element) {
                        None => problems.push(format!("{path}[{element:?}]: not recorded")),
                        Some(schema) => schema.check(item, &format!("{path}[{element}]"), problems),
                    }
                }
            }
            _ => {}
        }
    }
}

/// Every recorded stream event of `runtime`, observed and documented.
fn fixtures(runtime: &str) -> Vec<Value> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../daemon/tests/fixtures/headless");
    let mut events = Vec::new();
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // The input fixture is what was written to stdin, not what the CLI printed.
        if !name.starts_with(runtime) || !name.ends_with(".jsonl") || name.contains("-input") {
            continue;
        }
        let text = fs::read_to_string(&path).unwrap();
        events.extend(
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| serde_json::from_str::<Value>(l).unwrap())
                // The hooks fixture holds hook payloads, which have no `type`.
                .filter(|v| v.get("type").is_some()),
        );
    }
    events
}

/// Decision 51's shape test, both ways: for every event `fake-agent` wrote, against
/// every recorded sample of its type merged, each key it writes is recorded, each key
/// every sample has is written, and each value has a recorded JSON type. A key only
/// some samples have is optional. Returns the event types checked.
pub fn assert_conforms(runtime: &str, events: &[Value]) -> BTreeSet<String> {
    let mut schemas: BTreeMap<String, Schema> = BTreeMap::new();
    for sample in fixtures(runtime) {
        schemas
            .entry(event_type(&sample))
            .or_default()
            .merge(&sample);
    }
    let mut types = BTreeSet::new();
    for event in events {
        let kind = event_type(event);
        let schema = schemas
            .get(&kind)
            .unwrap_or_else(|| panic!("{runtime}: no recorded event of type {kind} for {event}"));
        let mut problems = Vec::new();
        schema.check(event, &kind, &mut problems);
        assert!(
            problems.is_empty(),
            "{runtime}: {event}\n{}",
            problems.join("\n")
        );
        types.insert(kind);
    }
    types
}
