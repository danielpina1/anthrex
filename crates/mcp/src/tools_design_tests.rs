//! Milestone 9.6 task M9.6.6: the design flow's tools (Interfaces "MCP tools"): which
//! role sees which, and their schemas. The daemon parses every call again
//! (`daemon::run::orch::tools::parse_call`).

use crate::tools::{allowed, role_name, tools_for};
use proto::AgentRole;
use serde_json::{Value, json};

const EVERY_ROLE: [AgentRole; 10] = [
    AgentRole::Orchestrator,
    AgentRole::Worker,
    AgentRole::Reviewer,
    AgentRole::Scout,
    AgentRole::Planner,
    AgentRole::Decider,
    AgentRole::Racer,
    AgentRole::TestWriter,
    AgentRole::Brainstormer,
    AgentRole::DocReviewer,
];

fn names(role: AgentRole) -> Vec<String> {
    tools_for(role).iter().map(|t| t.name.to_string()).collect()
}

fn schema(role: AgentRole, name: &str) -> Value {
    let tool = (tools_for(role).into_iter().find(|t| t.name == name))
        .unwrap_or_else(|| panic!("{} has no tool {name}", role_name(role)));
    Value::Object((*tool.input_schema).clone())
}

/// The orchestrator gains `start_brainstorm`, `submit_doc` and `get_doc` after its seven;
/// a brainstormer has `submit_doc` only; a document reviewer `get_doc` and
/// `submit_findings`. Every other role is unchanged, and no tool is allowed outside its
/// role's list.
#[test]
fn each_role_gets_exactly_its_tools() {
    let expected: [(AgentRole, &[&str]); 10] = [
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
                "start_brainstorm",
                "submit_doc",
                "get_doc",
            ],
        ),
        (
            AgentRole::Worker,
            &["task_done", "task_blocked", "task_note"],
        ),
        (AgentRole::Reviewer, &["submit_review"]),
        (AgentRole::Scout, &["submit_scout_report"]),
        (AgentRole::Planner, &["get_context", "submit_epic"]),
        (AgentRole::Decider, &[]),
        (
            AgentRole::Racer,
            &["task_done", "task_blocked", "task_note"],
        ),
        (
            AgentRole::TestWriter,
            &["task_done", "task_blocked", "task_note"],
        ),
        (AgentRole::Brainstormer, &["submit_doc"]),
        (AgentRole::DocReviewer, &["get_doc", "submit_findings"]),
    ];
    let every: Vec<String> = EVERY_ROLE.iter().flat_map(|r| names(*r)).collect();
    for (role, tools) in expected {
        assert_eq!(names(role), tools, "{}", role_name(role));
        for tool in &every {
            assert_eq!(
                allowed(role, tool),
                tools.contains(&tool.as_str()),
                "{} and {tool}",
                role_name(role)
            );
        }
    }
    // The user's design rule: no tool approves, edits a gate, rethinks, goes back or
    // rejects.
    for tool in &every {
        for word in ["approve", "accept", "reject", "rethink", "gate", "merge"] {
            assert!(!tool.contains(word), "{tool}");
        }
    }
}

