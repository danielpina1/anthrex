//! Task M9.11, review finding 3: the daemon enforces the Interfaces table's bounds
//! inside `plan_edit`, `plan_task` and `route`, and refuses their unknown fields, on a
//! tool call's untrusted arguments (the MCP schema is only what the model sees). Pure.

use proto::{AgentRole, PlanEdit};
use serde_json::{Value, json};

use super::*;

/// A valid `plan_task`, each string at its table maximum when `full`.
fn task(full: bool) -> Value {
    let s = |n: usize, c: char| {
        if full {
            c.to_string().repeat(n)
        } else {
            c.to_string()
        }
    };
    let list = |n: usize, each: usize| -> Vec<String> {
        (0..if full { n } else { 1 })
            .map(|i| format!("{i:0>width$}", width = if full { each } else { 1 }))
            .collect()
    };
    json!({
        "id": "m1", "title": s(120, 't'), "epic": s(11, 'e'), "kind": "code", "size": "S",
        "interface_change": false, "test_mode": "check", "test_mode_reason": s(300, 'r'),
        "owns": list(20, 300), "deps": list(20, 16), "priority": 3, "brief": s(8000, 'b'),
        "acceptance": list(20, 500), "test_to_write": s(300, 'w'),
        "scout_refs": list(20, 48), "review_target": s(200, 'v'),
        "stage": 2, "atomic": true, "atomic_reason": s(300, 'a'),
        "route": {"runtime": "claude", "model": s(100, 'm'), "strength": "standard",
                  "effort": "high"},
    })
}

fn edit_plan(edit: Value) -> Result<OrchCall, String> {
    parse_call(
        AgentRole::Orchestrator,
        "edit_plan",
        &json!({ "edits": [edit] }),
    )
}

fn refusal(edit: Value) -> String {
    edit_plan(edit).expect_err("refused")
}

#[test]
fn a_task_at_every_bound_is_accepted() {
    for full in [false, true] {
        let edit = json!({"op": "add_task", "task": task(full)});
        assert!(edit_plan(edit).is_ok(), "full: {full}");
    }
    let into: Vec<Value> = (0..12).map(|_| task(true)).collect();
    assert!(edit_plan(json!({"op": "split_task", "task_id": "t1", "into": into})).is_ok());
}

/// The reviewer's case: a forged `add_task` whose brief is 200 000 characters.
#[test]
fn nested_strings_past_their_bound_are_refused() {
    let with = |key: &str, value: Value| {
        let mut t = task(false);
        t[key] = value;
        json!({"op": "add_task", "task": t})
    };
    let long = |n: usize| Value::String("x".repeat(n));
    let cases = [
        (
            with("brief", long(200_000)),
            "task: brief: must be 1 to 8000 characters",
        ),
        (
            with("title", long(121)),
            "task: title: must be 1 to 120 characters",
        ),
        (
            with("title", long(0)),
            "task: title: must be 1 to 120 characters",
        ),
        (
            with("epic", long(12)),
            "task: epic: must be 1 to 11 characters",
        ),
        (
            with("test_to_write", long(301)),
            "task: test_to_write: must be 1 to 300 characters",
        ),
        (
            with("review_target", long(201)),
            "task: review_target: must be 1 to 200 characters",
        ),
        (
            with("owns", json!([long(301)])),
            "task: owns[0]: must be 1 to 300 characters",
        ),
        (
            with("deps", json!([long(17)])),
            "task: deps[0]: must be 1 to 16 characters",
        ),
        (
            with("route", json!({"model": long(101)})),
            "task: route: model: must be 0 to 100 characters",
        ),
        (with("title", json!(7)), "task: title: must be a string"),
        (
            with("atomic_reason", long(301)),
            "task: atomic_reason: must be 1 to 300 characters",
        ),
    ];
    for (edit, want) in cases {
        assert_eq!(
            refusal(edit),
            format!("invalid arguments: edits[0]: {want}")
        );
    }
    let edit_cases = [
        (
            json!({"op": "amend_task", "task_id": "t1", "brief": "x".repeat(8001)}),
            "brief: must be 1 to 8000 characters",
        ),
        (
            json!({"op": "answer", "task_id": "t1", "text": "x".repeat(8001)}),
            "text: must be 1 to 8000 characters",
        ),
        (
            json!({"op": "add_dep", "task_id": "t1", "dep": "x".repeat(17)}),
            "dep: must be 1 to 16 characters",
        ),
        (
            json!({"op": "amend_task", "task_id": "x".repeat(17)}),
            "task_id: must be 1 to 16 characters",
        ),
        (
            json!({"op": "amend_task", "task_id": "t1", "test_mode_reason": "x".repeat(301)}),
            "test_mode_reason: must be 1 to 300 characters",
        ),
    ];
    for (edit, want) in edit_cases {
        assert_eq!(
            refusal(edit),
            format!("invalid arguments: edits[0]: {want}")
        );
    }
}

