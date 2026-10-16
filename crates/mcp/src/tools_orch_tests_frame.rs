//! Milestone 9.17 task M9.17.5 (Interfaces "`mcp`"): `edit_plan` and `submit_epic` take
//! the plan frame, `decided` and `out_of_scope`; `plan_edit` takes `replaces`. Kept apart
//! from `tools_orch_tests.rs`, which is near the 600-line cap (pre-flight ruling C14).

use crate::tools::tools_for;
use proto::AgentRole;
use serde_json::{Value, json};

fn schema(role: AgentRole, name: &str) -> Value {
    let tools = tools_for(role);
    let tool = tools.iter().find(|t| t.name == name).unwrap();
    Value::Object((*tool.input_schema).clone())
}

fn decided() -> Value {
    json!({
        "type": "array",
        "maxItems": 20,
        "items": {
            "type": "object",
            "additionalProperties": false,
            "properties": {
                "question": {"type": "string", "minLength": 1, "maxLength": 300},
                "answer": {"type": "string", "minLength": 1, "maxLength": 500},
            },
            "required": ["question", "answer"],
        },
    })
}

fn out_of_scope() -> Value {
    json!({
        "type": "array",
        "maxItems": 20,
        "items": {"type": "string", "minLength": 1, "maxLength": 300},
    })
}

#[test]
fn edit_plan_and_submit_epic_take_decided_and_out_of_scope() {
    for (role, name) in [
        (AgentRole::Orchestrator, "edit_plan"),
        (AgentRole::Planner, "submit_epic"),
    ] {
        let schema = schema(role, name);
        let props = &schema["properties"];
        assert_eq!(props["decided"], decided(), "{name}");
        assert_eq!(props["out_of_scope"], out_of_scope(), "{name}");
        // Neither key is required by the schema: the engine refuses a submit without
        // them, in words the model reads (decision 14).
        let required = schema["required"].as_array().unwrap();
        assert!(
            !required.contains(&json!("decided")) && !required.contains(&json!("out_of_scope")),
            "{name}: {required:?}"
        );
    }
}

#[test]
fn plan_edit_takes_replaces() {
    for (role, name) in [
        (AgentRole::Orchestrator, "edit_plan"),
        (AgentRole::Planner, "submit_epic"),
    ] {
        let schema = schema(role, name);
        let edit = &schema["properties"]["edits"]["items"];
        assert_eq!(
            edit["properties"]["replaces"],
            json!({
                "type": "array",
                "maxItems": 12,
                "items": {"type": "string", "minLength": 1, "maxLength": 16},
            }),
            "{name}"
        );
        assert_eq!(edit["required"], json!(["op"]), "{name}");
        // A task as `add_task` takes it has no `replaces` (decision 22).
        assert_eq!(
            edit["properties"]["task"]["properties"]["replaces"],
            Value::Null
        );
    }
}