/// Interfaces "MCP tools": each design tool's arguments and their limits, as the model
/// sees them. `submit_doc` takes only the kinds its role may submit.
#[test]
fn design_tool_schemas_are_exact() {
    let orch = AgentRole::Orchestrator;
    assert_eq!(
        schema(orch, "start_brainstorm"),
        json!({
            "type": "object", "additionalProperties": false,
            "properties": {"answers": {"type": "string", "minLength": 0, "maxLength": 8192}},
            "required": ["answers"],
        })
    );
    let response = json!({
        "type": "object", "additionalProperties": false,
        "properties": {
            "id": {"type": "string", "pattern": "^[A-Za-z0-9][A-Za-z0-9-]{0,15}$"},
            "answer": {"type": "string", "minLength": 1, "maxLength": 2000},
        },
        "required": ["id", "answer"],
    });
    let responses = json!({"type": "array", "maxItems": 40, "items": response});
    assert_eq!(
        schema(orch, "submit_doc"),
        json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "kind": {"type": "string", "enum": ["brainstorm", "spec"]},
                "text": {"type": "string", "minLength": 1, "maxLength": 65536},
                "ready": {"type": "boolean"},
                "amend": {"type": "boolean"},
                "responses": responses,
            },
            "required": ["kind", "text"],
        })
    );
    assert_eq!(
        schema(AgentRole::Brainstormer, "submit_doc"),
        json!({
            "type": "object", "additionalProperties": false,
            "properties": {
                "kind": {"type": "string", "enum": ["brainstorm_draft"]},
                "text": {"type": "string", "minLength": 1, "maxLength": 12288},
            },
            "required": ["kind", "text"],
        })
    );
    let get_doc = json!({
        "type": "object", "additionalProperties": false,
        "properties": {
            "kind": {"type": "string", "enum": ["brainstorm_draft", "brainstorm", "spec", "plan"]},
            "version": {"type": "integer", "minimum": 1},
            "from": {"type": "string", "pattern": "^[A-Za-z0-9_-]{1,32}$"},
        },
        "required": ["kind"],
    });
    assert_eq!(schema(orch, "get_doc"), get_doc);
    // Ruling T5-1 (task M9.6.10): the reviewer also names its spec's review draft.
    let mut reviewers = get_doc.clone();
    reviewers["properties"]["draft"] = json!({"type": "integer", "minimum": 1});
    assert_eq!(schema(AgentRole::DocReviewer, "get_doc"), reviewers);
    assert_eq!(
        schema(AgentRole::DocReviewer, "submit_findings"),
        json!({
            "type": "object", "additionalProperties": false,
            "properties": {"findings": {"type": "array", "maxItems": 40, "items": {
                "type": "object", "additionalProperties": false,
                "properties": {
                    "id": {"type": "string", "pattern": "^[A-Za-z0-9][A-Za-z0-9-]{0,15}$"},
                    "severity": {"type": "string", "enum": ["blocking", "minor"]},
                    "place": {"type": "string", "minLength": 1, "maxLength": 300},
                    "text": {"type": "string", "minLength": 1, "maxLength": 2000},
                },
                "required": ["id", "severity", "place", "text"],
            }}},
            "required": ["findings"],
        })
    );
    // `edit_plan` takes the same `responses`; `plan_task` takes `covers` (decision 17),
    // for the orchestrator and the sub-planner alike.
    assert_eq!(
        schema(orch, "edit_plan")["properties"]["responses"],
        responses
    );
    let covers = json!({"type": "array", "maxItems": 32,
        "items": {"type": "string", "pattern": "^R[0-9]+$", "maxLength": 8}});
    for (role, tool) in [(orch, "edit_plan"), (AgentRole::Planner, "submit_epic")] {
        let edit = &schema(role, tool)["properties"]["edits"]["items"]["properties"];
        for task in [&edit["task"], &edit["into"]["items"]] {
            assert_eq!(task["properties"]["covers"], covers, "{tool}");
            let required = task["required"].as_array().unwrap();
            assert!(!required.contains(&json!("covers")), "{tool}");
        }
    }
}

/// The design tools' descriptions: what each does and when (DF §9).
#[test]
fn design_tools_say_what_they_do() {
    let described = |role: AgentRole, name: &str| {
        let tool = tools_for(role)
            .into_iter()
            .find(|t| t.name == name)
            .unwrap();
        tool.description.as_deref().unwrap_or_default().to_string()
    };
    let orch = AgentRole::Orchestrator;
    assert_eq!(
        described(orch, "start_brainstorm"),
        "Start the two brainstormers with the user's answers to your questions (empty if \
         they skipped). Only in the brainstorming phase. Returns at once."
    );
    assert_eq!(
        described(orch, "submit_doc"),
        "Submit a design document: the merged brainstorm report (kind brainstorm) or the \
         spec (kind spec). A spec with ready false goes to review; with ready true it opens \
         the gate, and responses must answer every finding of the latest review."
    );
    assert_eq!(
        described(AgentRole::Brainstormer, "submit_doc"),
        "Submit your brainstorm draft (kind brainstorm_draft) in the six-section template. \
         If it is refused, fix what the refusal names and submit again."
    );
    assert_eq!(
        described(orch, "get_doc"),
        "Read a design document: the latest of a kind, one version, or a brainstormer's \
         draft (from)."
    );
    assert_eq!(
        described(AgentRole::DocReviewer, "get_doc"),
        "Read a design document: the latest of a kind, one version, or the spec's review \
         draft your first message names (draft)."
    );
    assert_eq!(
        described(AgentRole::DocReviewer, "submit_findings"),
        "Submit your review's findings once, then stop. Use blocking only for placeholders, \
         contradictions, untestable requirements, scope beyond the approved approach, dropped \
         brainstorm decisions, a requirement without an acceptance check, or a plan task that \
         does not deliver its covers."
    );
}