#[test]
fn nested_lists_past_their_bound_are_refused() {
    let with = |key: &str, n: usize| {
        let mut t = task(false);
        t[key] = json!((0..n).map(|i| format!("a{i}")).collect::<Vec<_>>());
        json!({"op": "add_task", "task": t})
    };
    let cases = [
        (with("owns", 21), "task: owns: at most 20 items"),
        (with("deps", 21), "task: deps: at most 20 items"),
        (with("scout_refs", 21), "task: scout_refs: at most 20 items"),
        (with("acceptance", 21), "task: acceptance: at most 20 items"),
        (with("acceptance", 0), "task: acceptance: at least 1 item"),
    ];
    for (edit, want) in cases {
        assert_eq!(
            refusal(edit),
            format!("invalid arguments: edits[0]: {want}")
        );
    }
    let into: Vec<Value> = (0..13).map(|_| task(false)).collect();
    let split = json!({"op": "split_task", "task_id": "t1", "into": into});
    assert_eq!(
        refusal(split),
        "invalid arguments: edits[0]: into: at most 12 items"
    );
    let mut child = task(false);
    child["brief"] = json!("x".repeat(8001));
    let split = json!({"op": "split_task", "task_id": "t1", "into": [task(false), child]});
    assert_eq!(
        refusal(split),
        "invalid arguments: edits[0]: into[1]: brief: must be 1 to 8000 characters"
    );
    let ids: Vec<String> = (0..21).map(|i| format!("t{i}")).collect();
    let message = json!({"op": "message", "to": ids, "kind": "info", "text": "hi"});
    assert_eq!(
        refusal(message),
        "invalid arguments: edits[0]: to: at most 20 items"
    );
    let message = json!({"op": "message", "to": ["x".repeat(17)], "kind": "info", "text": "hi"});
    assert_eq!(
        refusal(message),
        "invalid arguments: edits[0]: to[0]: must be 1 to 16 characters"
    );
    let amend = json!({"op": "amend_task", "task_id": "t1", "acceptance": vec!["a"; 21]});
    assert_eq!(
        refusal(amend),
        "invalid arguments: edits[0]: acceptance: at most 20 items"
    );
}

/// The schema's objects are closed: a field it does not have is refused, at every
/// level. `budget` is left to rule 7.1's own refusal (`edit_plan_reply_shapes`), and an
/// unknown `op` to serde's (`orchestrator_tools_cannot_approve`).
#[test]
fn unknown_nested_fields_are_refused() {
    let mut budgeted = task(false);
    budgeted["hub"] = json!(true);
    let mut routed = task(false);
    routed["route"]["provider"] = json!("x");
    let cases = [
        (
            json!({"op": "add_task", "task": budgeted}),
            "task: hub: unknown field",
        ),
        (
            json!({"op": "add_task", "task": routed}),
            "task: route: provider: unknown field",
        ),
        (
            json!({"op": "pause", "reason": "x"}),
            "reason: unknown field",
        ),
    ];
    for (edit, want) in cases {
        assert_eq!(
            refusal(edit),
            format!("invalid arguments: edits[0]: {want}")
        );
    }
    // `submit_epic` goes through the same check.
    let mut t = task(false);
    t["brief"] = json!("x".repeat(8001));
    let args = json!({"edits": [{"op": "add_task", "task": t}]});
    assert_eq!(
        parse_call(AgentRole::Planner, "submit_epic", &args),
        Err("invalid arguments: edits[0]: task: brief: must be 1 to 8000 characters".into())
    );
}

/// Re-review finding 1: `null` in an optional field is absent, as serde's `Option`
/// reads it; the bounds pass it on to serde at every level.
#[test]
fn null_in_an_optional_field_is_absent() {
    let mut t = task(false);
    t["epic"] = Value::Null;
    t["test_to_write"] = Value::Null;
    t["route"] = json!({"runtime": "claude", "model": null});
    assert!(edit_plan(json!({"op": "add_task", "task": t})).is_ok());
    // A field serde does not read as an `Option` refuses `null` with serde's text.
    let mut t = task(false);
    t["route"] = Value::Null;
    let error = refusal(json!({"op": "add_task", "task": t}));
    assert!(error.contains("invalid type: null"), "{error}");
    let amend = json!({"op": "amend_task", "task_id": "t1", "brief": null, "acceptance": null});
    assert!(edit_plan(amend.clone()).is_ok(), "{:?}", edit_plan(amend));
    // Present and not null, a field is still bounded.
    let amend = json!({"op": "amend_task", "task_id": "t1", "brief": 7});
    assert_eq!(
        refusal(amend),
        "invalid arguments: edits[0]: brief: must be a string"
    );
}

