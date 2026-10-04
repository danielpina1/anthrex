//! Milestone 9.6 task M9.6.6: `plan_task.covers` and `edit_plan.responses`, bounded by
//! the daemon, and the role that may make each design tool call (Interfaces "MCP
//! tools"); each tool's own arguments are `tools_tests_design_args.rs`'. The MCP schemas
//! are only what the model sees; these are the checks a forged call meets. Pure.

use proto::{AgentRole, FindingAnswer, PlanEdit};
use serde_json::{Value, json};

use super::*;

const ROLES: [AgentRole; 10] = [
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

fn orch(tool: &str, args: Value) -> Result<OrchCall, String> {
    parse_call(AgentRole::Orchestrator, tool, &args)
}

fn task(covers: Value) -> Value {
    json!({"id": "t1", "title": "T", "size": "S", "owns": ["a/**"], "brief": "B",
           "acceptance": ["A"], "covers": covers})
}

fn answer(id: &str, answer: &str) -> FindingAnswer {
    FindingAnswer {
        id: id.into(),
        answer: answer.into(),
    }
}

/// Decision 17: `covers` holds at most 32 ids of at most 8 characters, each `R<n>`;
/// decision 20: `edit_plan`'s `responses` answer findings, `"fixed"` or `"kept: <reason>"`,
/// at most 40, each id once. Both are optional and default to empty.
#[test]
fn covers_and_responses_parse_and_are_bounded() {
    let covers: Vec<String> = (1..=32).map(|n| format!("R{n}")).collect();
    let call = orch(
        "edit_plan",
        json!({"edits": [
            {"op": "add_task", "task": task(json!(covers))},
            {"op": "split_task", "task_id": "t0", "into": [task(json!(["R1234567"]))]},
        ], "submit": true, "responses": [{"id": "F1", "answer": "fixed"},
            {"id": "f-2", "answer": "kept: the user asked for it"}]}),
    );
    let Ok(OrchCall::EditPlan {
        edits, responses, ..
    }) = call
    else {
        panic!("{call:?}");
    };
    match &edits[..] {
        [PlanEdit::AddTask { task }, PlanEdit::SplitTask { into, .. }] => {
            assert_eq!(task.covers, covers);
            assert_eq!(into[0].covers, ["R1234567"]);
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        responses,
        [
            answer("F1", "fixed"),
            answer("f-2", "kept: the user asked for it")
        ]
    );
    // Absent: empty.
    let Ok(OrchCall::EditPlan {
        edits, responses, ..
    }) = orch(
        "edit_plan",
        json!({"edits": [{"op": "add_task", "task": task(json!([]))}]}),
    )
    else {
        panic!("a plain edit_plan parses");
    };
    assert!(responses.is_empty());
    let PlanEdit::AddTask { task: plain } = &edits[0] else {
        panic!("{edits:?}");
    };
    assert!(plain.covers.is_empty());

    let refused = |covers: Value| {
        orch(
            "edit_plan",
            json!({"edits": [{"op": "add_task", "task": task(covers)}]}),
        )
        .expect_err("refused")
    };
    for bad in [
        json!(["R"]),
        json!(["r4"]),
        json!(["4"]),
        json!(["R4a"]),
        json!(["R-1"]),
    ] {
        assert_eq!(
            refused(bad.clone()),
            "invalid arguments: edits[0]: task: covers entries look like R4",
            "{bad}"
        );
    }
    let split = orch(
        "edit_plan",
        json!({"edits": [{"op": "split_task", "task_id": "t0", "into": [task(json!(["X1"]))]}]}),
    );
    assert_eq!(
        split,
        Err("invalid arguments: edits[0]: into[0]: covers entries look like R4".into())
    );
    let mut over = covers.clone();
    over.push("R33".into());
    assert_eq!(
        refused(json!(over)),
        "invalid arguments: edits[0]: task: covers: at most 32 items"
    );
    assert_eq!(
        refused(json!(["R12345678"])),
        "invalid arguments: edits[0]: task: covers[0]: must be 1 to 8 characters"
    );
    assert_eq!(
        refused(json!("R1")),
        "invalid arguments: edits[0]: task: covers: must be an array"
    );
    // Ruling T6-1 (m2): an id once.
    assert_eq!(
        refused(json!(["R1", "R2", "R1"])),
        "invalid arguments: edits[0]: task: covers lists R1 twice"
    );
    // The sub-planner's tasks are bounded the same way.
    let epic = parse_call(
        AgentRole::Planner,
        "submit_epic",
        &json!({"edits": [{"op": "add_task", "task": task(json!(["R1", "x"]))}]}),
    );
    assert_eq!(
        epic,
        Err("invalid arguments: edits[0]: task: covers entries look like R4".into())
    );

    let responses = |list: Value| orch("edit_plan", json!({"submit": true, "responses": list}));
    let forty: Vec<Value> = (1..=40)
        .map(|n| json!({"id": format!("F{n}"), "answer": "fixed"}))
        .collect();
    assert!(responses(json!(forty)).is_ok());
    let mut more = forty.clone();
    more.push(json!({"id": "F41", "answer": "fixed"}));
    for (list, problem) in [
        (json!(more), "responses: at most 40 items"),
        (json!({"id": "F1"}), "responses: must be an array"),
        (json!([{"id": "F1"}]), "responses[0].answer: required"),
        (
            json!([{"id": "F1", "answer": "fixed", "why": "x"}]),
            "responses[0].why: unknown field",
        ),
        (
            json!([{"id": "F 1", "answer": "fixed"}]),
            "responses[0].id: must match ^[A-Za-z0-9][A-Za-z0-9-]{0,15}$",
        ),
        (
            json!([{"id": "F1", "answer": "done"}]),
            "responses[0].answer: must be \"fixed\" or \"kept: <reason>\"",
        ),
        (
            json!([{"id": "F1", "answer": "kept:  "}]),
            "responses[0].answer: must be \"fixed\" or \"kept: <reason>\"",
        ),
        // Ruling T6-1 (m1): exactly `fixed`, or `kept: ` with its space.
        (
            json!([{"id": "F1", "answer": "kept:no space"}]),
            "responses[0].answer: must be \"fixed\" or \"kept: <reason>\"",
        ),
        (
            json!([{"id": "F1", "answer": "fixed "}]),
            "responses[0].answer: must be \"fixed\" or \"kept: <reason>\"",
        ),
        (
            json!([{"id": "F1", "answer": "fixed"}, {"id": "F1", "answer": "kept: no"}]),
            "responses[1].id: F1 is answered twice",
        ),
        (
            json!([{"id": "F1", "answer": format!("kept: {}", "x".repeat(2000))}]),
            "responses[0].answer: must be 1 to 2000 characters",
        ),
    ] {
        assert_eq!(
            responses(list.clone()),
            Err(format!("invalid arguments: {problem}")),
            "{list}"
        );
    }
}

/// Interfaces "MCP tools": each design tool is refused to every role but its own, with
/// `tool <tool> is not available to the <role> role`, before its arguments are read.
/// The orchestrator's and the sub-planner's earlier tools stay theirs.
#[test]
fn a_tool_outside_its_role_is_refused_exactly() {
    let owners: [(&str, &[AgentRole]); 5] = [
        ("start_brainstorm", &[AgentRole::Orchestrator]),
        (
            "submit_doc",
            &[AgentRole::Orchestrator, AgentRole::Brainstormer],
        ),
        (
            "get_doc",
            &[AgentRole::Orchestrator, AgentRole::DocReviewer],
        ),
        ("submit_findings", &[AgentRole::DocReviewer]),
        ("edit_plan", &[AgentRole::Orchestrator]),
    ];
    for (tool, roles) in owners {
        for role in ROLES {
            let parsed = parse_call(role, tool, &json!("not even an object"));
            let text = format!(
                "tool {tool} is not available to the {} role",
                super::super::json::label(&role)
            );
            if roles.contains(&role) {
                assert_eq!(
                    parsed,
                    Err("invalid arguments: arguments: must be an object".into()),
                    "{tool} from {role:?}"
                );
            } else {
                assert_eq!(parsed, Err(text), "{tool} from {role:?}");
            }
        }
    }
    // The design agents get none of the earlier tools.
    for role in [AgentRole::Brainstormer, AgentRole::DocReviewer] {
        for tool in [
            "get_context",
            "spawn_scout",
            "spawn_subplanner",
            "edit_plan",
            "run_status",
            "task_result",
            "start_goal",
            "submit_epic",
            "task_note",
            "submit_scout_report",
        ] {
            assert!(
                parse_call(role, tool, &json!({})).is_err(),
                "{tool} {role:?}"
            );
        }
    }
    assert_eq!(
        parse_call(
            AgentRole::Brainstormer,
            "get_doc",
            &json!({"kind": "brainstorm_draft"})
        ),
        Err("tool get_doc is not available to the brainstormer role".into())
    );
    assert_eq!(
        parse_call(AgentRole::DocReviewer, "submit_doc", &json!({})),
        Err("tool submit_doc is not available to the doc_reviewer role".into())
    );
}
