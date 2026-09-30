//! Milestone 9.1 task M9.1.11 through the orchestrator's and a sub-planner's tools:
//! the reserved `stage-<n>` and `fix<n>` ids in every plan source (decisions 39, 45),
//! and `stage`, `atomic` and `atomic_reason` reaching `edit_plan` and `submit_epic`
//! (the M9.1.3 review's finding on decision 43). Each call goes through the tool
//! parser, its raw-argument bounds and the plan rules, as a real call does.

use proto::{RunState, TaskState};
use serde_json::{Value, json};

use super::dispatch::replies;
use super::orch::{add, answer, edit_plan, launched};
use super::planners::{PLANNER, planner_started, spawn, submit_epic};
use crate::run::engine::Effect;
use crate::run::plan::PlanError;

const STAGE: &str = "stage-<n> is reserved for stage branches";
const FIX: &str = "fix<n> ids are reserved for fix tasks the engine adds";

/// `add` with `fields` merged into its task.
fn add_with(id: &str, module: &str, fields: Value) -> Value {
    let mut edit = add(id, module);
    for (k, v) in fields.as_object().unwrap() {
        edit["task"][k] = v.clone();
    }
    edit
}

/// The `(task, field, rule, message)` of each error of a rejected reply.
fn errors(effects: &[Effect]) -> Vec<(String, String, String, String)> {
    let (ok, value) = answer(effects);
    assert!(!ok, "{value}");
    value["errors"]
        .as_array()
        .unwrap_or_else(|| panic!("{value}"))
        .iter()
        .map(|e| {
            let s = |k: &str| e[k].as_str().unwrap_or_default().to_string();
            (s("task"), s("field"), s("rule"), s("message"))
        })
        .collect()
}

fn reserved(id: &str) -> (String, String, String, String) {
    let message = if id.starts_with("stage-") {
        STAGE.replace("stage-<n>", id)
    } else {
        FIX.to_string()
    };
    (id.into(), "id".into(), "id".into(), message)
}

const IDS: [&str; 3] = ["stage-1", "fix1", "fix12"];

fn expected() -> Vec<(String, String, String, String)> {
    IDS.iter().map(|id| reserved(id)).collect()
}

/// The one reply is a success (`submit_epic`'s is plain text, `edit_plan`'s JSON).
fn accepted(effects: &[Effect]) {
    let replies = replies(effects);
    assert!(matches!(replies[..], [Ok(_)]), "{replies:?}");
}

#[test]
fn stage_and_fix_ids_are_reserved_for_every_source() {
    // A plan file.
    let pe = |e: &PlanError| {
        (
            e.task.clone().unwrap_or_default(),
            e.field.clone(),
            e.rule.clone(),
            e.message.clone(),
        )
    };
    let tasks: Vec<String> = IDS
        .iter()
        .map(|id| {
            crate::run::test_support::task_toml(id, "S", &format!("[\"crates/{id}/**\"]"), "")
        })
        .collect();
    let text = crate::run::test_support::plan_with(crate::run::test_support::PROFILE, &tasks);
    let got: Vec<_> = crate::run::test_support::errors_of(&text)
        .iter()
        .map(pe)
        .collect();
    assert_eq!(got, expected(), "plan file");

    let edits = || -> Vec<Value> { IDS.iter().map(|id| add(id, id)).collect() };

    // The orchestrator's plan, submitted at the gate (its `submit_plan`).
    let mut fx = launched(false);
    let effects = edit_plan(&mut fx, json!({"edits": edits(), "submit": true}));
    assert_eq!(errors(&effects), expected(), "submit");
    assert_eq!(fx.run().state, RunState::Planning);

    // An orchestrator `add_task` on a running run.
    let mut fx = launched(false);
    accepted(&edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    ));
    fx.approve();
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(
        errors(&edit_plan(&mut fx, json!({"edits": edits()}))),
        expected(),
        "add_task"
    );

    // A sub-planner's plan.
    let mut fx = launched(false);
    accepted(&edit_plan(&mut fx, json!({"edits": [add("t1", "auth")]})));
    spawn(&mut fx, "mail");
    planner_started(&mut fx, PLANNER);
    let mail: Vec<Value> = IDS
        .iter()
        .map(|id| add(id, &format!("mail/{id}")))
        .collect();
    assert_eq!(
        errors(&submit_epic(&mut fx, json!(mail))),
        expected(),
        "epic"
    );

    // Ids that only look like them are accepted, in every source.
    let text = crate::run::test_support::plan_with(
        crate::run::test_support::PROFILE,
        &["fixture", "stage", "stage-a", "fix"]
            .iter()
            .map(|id| {
                crate::run::test_support::task_toml(id, "S", &format!("[\"crates/{id}/**\"]"), "")
            })
            .collect::<Vec<_>>(),
    );
    crate::run::test_support::run_ok(&text);
    let mut fx = launched(false);
    accepted(&edit_plan(
        &mut fx,
        json!({"edits": [add("fixture", "a"), add("stage", "b")], "submit": true}),
    ));
    let mut fx = launched(false);
    accepted(&edit_plan(&mut fx, json!({"edits": [add("t1", "auth")]})));
    spawn(&mut fx, "mail");
    planner_started(&mut fx, PLANNER);
    accepted(&submit_epic(
        &mut fx,
        json!([add("fixture", "mail/a"), add("stage", "mail/b")]),
    ));
}

