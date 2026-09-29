//! M8b.9: the scout's tool and its schema.

use crate::tools::{allowed, tools_for};
use proto::AgentRole;
use serde_json::{Value, json};

fn schema() -> Value {
    let tool = tools_for(AgentRole::Scout)
        .into_iter()
        .next()
        .expect("a scout tool");
    Value::Object((*tool.input_schema).clone())
}

#[test]
fn scout_tool_is_submit_scout_report() {
    let tools = tools_for(AgentRole::Scout);
    let names: Vec<String> = tools.iter().map(|t| t.name.to_string()).collect();
    assert_eq!(names, ["submit_scout_report"]);
    assert_eq!(
        tools[0].description.as_deref(),
        Some("Submit your findings. Call it once, then stop.")
    );
    assert!(allowed(AgentRole::Scout, "submit_scout_report"));
    for other in ["task_done", "task_blocked", "submit_review"] {
        assert!(!allowed(AgentRole::Scout, other), "{other}");
    }
    for role in [
        AgentRole::Worker,
        AgentRole::Reviewer,
        AgentRole::Orchestrator,
    ] {
        assert!(!allowed(role, "submit_scout_report"), "{role:?}");
    }
}

/// Every property name at every level, with the objects' closedness checked on the way.
fn walk(path: &str, v: &Value, names: &mut Vec<String>) {
    if v["type"] == "object" {
        assert_eq!(
            v["additionalProperties"],
            json!(false),
            "{path} is open: {v}"
        );
        let properties = v["properties"].as_object().expect("properties");
        for (k, p) in properties {
            names.push(k.clone());
            walk(&format!("{path}.{k}"), p, names);
        }
    }
    if let Some(items) = v.get("items") {
        walk(&format!("{path}[]"), items, names);
    }
}

#[test]
fn scout_schema_limits() {
    let s = schema();
    let text = |min: u64, max: u64| json!({"type": "string", "minLength": min, "maxLength": max});
    let list =
        |max: u64, chars: u64| json!({"type": "array", "maxItems": max, "items": text(1, chars)});
    assert_eq!(s["required"], json!(["summary", "files"]));
    let p = &s["properties"];
    assert_eq!(p["summary"], text(1, 8000));
    assert_eq!(p["files"]["maxItems"], 60);
    let file = &p["files"]["items"];
    assert_eq!(file["required"], json!(["path", "why"]));
    assert_eq!(file["properties"]["path"], text(1, 500));
    assert_eq!(file["properties"]["why"], text(1, 300));
    assert_eq!(p["modules"], list(40, 200));
    assert_eq!(p["interfaces"], list(40, 500));
    assert_eq!(p["risks"], list(20, 500));

    let profile = &p["profile"];
    assert_eq!(profile["required"], json!([]));
    let q = &profile["properties"];
    assert_eq!(q["languages"], list(10, 40));
    for key in ["modules", "hub", "source", "generated", "protected"] {
        assert_eq!(q[key], list(40, 300), "{key}");
    }
    assert_eq!(q["setup"], text(1, 2000));
    assert_eq!(q["check"], text(1, 2000));
    assert_eq!(
        q["check_timeout_secs"],
        json!({"type": "integer", "minimum": 10, "maximum": 14400})
    );
    assert_eq!(q["single_test"], text(1, 1000));
    assert_eq!(q["test_passed"], text(1, 300));
    assert_eq!(q["sample_test"], text(1, 300));
    assert_eq!(
        q["output_filter"],
        json!({"type": "string", "enum": ["failures-only", "tail", "none"]})
    );
    assert_eq!(q["filter_prefixes"], list(10, 100));
    assert_eq!(q["conventions"], list(20, 300));
    assert_eq!(q["manifests"], list(50, 300));
    assert_eq!(
        q["env"],
        json!({
            "type": "object", "additionalProperties": false, "properties": {},
            "maxProperties": 20,
            "propertyNames": {"maxLength": 64},
            "patternProperties": {
                "^[A-Za-z_][A-Za-z0-9_]*$": {"type": "string", "minLength": 0, "maxLength": 1000},
            },
        })
    );
    let keys: Vec<&String> = q.as_object().unwrap().keys().collect();
    assert_eq!(keys.len(), 29, "{keys:?}");

    let mut names = Vec::new();
    walk("submit_scout_report", &s, &mut names);
    assert!(names.len() > 20, "{names:?}");
    for name in &names {
        assert!(
            name != "cache_dirs" && !name.starts_with("confined_"),
            "a confinement setting in the scout schema: {name}"
        );
    }
}

/// Milestone 9.1 decision 12: the onboarding scout may propose the tier keys, each with
/// the daemon's limits, and still nothing else.
#[test]
fn mcp_scout_schema_lists_the_new_profile_keys() {
    let s = schema();
    let q = &s["properties"]["profile"]["properties"];
    let text = |max: u64| json!({"type": "string", "minLength": 1, "maxLength": max});
    let list =
        |max: u64, chars: u64| json!({"type": "array", "maxItems": max, "items": text(chars)});
    for key in [
        "build_check",
        "module_test",
        "module_tests",
        "module_graph",
        "slow_tests",
        "timing_tests",
        "toolchain_id",
    ] {
        assert_eq!(q[key], text(2000), "{key}");
    }
    assert_eq!(
        q["module_names"],
        json!({"type": "string", "enum": ["cargo", "dir"]})
    );
    assert_eq!(q["full_triggers"], list(40, 300));
    assert_eq!(q["test_paths"], list(40, 300));
    assert_eq!(q["skip_markers"], list(32, 64));
    assert_eq!(
        q["full_shards"],
        json!({"type": "integer", "minimum": 1, "maximum": 16})
    );
    assert_eq!(q.as_object().unwrap().len(), 29);
}
