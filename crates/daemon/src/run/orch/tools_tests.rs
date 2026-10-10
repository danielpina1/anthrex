//! Task M9.6: the orchestrator's, sub-planners' and `task_note`'s arguments (Interfaces
//! "MCP", checked again by the daemon). Pure.

use proto::{AgentRole, MessageKind, MessageTarget, PlanEdit, TaskNoteKind};
use serde_json::{Value, json};

use super::*;

fn orch(tool: &str, args: Value) -> Result<OrchCall, String> {
    parse_call(AgentRole::Orchestrator, tool, &args)
}

#[test]
fn parse_call_for_each_tool() {
    assert_eq!(
        orch("get_context", json!({})),
        Ok(OrchCall::GetContext { scouts: None })
    );
    assert_eq!(
        parse_call(
            AgentRole::Planner,
            "get_context",
            &json!({"scouts": ["3f9a-daemon", "onboarding"]})
        ),
        Ok(OrchCall::GetContext {
            scouts: Some(vec!["3f9a-daemon".into(), "onboarding".into()])
        })
    );
    assert_eq!(
        orch(
            "spawn_scout",
            json!({"id": "daemon-2", "question": "Where are hooks parsed?", "area": ["crates/daemon/**"], "web": true})
        ),
        Ok(OrchCall::SpawnScout {
            id: "daemon-2".into(),
            question: "Where are hooks parsed?".into(),
            area: vec!["crates/daemon/**".into()],
            web: true,
        })
    );
    assert_eq!(
        orch(
            "spawn_scout",
            json!({"id": "d", "question": "q", "area": ["a"]})
        ),
        Ok(OrchCall::SpawnScout {
            id: "d".into(),
            question: "q".into(),
            area: vec!["a".into()],
            web: false,
        })
    );
    assert_eq!(
        orch(
            "spawn_subplanner",
            json!({"epic": "daemon", "title": "The daemon", "area": ["crates/daemon/**"],
                   "brief": "Plan it.", "scout_refs": ["3f9a-daemon"]})
        ),
        Ok(OrchCall::SpawnSubplanner {
            epic: "daemon".into(),
            title: "The daemon".into(),
            area: vec!["crates/daemon/**".into()],
            brief: "Plan it.".into(),
            scout_refs: vec!["3f9a-daemon".into()],
            covers: Vec::new(),
        })
    );
    assert_eq!(
        orch(
            "edit_plan",
            json!({"edits": [{"op": "pause"}, {"op": "message", "to": "running", "text": "hi", "kind": "info"}],
                   "submit": true, "summary": "All done."})
        ),
        Ok(OrchCall::EditPlan {
            edits: vec![
                PlanEdit::Pause,
                PlanEdit::Message {
                    to: MessageTarget::Running,
                    text: "hi".into(),
                    kind: MessageKind::Info,
                }
            ],
            submit: true,
            summary: Some("All done.".into()),
            iterate: None,
            responses: Vec::new(),
        })
    );
    assert_eq!(
        orch("edit_plan", json!({"edits": []})),
        Ok(OrchCall::EditPlan {
            edits: Vec::new(),
            submit: false,
            summary: None,
            iterate: None,
            responses: Vec::new(),
        })
    );
    assert_eq!(
        orch("run_status", json!({"since": 42, "wait_secs": 50})),
        Ok(OrchCall::RunStatus {
            since: Some(42),
            wait_secs: 50,
        })
    );
    assert_eq!(
        orch("run_status", json!({})),
        Ok(OrchCall::RunStatus {
            since: None,
            wait_secs: 0,
        })
    );
    assert_eq!(
        orch("task_result", json!({"task_id": "t2"})),
        Ok(OrchCall::TaskResult {
            task_id: "t2".into()
        })
    );
    assert_eq!(
        parse_call(
            AgentRole::Planner,
            "submit_epic",
            &json!({"edits": [{"op": "cancel_task", "task_id": "a1"}], "note": "needs an interface task"})
        ),
        Ok(OrchCall::SubmitEpic {
            edits: vec![PlanEdit::CancelTask {
                task_id: "a1".into()
            }],
            note: Some("needs an interface task".into()),
        })
    );
    assert_eq!(
        parse_call(
            AgentRole::Worker,
            "task_note",
            &json!({"kind": "discovery", "text": "the hook fires twice"})
        ),
        Ok(OrchCall::TaskNote {
            kind: TaskNoteKind::Discovery,
            text: "the hook fires twice".into(),
        })
    );
}

