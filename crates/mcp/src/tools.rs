//! The tools each agent role sees, with hand-built JSON schemas (decision 5).
//!
//! Every schema is a closed object (`"additionalProperties": false`). The limits here
//! are what the model sees; the engine re-validates every argument on its side and
//! answers `invalid arguments: <field>: <problem>` when one is violated.

use proto::AgentRole;
use rmcp::model::{JsonObject, Tool};
use serde_json::{Value, json};

pub const TASK_DONE: &str = "task_done";
pub const TASK_BLOCKED: &str = "task_blocked";
pub const SUBMIT_REVIEW: &str = "submit_review";

/// The tools `role` may call. `Orchestrator` gets none in this milestone (M9 adds its
/// own).
pub fn tools_for(role: AgentRole) -> Vec<Tool> {
    match role {
        AgentRole::Worker => vec![task_done(), task_blocked()],
        AgentRole::Reviewer => vec![submit_review()],
        AgentRole::Orchestrator => Vec::new(),
    }
}

/// Whether `role` may call `tool`: the gate that keeps a tool outside the role's list
/// from ever reaching the daemon.
pub fn allowed(role: AgentRole, tool: &str) -> bool {
    tools_for(role).iter().any(|t| t.name == tool)
}

/// The role as `anthrex mcp --role` and the error texts spell it.
pub fn role_name(role: AgentRole) -> &'static str {
    match role {
        AgentRole::Orchestrator => "orchestrator",
        AgentRole::Worker => "worker",
        AgentRole::Reviewer => "reviewer",
    }
}

fn task_done() -> Tool {
    Tool::new(
        TASK_DONE,
        "Tell the engine the task is complete and committed. For a tdd task, name the test \
         and the red commit.",
        closed(
            json!({
                "summary": text(4000),
                "test": text(300),
                "red": {"type": "string", "pattern": "^[0-9a-f]{7,40}$"},
            }),
            &["summary"],
        ),
    )
}

fn task_blocked() -> Tool {
    Tool::new(
        TASK_BLOCKED,
        "Tell the engine you cannot continue, and why.",
        closed(
            json!({
                "kind": one_of(&["question", "mis_sized", "environment"]),
                "reason": text(4000),
            }),
            &["reason"],
        ),
    )
}

fn submit_review() -> Tool {
    let finding = closed(
        json!({
            "severity": one_of(&["critical", "important", "minor"]),
            "file": text(500),
            "line": {"type": "integer", "minimum": 1},
            "input": text(2000),
            "text": text(2000),
        }),
        &["severity", "text"],
    );
    Tool::new(
        SUBMIT_REVIEW,
        "Submit your verdict and findings for this review round. Call it once.",
        closed(
            json!({
                "verdict": one_of(&["approve", "changes"]),
                "summary": text(4000),
                "findings": {"type": "array", "maxItems": 50, "items": finding},
            }),
            &["verdict", "summary", "findings"],
        ),
    )
}

/// A string of 1 to `max` characters.
fn text(max: u64) -> Value {
    json!({"type": "string", "minLength": 1, "maxLength": max})
}

fn one_of(values: &[&str]) -> Value {
    json!({"type": "string", "enum": values})
}

