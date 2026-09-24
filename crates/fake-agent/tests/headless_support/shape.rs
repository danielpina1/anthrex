//! Decision 51's shape test: `fake-agent`'s headless output against M8a.1's recorded
//! fixtures.

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use serde_json::{Value, json};

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

/// Keys whose values are free-form input (a tool's arguments): only their presence is
/// checked, never their inner keys.
const OPAQUE: &[&str] = &["input", "arguments", "tool_input"];

/// Whether `fixture`, recursively, has every key `ours` writes. An array in `ours` must
/// have each element contained in some element of the fixture's array.
fn contains(fixture: &Value, ours: &Value) -> bool {
    match ours {
        Value::Object(object) => {
            let Value::Object(theirs) = fixture else {
                return false;
            };
            object.iter().all(|(key, value)| match theirs.get(key) {
                None => false,
                Some(_) if OPAQUE.contains(&key.as_str()) => true,
                Some(their) => contains(their, value),
            })
        }
        Value::Array(items) => {
            let Value::Array(theirs) = fixture else {
                return items.is_empty();
            };
            items
                .iter()
                .all(|item| theirs.iter().any(|their| contains(their, item)))
        }
        _ => true,
    }
}

fn fixtures(runtime: &str) -> (Vec<Value>, Vec<Value>) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../daemon/tests/fixtures/headless");
    let (mut observed, mut documented) = (Vec::new(), Vec::new());
    for entry in fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        // The input fixture is what was written to stdin, not what the CLI printed.
        if !name.starts_with(runtime) || !name.ends_with(".jsonl") || name.contains("-input") {
            continue;
        }
        let meta: Value = serde_json::from_slice(
            &fs::read(path.with_file_name(name.replace(".jsonl", ".meta.json"))).unwrap(),
        )
        .unwrap();
        let text = fs::read_to_string(&path).unwrap();
        let events = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| serde_json::from_str::<Value>(l).unwrap())
            // The hooks fixture holds hook payloads, which have no `type`.
            .filter(|v| v.get("type").is_some());
        if meta["observed"] == json!(false) {
            documented.extend(events);
        } else {
            observed.extend(events);
        }
    }
    (observed, documented)
}

/// Decision 51's shape test: for every event `fake-agent` wrote, a fixture event of the
/// same type (observed first, else documented) contains every key it wrote. Returns
/// the event types checked.
pub fn assert_conforms(runtime: &str, events: &[Value]) -> BTreeSet<String> {
    let (observed, documented) = fixtures(runtime);
    let mut types = BTreeSet::new();
    for event in events {
        let kind = event_type(event);
        let same = |pool: &[Value]| -> Vec<Value> {
            pool.iter()
                .filter(|f| event_type(f) == kind)
                .cloned()
                .collect()
        };
        let (observed, documented) = (same(&observed), same(&documented));
        assert!(
            !observed.is_empty() || !documented.is_empty(),
            "{runtime}: no recorded event of type {kind} for {event}"
        );
        // Observed recordings first; a shape only the documented file has (a failed
        // turn's synthetic assistant line) is checked against it.
        assert!(
            observed.iter().any(|c| contains(c, event))
                || documented.iter().any(|c| contains(c, event)),
            "{runtime}: no recorded {kind} event has every key of {event}"
        );
        types.insert(kind);
    }
    types
}
