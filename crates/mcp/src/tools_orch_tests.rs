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

/// Milestone 9.3 task M9.3.7: `edit_plan` names its `iterate`, which comes alone (the
/// schema cannot say so). `edits` may be left out in every call (fix round 1).
const EDIT_PLAN: &str = "Apply plan edits as one batch. Set submit to open the plan gate. Add \
    a summary for the user when the run is complete. Set iterate, with no edits and nothing \
    else, to start a round the user asked for. Returns at once.";

/// Milestone 9.3 decision 29.
const START_GOAL: &str = "Start a new goal on this orchestrator when the user gives you one; \
    only while your last run has ended.";

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
        ("edit_plan", EDIT_PLAN),
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
        ("start_goal", START_GOAL),
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
                "start_goal",
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
    // `into`'s plan_task and route: seven objects with the tool's own. start_goal
    // (milestone 9.3) is the orchestrator's thirteenth.
    assert_eq!(
        counts,
        [
            ("orchestrator", 13),
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
        checked, 10,
        "seven orchestrator tools, two planner tools, task_note"
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

/// Milestone 9.2 decisions 30 and 31: `plan_edit` takes `reply_comment` with `pr`,
/// `thread` and `body`, and `plan_task` takes `addresses` as a list of strings, for
/// the orchestrator and (bounded the same way, refused by the daemon) the sub-planner.
#[test]
fn mcp_schema_has_reply_comment_and_addresses() {
    for (role, tool, edits) in [
        (AgentRole::Orchestrator, "edit_plan", 60),
        (AgentRole::Planner, "submit_epic", 60),
    ] {
        let tools = tools_for(role);
        let found = tools.iter().find(|t| t.name == tool).unwrap();
        let schema = Value::Object((*found.input_schema).clone());
        assert_eq!(schema["properties"]["edits"]["maxItems"], edits);
        let edit = &schema["properties"]["edits"]["items"]["properties"];
        let ops: Vec<&str> = (edit["op"]["enum"].as_array().unwrap().iter())
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(ops.last(), Some(&"reply_comment"), "{tool}: {ops:?}");
        assert_eq!(edit["pr"], json!({"type": "integer", "minimum": 1}));
        assert_eq!(
            edit["thread"],
            json!({"type": "string", "minLength": 1, "maxLength": 64})
        );
        assert_eq!(
            edit["body"],
            json!({"type": "string", "minLength": 1, "maxLength": 4000})
        );
        for task in [&edit["task"], &edit["into"]["items"]] {
            assert_eq!(
                task["properties"]["addresses"],
                json!({"type": "array", "maxItems": 20,
                       "items": {"type": "string", "minLength": 1, "maxLength": 64}}),
                "{tool}"
            );
            let required = task["required"].as_array().unwrap();
            assert!(!required.contains(&json!("addresses")));
        }
    }
}

/// Milestone 9.3 decision 29: the orchestrator lists seven tools, `start_goal` last,
/// taking one required `goal` of 1 to 16,384 characters.
#[test]
fn orchestrator_tools_list_start_goal_last() {
    let tools = tools_for(AgentRole::Orchestrator);
    assert_eq!(tools.len(), 7);
    let last = tools.last().unwrap();
    assert_eq!(last.name, "start_goal");
    assert_eq!(last.description.as_deref(), Some(START_GOAL));
    assert_eq!(
        Value::Object((*last.input_schema).clone()),
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": {"goal": {"type": "string", "minLength": 1, "maxLength": proto::GOAL_MAX_CHARS}},
            "required": ["goal"],
        })
    );
    assert!(allowed(AgentRole::Orchestrator, "start_goal"));
}

/// Decisions 29 and 30, and task M9.3.4a's carried item: `edit_plan` takes `iterate`
/// (1 to 16,384 characters), and `edits` is not required, because the daemon's
/// `parse_call` reads a missing `edits` as an empty batch (task M9.3.7's fix round 1),
/// so this plain schema is exact. `iterate` is not a `plan_edit` op.
#[test]
fn edit_plan_takes_iterate() {
    let tools = tools_for(AgentRole::Orchestrator);
    let edit_plan = tools.iter().find(|t| t.name == "edit_plan").unwrap();
    let schema = Value::Object((*edit_plan.input_schema).clone());
    assert_eq!(
        schema["properties"]["iterate"],
        json!({"type": "string", "minLength": 1, "maxLength": proto::GOAL_MAX_CHARS})
    );
    assert_eq!(schema["required"], json!([]));
    let mut keys: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(keys, ["edits", "iterate", "submit", "summary"]);
    let ops = &schema["properties"]["edits"]["items"]["properties"]["op"]["enum"];
    assert!(
        !ops.as_array().unwrap().contains(&json!("iterate")),
        "{ops}"
    );
    assert_eq!(edit_plan.description.as_deref(), Some(EDIT_PLAN));
    // submit_epic is unchanged: edits stay required, and it has no iterate.
    let planner = tools_for(AgentRole::Planner);
    let submit_epic = planner.iter().find(|t| t.name == "submit_epic").unwrap();
    let schema = Value::Object((*submit_epic.input_schema).clone());
    assert_eq!(schema["required"], json!(["edits"]));
    assert_eq!(schema["properties"]["iterate"], Value::Null);
}

/// Decision 29 (KG §4): a sub-planner can call neither `start_goal` nor `iterate`. Its
/// tools are unchanged, and `start_goal` never reaches the daemon from its window.
#[test]
fn planners_see_neither() {
    assert_eq!(names(AgentRole::Planner), ["get_context", "submit_epic"]);
    assert!(!allowed(AgentRole::Planner, "start_goal"));
    for role in ROLES.into_iter().filter(|r| *r != AgentRole::Orchestrator) {
        assert!(!allowed(role, "start_goal"), "{}", role_name(role));
    }
}
