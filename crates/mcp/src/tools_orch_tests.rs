//! Milestone 9 task M9.11: the orchestrator's and the sub-planner's tools, the worker's
//! `task_note`, and the role lists they belong to (decision 15, Interfaces "MCP").

use crate::tools::{allowed, role_name, tools_for};
use proto::AgentRole;
use rmcp::model::Tool;
use serde_json::{Value, json};

const ROLES: [AgentRole; 6] = [
    AgentRole::Orchestrator,
    AgentRole::Worker,
    AgentRole::Reviewer,
    AgentRole::Scout,
    AgentRole::Planner,
    AgentRole::Decider,
];

fn names(role: AgentRole) -> Vec<String> {
    tools_for(role).iter().map(|t| t.name.to_string()).collect()
}

fn described(tools: &[Tool]) -> Vec<(String, String)> {
    tools
        .iter()
        .map(|t| {
            let d = t.description.as_deref().unwrap_or_default().to_string();
            (t.name.to_string(), d)
        })
        .collect()
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(n, d)| (n.to_string(), d.to_string()))
        .collect()
}

const GET_CONTEXT: &str = "Read the run's context: the repository profile, the models you can \
    route to, the limits, scout reports, epics and the plan so far.";

#[test]
fn tools_for_orchestrator_and_planner_are_exact() {
    let orchestrator = [
        ("get_context", GET_CONTEXT),
        (
            "spawn_scout",
            "Start a read-only scout on one area with one question. Returns at once; its \
             report appears in run_status and get_context.",
        ),
        (
            "spawn_subplanner",
            "Start a sub-planner for one epic with its own area, or a fresh one to re-plan \
             an existing epic. Returns at once.",
        ),
        (
            "edit_plan",
            "Apply plan edits as one batch. Set submit to open the plan gate. Add a summary \
             for the user when the run is complete. Returns at once.",
        ),
        (
            "run_status",
            "Read the run digest. With since and wait_secs, wait up to wait_secs seconds (at \
             most 50) for it to change.",
        ),
        (
            "task_result",
            "Read everything about one task: brief, commits, diff size, checks, proofs, \
             reviews, agent rounds and any report.",
        ),
    ];
    assert_eq!(
        described(&tools_for(AgentRole::Orchestrator)),
        pairs(&orchestrator)
    );
    let planner = [
        ("get_context", GET_CONTEXT),
        (
            "submit_epic",
            "Submit your epic's tasks as one batch of plan edits. If it returns errors, fix \
             them and call it again. When it is accepted you are done.",
        ),
    ];
    assert_eq!(described(&tools_for(AgentRole::Planner)), pairs(&planner));
}

/// No role holds a tool it must not have: nothing approves, accepts, merges or
/// overrides; a decider has none (M9.2); each role's tools are only its own.
#[test]
fn no_role_gets_a_tool_it_must_not_have() {
    let expected: [(AgentRole, &[&str]); 6] = [
        (
            AgentRole::Orchestrator,
            &[
                "get_context",
                "spawn_scout",
                "spawn_subplanner",
                "edit_plan",
                "run_status",
                "task_result",
            ],
        ),
        (AgentRole::Planner, &["get_context", "submit_epic"]),
        (
            AgentRole::Worker,
            &["task_done", "task_blocked", "task_note"],
        ),
        (AgentRole::Reviewer, &["submit_review"]),
        (AgentRole::Scout, &["submit_scout_report"]),
        (AgentRole::Decider, &[]),
    ];
    let every: Vec<String> = ROLES.iter().flat_map(|r| names(*r)).collect();
    for (role, tools) in expected {
        assert_eq!(names(role), tools, "{role:?}");
        for tool in &every {
            assert_eq!(
                allowed(role, tool),
                tools.contains(&tool.as_str()),
                "{} and {tool}",
                role_name(role)
            );
        }
    }
    for tool in &every {
        for word in ["approve", "accept", "merge", "override"] {
            assert!(!tool.contains(word), "{tool}");
        }
    }
}