#[test]
fn stage_and_atomic_reach_edit_plan_and_submit_epic() {
    let mut fx = launched(false);
    let atomic = json!({"stage": 2, "atomic": true, "atomic_reason": "protocol bump"});
    accepted(&edit_plan(
        &mut fx,
        json!({"edits": [
            add("t1", "auth"),
            add_with("t2", "proto", atomic),
            add_with("t3", "mail", json!({"stage": 2})),
        ], "submit": true}),
    ));
    let t2 = fx.task("t2");
    assert_eq!(
        (
            t2.spec.stage,
            t2.spec.atomic,
            t2.spec.atomic_reason.as_deref()
        ),
        (2, true, Some("protocol bump"))
    );
    assert!(t2.hub, "decision 54: an atomic task is a hub task");
    assert_eq!(fx.task("t3").spec.stage, 2);

    // `amend_task`'s `stage` on a task that has not started.
    accepted(&edit_plan(
        &mut fx,
        json!({"edits": [{"op": "amend_task", "task_id": "t3", "stage": 1}]}),
    ));
    assert_eq!(fx.task("t3").spec.stage, 1);

    // A stage with another field on a started task is refused as a whole, and the
    // edit log records the batch as rejected, not as a move.
    fx.approve();
    fx.run_mut().tasks[0].state = TaskState::Working;
    let before = fx.task("t1").spec.clone();
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [{"op": "amend_task", "task_id": "t1", "stage": 2, "priority": 5}]}),
    );
    assert_eq!(
        errors(&effects),
        [(
            "t1".to_string(),
            String::new(),
            "13".to_string(),
            "task t1 is working; stage can be amended only on pending, queued or blocked tasks"
                .to_string()
        )]
    );
    assert_eq!(fx.task("t1").spec, before);
    let record = fx.run().plan_edits.last().unwrap();
    assert!(!record.accepted, "{record:?}");

    // A sub-planner's `submit_epic` carries them too.
    let mut fx = launched(false);
    accepted(&edit_plan(&mut fx, json!({"edits": [add("t1", "auth")]})));
    spawn(&mut fx, "mail");
    planner_started(&mut fx, PLANNER);
    accepted(&submit_epic(
        &mut fx,
        json!([add_with(
            "m1",
            "mail",
            json!({"stage": 2, "atomic": true, "atomic_reason": "one step"})
        )]),
    ));
    let m1 = fx.task("m1");
    assert_eq!((m1.spec.stage, m1.spec.atomic), (2, true));
}