/// `{"type":"object","additionalProperties":false,"properties":…,"required":…}`.
fn closed(properties: Value, required: &[&str]) -> JsonObject {
    let mut o = JsonObject::new();
    o.insert("type".into(), json!("object"));
    o.insert("additionalProperties".into(), json!(false));
    o.insert("properties".into(), properties);
    o.insert("required".into(), json!(required));
    o
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(role: AgentRole) -> Vec<String> {
        tools_for(role).iter().map(|t| t.name.to_string()).collect()
    }

    fn tool(role: AgentRole, name: &str) -> Tool {
        tools_for(role)
            .into_iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("{role:?} has no tool {name}"))
    }

    fn schema(role: AgentRole, name: &str) -> Value {
        Value::Object((*tool(role, name).input_schema).clone())
    }

    #[test]
    fn worker_tools_are_task_done_and_task_blocked() {
        assert_eq!(names(AgentRole::Worker), ["task_done", "task_blocked"]);
        assert_eq!(
            tool(AgentRole::Worker, "task_done").description.as_deref(),
            Some(
                "Tell the engine the task is complete and committed. For a tdd task, name \
                 the test and the red commit."
            )
        );
        assert_eq!(
            tool(AgentRole::Worker, "task_blocked")
                .description
                .as_deref(),
            Some("Tell the engine you cannot continue, and why.")
        );
        let done = schema(AgentRole::Worker, "task_done");
        assert_eq!(done["required"], json!(["summary"]));
        let blocked = schema(AgentRole::Worker, "task_blocked");
        assert_eq!(blocked["required"], json!(["reason"]));
    }

    #[test]
    fn reviewer_tool_is_submit_review() {
        assert_eq!(names(AgentRole::Reviewer), ["submit_review"]);
        assert_eq!(
            tool(AgentRole::Reviewer, "submit_review")
                .description
                .as_deref(),
            Some("Submit your verdict and findings for this review round. Call it once.")
        );
        let review = schema(AgentRole::Reviewer, "submit_review");
        assert_eq!(
            review["required"],
            json!(["verdict", "summary", "findings"])
        );
        assert_eq!(
            review["properties"]["findings"]["items"]["required"],
            json!(["severity", "text"])
        );
    }

    #[test]
    fn orchestrator_tools_are_empty() {
        assert!(tools_for(AgentRole::Orchestrator).is_empty());
    }

    /// Every object in every schema, the finding items included, is closed.
    #[test]
    fn every_schema_is_a_closed_object() {
        fn check(path: &str, v: &Value, objects: &mut usize) {
            if v["type"] == "object" {
                *objects += 1;
                assert_eq!(
                    v["additionalProperties"],
                    json!(false),
                    "{path} is not closed: {v}"
                );
                assert!(v["properties"].is_object(), "{path} has no properties");
                for (k, p) in v["properties"].as_object().unwrap() {
                    check(&format!("{path}.{k}"), p, objects);
                }
            }
            if let Some(items) = v.get("items") {
                check(&format!("{path}[]"), items, objects);
            }
        }
        let mut objects = 0;
        for role in [AgentRole::Worker, AgentRole::Reviewer] {
            for t in tools_for(role) {
                let s = Value::Object((*t.input_schema).clone());
                assert_eq!(s["type"], "object", "{}", t.name);
                check(&t.name, &s, &mut objects);
            }
        }
        assert_eq!(objects, 4, "three tool schemas and the finding object");
    }

    #[test]
    fn schema_limits() {
        let text =
            |min: u64, max: u64| json!({"type": "string", "minLength": min, "maxLength": max});

        let done = schema(AgentRole::Worker, "task_done");
        assert_eq!(done["properties"]["summary"], text(1, 4000));
        assert_eq!(done["properties"]["test"], text(1, 300));
        assert_eq!(
            done["properties"]["red"],
            json!({"type": "string", "pattern": "^[0-9a-f]{7,40}$"})
        );

        let blocked = schema(AgentRole::Worker, "task_blocked");
        assert_eq!(
            blocked["properties"]["kind"],
            json!({"type": "string", "enum": ["question", "mis_sized", "environment"]})
        );
        assert_eq!(blocked["properties"]["reason"], text(1, 4000));

        let review = schema(AgentRole::Reviewer, "submit_review");
        let props = &review["properties"];
        assert_eq!(
            props["verdict"],
            json!({"type": "string", "enum": ["approve", "changes"]})
        );
        assert_eq!(props["summary"], text(1, 4000));
        assert_eq!(props["findings"]["type"], "array");
        assert_eq!(props["findings"]["maxItems"], 50);
        let finding = &props["findings"]["items"]["properties"];
        assert_eq!(
            finding["severity"],
            json!({"type": "string", "enum": ["critical", "important", "minor"]})
        );
        assert_eq!(finding["file"], text(1, 500));
        assert_eq!(finding["line"], json!({"type": "integer", "minimum": 1}));
        assert_eq!(finding["input"], text(1, 2000));
        assert_eq!(finding["text"], text(1, 2000));
    }
}