/// Every object in every schema of every role, at every level (properties, array
/// items, `oneOf` branches, pattern properties), is closed.
#[test]
fn every_schema_is_closed_at_every_level() {
    fn walk(path: &str, v: &Value, objects: &mut usize) {
        if v["type"] == "object" {
            *objects += 1;
            assert_eq!(
                v["additionalProperties"],
                json!(false),
                "{path} is not closed: {v}"
            );
            assert!(v["properties"].is_object(), "{path} has no properties");
        }
        for key in ["properties", "patternProperties"] {
            if let Some(map) = v.get(key).and_then(Value::as_object) {
                for (k, p) in map {
                    walk(&format!("{path}.{k}"), p, objects);
                }
            }
        }
        if let Some(items) = v.get("items") {
            walk(&format!("{path}[]"), items, objects);
        }
        for key in ["oneOf", "anyOf", "allOf"] {
            if let Some(branches) = v.get(key).and_then(Value::as_array) {
                for (i, b) in branches.iter().enumerate() {
                    walk(&format!("{path}.{key}[{i}]"), b, objects);
                }
            }
        }
    }
    let mut counts = Vec::new();
    for role in ROLES {
        let mut objects = 0;
        for t in tools_for(role) {
            let s = Value::Object((*t.input_schema).clone());
            assert_eq!(s["type"], "object", "{}", t.name);
            walk(&t.name, &s, &mut objects);
        }
        counts.push((role_name(role), objects));
    }
    // edit_plan and submit_epic each hold plan_edit, its plan_task and route, and
    // `into`'s plan_task and route: seven objects with the tool's own.
    assert_eq!(
        counts,
        [
            ("orchestrator", 12),
            ("worker", 3),
            ("reviewer", 2),
            ("scout", 4),
            ("planner", 8),
            ("decider", 0),
        ]
    );
}

/// Each tool's schema equals the Interfaces table's, checked in as a fixture built from
/// the table alone.
#[test]
fn schemas_match_the_interface_table() {
    let fixture: Value =
        serde_json::from_str(include_str!("fixtures/orch_tools.json")).expect("the fixture");
    let mut checked = 0;
    for (role, key) in [
        (AgentRole::Orchestrator, "orchestrator"),
        (AgentRole::Planner, "planner"),
        (AgentRole::Worker, "worker"),
    ] {
        for want in fixture[key].as_array().expect("a list") {
            let name = want["name"].as_str().unwrap();
            let tool = tools_for(role)
                .into_iter()
                .find(|t| t.name == name)
                .unwrap_or_else(|| panic!("{key} has no tool {name}"));
            assert_eq!(
                tool.description.as_deref(),
                want["description"].as_str(),
                "{key}.{name}"
            );
            let got = Value::Object((*tool.input_schema).clone());
            assert_eq!(got, want["inputSchema"], "{key}.{name}");
            checked += 1;
        }
    }
    assert_eq!(
        checked, 9,
        "six orchestrator tools, two planner tools, task_note"
    );
}

/// The orchestrator's and the sub-planner's schemas stay plain (task M9.1.11): no
/// `oneOf`, `anyOf` or `allOf` at any depth, except `plan_edit.to`'s `oneOf` (a list
/// of task ids, or `running` / `stage:<n>`), which milestone 9 shipped and which is
/// recorded in M9.1's "Implementation notes" as the one exception.
#[test]
fn no_orchestrator_schema_combines_schemas_but_a_messages_to() {
    fn walk(path: &str, v: &Value, found: &mut Vec<String>) {
        match v {
            Value::Object(map) => {
                for (k, child) in map {
                    if ["oneOf", "anyOf", "allOf"].contains(&k.as_str()) {
                        found.push(format!("{path}.{k}"));
                    }
                    walk(&format!("{path}.{k}"), child, found);
                }
            }
            Value::Array(items) => {
                for (i, child) in items.iter().enumerate() {
                    walk(&format!("{path}[{i}]"), child, found);
                }
            }
            _ => {}
        }
    }
    for (role, exception) in [
        (AgentRole::Orchestrator, "edit_plan"),
        (AgentRole::Planner, "submit_epic"),
    ] {
        let mut found = Vec::new();
        for t in tools_for(role) {
            walk(
                &t.name,
                &Value::Object((*t.input_schema).clone()),
                &mut found,
            );
        }
        assert_eq!(
            found,
            [format!(
                "{exception}.properties.edits.items.properties.to.oneOf"
            )],
            "{}",
            role_name(role)
        );
    }
}

/// Decision 43: `plan_task` takes `stage`, `atomic` and `atomic_reason`, and
/// `plan_edit` takes `stage` (for `amend_task`), as plain types.
#[test]
fn plan_task_and_plan_edit_take_stage_and_atomic() {
    let tools = tools_for(AgentRole::Orchestrator);
    let edit_plan = tools.iter().find(|t| t.name == "edit_plan").unwrap();
    let schema = Value::Object((*edit_plan.input_schema).clone());
    let edit = &schema["properties"]["edits"]["items"]["properties"];
    let stage = json!({"type": "integer", "minimum": 1, "maximum": proto::STAGES_MAX});
    assert_eq!(edit["stage"], stage);
    assert_eq!(
        edit["atomic"],
        Value::Null,
        "atomic is set when a task is added"
    );
    for task in [&edit["task"], &edit["into"]["items"]] {
        let props = &task["properties"];
        assert_eq!(props["stage"], stage);
        assert_eq!(props["atomic"], json!({"type": "boolean"}));
        assert_eq!(
            props["atomic_reason"],
            json!({"type": "string", "minLength": 1, "maxLength": 300})
        );
    }
}
