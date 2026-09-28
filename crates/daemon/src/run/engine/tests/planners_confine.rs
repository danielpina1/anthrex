//! Milestone 9 task M9.8: a sub-planner is confined to its epic (decision 22; M9.4
//! review fixes, ruling 4, which left these to M9.8). It changes only its own epic's
//! tasks, whatever the op, and uses none of the ops that steer a run.

use serde_json::{Value, json};

use super::fixture::*;
use super::orch::{answer, edit_plan, error};
use super::planners::{epic, planning_mail, spawn, submit_epic, task_in};
use crate::run::engine::Effect;
use crate::run::orch::PlannerPhase;

/// [`planning_mail`] with `w1`, a task of epic `web` (planned and finished), beside the
/// orchestrator's `t1`, which names no epic.
fn with_neighbours() -> Fixture {
    let mut fx = planning_mail(false);
    spawn(&mut fx, "web");
    let web = fx
        .run_mut()
        .orch
        .epics
        .iter_mut()
        .find(|e| e.epic == "web")
        .unwrap();
    web.phase = PlannerPhase::Finished;
    let mut w1 = task_in("w1", "web");
    w1["task"]["epic"] = json!("web");
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [w1]})));
    assert!(ok, "{value}");
    fx
}

/// The rejection's messages, and the run unchanged but for the planner's counts.
fn refused(fx: &mut Fixture, edits: Value) -> Vec<String> {
    let tasks = fx.run().tasks.clone();
    let effects = submit_epic(fx, edits);
    let (ok, value) = answer(&effects);
    assert!(!ok, "{value}");
    assert_eq!(value["accepted"], false, "{value}");
    assert_eq!(fx.run().tasks, tasks, "the batch changed nothing");
    assert_eq!(epic(fx, "mail").phase, PlannerPhase::Planning);
    value["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["message"].as_str().unwrap().to_string())
        .collect()
}

fn not_own(id: &str) -> Vec<String> {
    vec![format!(
        "task {id}: a sub-planner changes only its own epic's tasks"
    )]
}

#[test]
fn submit_epic_cannot_amend_another_epics_or_the_orchestrators_task() {
    let mut fx = with_neighbours();
    for id in ["w1", "t1"] {
        let amend = json!({"op": "amend_task", "task_id": id, "brief": "Rewritten"});
        assert_eq!(refused(&mut fx, json!([amend])), not_own(id), "{id}");
        assert_ne!(fx.task(id).spec.brief, "Rewritten");
    }
}

#[test]
fn submit_epic_cannot_cancel_another_epics_or_the_orchestrators_task() {
    let mut fx = with_neighbours();
    for id in ["w1", "t1"] {
        let cancel = json!({"op": "cancel_task", "task_id": id});
        assert_eq!(refused(&mut fx, json!([cancel])), not_own(id), "{id}");
        assert_ne!(fx.task(id).state, proto::TaskState::Cancelled);
    }
}

#[test]
fn submit_epic_cannot_add_dep_on_another_epics_or_the_orchestrators_task() {
    let mut fx = with_neighbours();
    for id in ["w1", "t1"] {
        let dep = json!({"op": "add_dep", "task_id": id, "dep": "w1"});
        let edits = json!([task_in("m1", "mail"), dep]);
        assert_eq!(refused(&mut fx, edits), not_own(id), "{id}");
    }
    // The control: its own task may depend on any task.
    let dep = json!({"op": "add_dep", "task_id": "m1", "dep": "t1"});
    let effects = submit_epic(&mut fx, json!([task_in("m1", "mail"), dep]));
    let (ok, _) = super::planners_holds::reply(&effects);
    assert!(ok, "{effects:#?}");
    assert_eq!(fx.task("m1").spec.deps, vec!["t1".to_string()]);
}

/// The steering ops are refused whole, before any effect.
#[test]
fn submit_epic_refuses_answer_pause_resume_finish_message_refresh() {
    let mut fx = with_neighbours();
    let ops = [
        (
            "answer",
            json!({"op": "answer", "task_id": "t1", "text": "yes"}),
        ),
        ("pause", json!({"op": "pause"})),
        ("resume", json!({"op": "resume"})),
        ("finish", json!({"op": "finish"})),
        (
            "message",
            json!({"op": "message", "to": ["t1"], "text": "hi", "kind": "info"}),
        ),
        ("refresh", json!({"op": "refresh", "task_id": "t1"})),
    ];
    for (name, op) in ops {
        let before = fx.run().clone();
        let effects = submit_epic(&mut fx, json!([task_in("m1", "mail"), op]));
        assert_eq!(
            error(&effects),
            format!("op {name} is not available to a sub-planner")
        );
        assert!(
            !effects
                .iter()
                .any(|e| matches!(e, Effect::StopPlanner { .. })),
            "{effects:#?}"
        );
        assert_eq!(fx.run().tasks, before.tasks, "{name}");
        assert_eq!(fx.run().state, before.state, "{name}");
    }
}

#[test]
fn submit_epic_refuses_pause() {
    let mut fx = planning_mail(false);
    let effects = submit_epic(&mut fx, json!([{"op": "pause"}]));
    assert_eq!(
        error(&effects),
        "op pause is not available to a sub-planner"
    );
    assert_eq!(fx.run().state, proto::RunState::Running);
    assert_eq!(epic(&fx, "mail").phase, PlannerPhase::Planning);
}

/// M9.4's confinement of added and split tasks still holds through `submit_epic`.
#[test]
fn submit_epic_cannot_add_to_or_split_another_epic() {
    let mut fx = with_neighbours();
    let mut other = task_in("m1", "mail");
    other["task"]["epic"] = json!("web");
    let messages = refused(&mut fx, json!([other]));
    assert_eq!(
        messages,
        vec!["a sub-planner adds tasks only to its own epic mail".to_string()]
    );
    let split = json!({"op": "split_task", "task_id": "t1",
        "into": [task_in("m1", "mail")["task"].clone()]});
    let messages = refused(&mut fx, json!([split]));
    assert_eq!(
        messages,
        vec!["a sub-planner splits only tasks of its own epic mail".to_string()]
    );
}