#[test]
fn invalid_arguments_name_the_field_and_the_problem() {
    let cases: Vec<(&str, Value, &str)> = vec![
        ("get_context", json!([]), "arguments: must be an object"),
        ("get_context", json!({"extra": 1}), "extra: unknown field"),
        (
            "get_context",
            json!({"scouts": "a"}),
            "scouts: must be an array",
        ),
        (
            "get_context",
            json!({"scouts": [""]}),
            "scouts[0]: must be 1 to 48 characters",
        ),
        (
            "get_context",
            json!({"scouts": vec!["a"; 51]}),
            "scouts: at most 50 items",
        ),
        (
            "spawn_scout",
            json!({"question": "q", "area": ["a"]}),
            "id: required",
        ),
        (
            "spawn_scout",
            json!({"id": "Daemon", "question": "q", "area": ["a"]}),
            "id: must match ^[a-z0-9][a-z0-9-]{0,31}$",
        ),
        (
            "spawn_scout",
            json!({"id": "d", "question": "q", "area": []}),
            "area: at least 1 item",
        ),
        (
            "spawn_scout",
            json!({"id": "d", "question": "q".repeat(2001), "area": ["a"]}),
            "question: must be 1 to 2000 characters",
        ),
        (
            "spawn_scout",
            json!({"id": "d", "question": "q", "area": ["a"], "web": "yes"}),
            "web: must be a boolean",
        ),
        (
            "spawn_subplanner",
            json!({"epic": "a-very-long-e", "title": "t", "area": ["a"], "brief": "b"}),
            "epic: must match ^[a-z0-9][a-z0-9-]{0,10}$",
        ),
        (
            "spawn_subplanner",
            json!({"epic": "e", "title": "t".repeat(81), "area": ["a"], "brief": "b"}),
            "title: must be 1 to 80 characters",
        ),
        (
            "edit_plan",
            json!({"edits": [], "summary": ""}),
            "summary: must be 1 to 8000 characters",
        ),
        (
            "edit_plan",
            json!({"edits": vec![json!({"op": "pause"}); 61]}),
            "edits: at most 60 items",
        ),
        (
            "edit_plan",
            json!({"edits": [{"op": "message", "to": ["t1"], "text": "x".repeat(4001), "kind": "info"}]}),
            "edits[0]: message: text: at most 4000 characters",
        ),
        (
            "run_status",
            json!({"since": -1}),
            "since: must be a non-negative integer",
        ),
        (
            "task_result",
            json!({"task_id": "t".repeat(17)}),
            "task_id: must be 1 to 16 characters",
        ),
    ];
    for (tool, args, problem) in cases {
        assert_eq!(
            orch(tool, args.clone()),
            Err(format!("invalid arguments: {problem}")),
            "{tool} {args}"
        );
    }
    let planner = |args: Value| parse_call(AgentRole::Planner, "submit_epic", &args);
    assert_eq!(
        planner(json!({"edits": []})),
        Err("invalid arguments: edits: at least 1 item".into())
    );
    assert_eq!(
        planner(json!({"edits": [{"op": "pause"}], "note": "n".repeat(2001)})),
        Err("invalid arguments: note: must be 1 to 2000 characters".into())
    );
    let worker = |args: Value| parse_call(AgentRole::Worker, "task_note", &args);
    assert_eq!(
        worker(json!({"kind": "idea", "text": "x"})),
        Err("invalid arguments: kind: must be one of discovery, risk, progress".into())
    );
    assert_eq!(
        worker(json!({"kind": "risk", "text": "x".repeat(4001)})),
        Err("invalid arguments: text: must be 1 to 4000 characters".into())
    );
}

#[test]
fn wait_secs_over_50_is_refused() {
    assert_eq!(RUN_STATUS_MAX_WAIT, 50);
    assert_eq!(
        orch("run_status", json!({"since": 1, "wait_secs": 51})),
        Err("invalid arguments: wait_secs: must be 0 to 50".into())
    );
    assert_eq!(
        orch("run_status", json!({"wait_secs": 1.5})),
        Err("invalid arguments: wait_secs: must be 0 to 50".into())
    );
}

#[test]
fn tool_outside_the_role_is_refused() {
    let refused = |role: AgentRole, tool: &str| {
        assert_eq!(
            parse_call(role, tool, &json!({})),
            Err(format!(
                "tool {tool} is not available to the {} role",
                crate::run::orch::json::label(&role)
            )),
            "{role:?} {tool}"
        );
    };
    for tool in [
        "get_context",
        "spawn_scout",
        "spawn_subplanner",
        "edit_plan",
        "run_status",
        "task_result",
        "submit_epic",
    ] {
        refused(AgentRole::Worker, tool);
        refused(AgentRole::Reviewer, tool);
        refused(AgentRole::Scout, tool);
        refused(AgentRole::Decider, tool);
    }
    refused(AgentRole::Orchestrator, "task_note");
    refused(AgentRole::Orchestrator, "submit_epic");
    refused(AgentRole::Orchestrator, "task_done");
    refused(AgentRole::Planner, "task_note");
    refused(AgentRole::Orchestrator, "frobnicate");
}

#[test]
fn planner_cannot_call_edit_plan() {
    for tool in [
        "edit_plan",
        "spawn_scout",
        "spawn_subplanner",
        "run_status",
        "task_result",
    ] {
        assert_eq!(
            parse_call(AgentRole::Planner, tool, &json!({"edits": []})),
            Err(format!("tool {tool} is not available to the planner role"))
        );
    }
}