/// Re-review finding 3: a task's id is the plan rules' (decision 19's structured
/// `id` error, `engine/tests/orch_edit.rs`), not a length bound here.
#[test]
fn a_task_id_is_left_to_the_plan_rules() {
    let mut t = task(false);
    t["id"] = json!("x".repeat(17));
    assert!(edit_plan(json!({"op": "add_task", "task": t})).is_ok());
}

/// The M9.1.3 review's finding on decision 43: `stage`, `atomic` and `atomic_reason`
/// reach `add_task`, `split_task`, `amend_task` and `submit_epic`. Their range is the
/// plan rules' (`validate_tests_stages.rs`), like every other integer here.
#[test]
fn stage_and_atomic_fields_are_accepted() {
    let edits = [
        json!({"op": "add_task", "task": task(false)}),
        json!({"op": "split_task", "task_id": "t1", "into": [task(false)]}),
        json!({"op": "amend_task", "task_id": "t1", "stage": 3}),
    ];
    let Ok(OrchCall::EditPlan { edits: parsed, .. }) = parse_call(
        AgentRole::Orchestrator,
        "edit_plan",
        &json!({ "edits": edits }),
    ) else {
        panic!("edit_plan must parse");
    };
    match &parsed[..] {
        [
            PlanEdit::AddTask { task },
            PlanEdit::SplitTask { into, .. },
            PlanEdit::AmendTask { stage, .. },
        ] => {
            assert_eq!(
                (task.stage, task.atomic, task.atomic_reason.as_deref()),
                (2, true, Some("a"))
            );
            assert_eq!(into[0].stage, 2);
            assert_eq!(*stage, Some(3));
        }
        other => panic!("{other:?}"),
    }
    let args = json!({"edits": [{"op": "add_task", "task": task(true)}]});
    let Ok(OrchCall::SubmitEpic { edits, .. }) =
        parse_call(AgentRole::Planner, "submit_epic", &args)
    else {
        panic!("submit_epic must parse");
    };
    let [PlanEdit::AddTask { task }] = &edits[..] else {
        panic!("{edits:?}");
    };
    assert_eq!((task.stage, task.atomic), (2, true));
    // An amend's `atomic` is not a field: atomic is set when a task is added.
    assert_eq!(
        refusal(json!({"op": "amend_task", "task_id": "t1", "atomic": true})),
        "invalid arguments: edits[0]: atomic: unknown field"
    );
}

/// Milestone 9.3 decision 30: `edit_plan`'s `iterate` is 1 to 16,384 characters, with
/// or without an `edits` array; an `iterate` edit inside `edits` is bounded the same
/// way and reaches the engine, which refuses it in the orchestrator's words.
#[test]
fn parse_call_bounds_iterate() {
    let parse = |args: Value| parse_call(AgentRole::Orchestrator, "edit_plan", &args);
    let at = "é".repeat(proto::GOAL_MAX_CHARS);
    let over = "x".repeat(proto::GOAL_MAX_CHARS + 1);
    assert_eq!(
        parse(json!({"iterate": at})),
        Ok(OrchCall::EditPlan {
            edits: Vec::new(),
            submit: false,
            summary: None,
            iterate: Some(at.clone()),
        })
    );
    let Ok(OrchCall::EditPlan { edits, iterate, .. }) =
        parse(json!({"iterate": "x", "edits": [], "submit": true}))
    else {
        panic!("edit_plan must parse");
    };
    assert_eq!((edits.len(), iterate.as_deref()), (0, Some("x")));
    assert_eq!(
        parse(json!({"iterate": over})),
        Err("invalid arguments: iterate: must be 1 to 16384 characters".into())
    );
    assert_eq!(
        parse(json!({"iterate": ""})),
        Err("invalid arguments: iterate: must be 1 to 16384 characters".into())
    );
    assert_eq!(
        parse(json!({"submit": true})),
        Err("invalid arguments: edits: required".into()),
        "edits stay required without iterate"
    );
    let Ok(OrchCall::EditPlan { edits, .. }) =
        parse(json!({"edits": [{"op": "iterate", "goal": at}]}))
    else {
        panic!("an iterate edit parses");
    };
    assert_eq!(edits, vec![PlanEdit::Iterate { goal: at }]);
    assert_eq!(
        refusal(json!({"op": "iterate", "goal": over})),
        "invalid arguments: edits[0]: goal: must be 1 to 16384 characters"
    );
}