/// Milestone 9.9 decision 11: `override` and `approve_hold` are the orchestrator's own
/// ops now and parse into `PlanEdit`s (each needs its reason); the plan's `approve` and
/// `accept` stay unknown ops, so a model still cannot approve a plan.
#[test]
fn override_and_approve_hold_parse_but_approve_and_accept_do_not() {
    let call = orch(
        "edit_plan",
        json!({"edits": [{"op": "override", "task_id": "t1", "reason": "fine"}]}),
    )
    .unwrap();
    let OrchCall::EditPlan { edits, .. } = call else {
        panic!("{call:?}");
    };
    assert_eq!(
        edits,
        vec![proto::PlanEdit::Override {
            task_id: "t1".into(),
            reason: "fine".into()
        }]
    );
    let call = orch(
        "edit_plan",
        json!({"edits": [{"op": "approve_hold", "hold": "promotion", "reason": "ok"}]}),
    )
    .unwrap();
    let OrchCall::EditPlan { edits, .. } = call else {
        panic!("{call:?}");
    };
    assert_eq!(
        edits,
        vec![proto::PlanEdit::ApproveHold {
            hold: "promotion".into(),
            reason: "ok".into()
        }]
    );
    // The reason is not optional.
    let error = orch(
        "edit_plan",
        json!({"edits": [{"op": "override", "task_id": "t1"}]}),
    )
    .unwrap_err();
    assert!(error.starts_with("invalid arguments: edits[0]:"), "{error}");
    assert!(error.contains("reason"), "{error}");
    for op in ["approve", "accept"] {
        let error = orch("edit_plan", json!({"edits": [{"op": op}]})).unwrap_err();
        assert!(
            error.starts_with(&format!(
                "invalid arguments: edits[0]: unknown variant `{op}`"
            )),
            "{error}"
        );
    }
}

/// Milestone 9.3 decision 29: `start_goal { goal }` (1 to 16,384 characters) is the
/// orchestrator's alone; every other role is refused with the role's text.
#[test]
fn start_goal_is_the_orchestrators_alone() {
    assert_eq!(
        orch("start_goal", json!({"goal": "add a login page"})),
        Ok(OrchCall::StartGoal {
            goal: "add a login page".into()
        })
    );
    let long = "x".repeat(proto::GOAL_MAX_CHARS + 1);
    for (args, problem) in [
        (json!({}), "goal: required"),
        (json!({"goal": ""}), "goal: must be 1 to 16384 characters"),
        (json!({"goal": long}), "goal: must be 1 to 16384 characters"),
        (json!({"goal": 3}), "goal: must be a string"),
        (json!({"goal": "x", "yes": true}), "yes: unknown field"),
    ] {
        assert_eq!(
            orch("start_goal", args),
            Err(format!("invalid arguments: {problem}"))
        );
    }
    for role in [
        AgentRole::Planner,
        AgentRole::Worker,
        AgentRole::Reviewer,
        AgentRole::Scout,
        AgentRole::Decider,
    ] {
        assert_eq!(
            parse_call(role, "start_goal", &json!({"goal": "x"})),
            Err(format!(
                "tool start_goal is not available to the {} role",
                crate::run::orch::json::label(&role)
            )),
            "{role:?}"
        );
    }
}

/// Milestone 9.9 decision 15: `ask_user` takes a question, up to nine options and a
/// context; the last two default to empty.
#[test]
fn ask_user_parses() {
    assert_eq!(
        orch(
            "ask_user",
            json!({"question": "tabs or spaces?", "options": ["tabs", "spaces"], "context": "no guide"})
        ),
        Ok(OrchCall::AskUser {
            question: "tabs or spaces?".into(),
            options: vec!["tabs".into(), "spaces".into()],
            context: "no guide".into(),
        })
    );
    assert_eq!(
        orch("ask_user", json!({"question": "go?"})),
        Ok(OrchCall::AskUser {
            question: "go?".into(),
            options: Vec::new(),
            context: String::new(),
        })
    );
    let bad = |args: Value| orch("ask_user", args).unwrap_err();
    assert_eq!(bad(json!({})), "invalid arguments: question: required");
    assert_eq!(
        bad(json!({"question": "q", "extra": 1})),
        "invalid arguments: extra: unknown field"
    );
    assert_eq!(
        bad(json!({"question": "q".repeat(501)})),
        "invalid arguments: question: at most 500 characters"
    );
    assert_eq!(
        bad(json!({"question": "q", "options": [""]})),
        "invalid arguments: options[0]: empty"
    );
    assert_eq!(
        bad(json!({"question": "q", "options": ["x".repeat(201)]})),
        "invalid arguments: options[0]: at most 200 characters"
    );
    assert_eq!(
        bad(json!({"question": "q", "context": "c".repeat(4001)})),
        "invalid arguments: context: at most 4000 characters"
    );
    // Only the orchestrator has it.
    for role in [
        AgentRole::Planner,
        AgentRole::Worker,
        AgentRole::Brainstormer,
    ] {
        let refused = parse_call(role, "ask_user", &json!({"question": "q"})).unwrap_err();
        assert!(
            refused.starts_with("tool ask_user is not available"),
            "{refused}"
        );
    }
}
