//! Milestone 9 task M9.8's second review: a sub-planner changes only its own session
//! round's work (ruling 1), the orchestrator does not split or add a dependency onto a
//! live epic's task (ruling 2), and before the gate nothing is approved yet (ruling 3).

use proto::{HoldState, TaskState};
use serde_json::{Value, json};

use super::dispatch::replies;
use super::fixture::*;
use super::gate_holds::held;
use super::orch::{answer, edit_plan, launched};
use super::planners::{
    PLANNER, hold_state, planner_ended, planner_started, spawn, submit_epic, task_in,
};
use super::planners_holds::reply;
use super::planners_review::{SECOND, replanned_with_t2_awaiting, second_submits};
use super::promote::hold_verdict;
use crate::run::engine::ScoutEnd;
use crate::run::engine::planners::EPIC_RECORDED;

const OWN_ROUND: &str = "a sub-planner changes only its own round's tasks";

fn cancel(id: &str) -> Value {
    json!({"op": "cancel_task", "task_id": id})
}

fn split(id: &str) -> Value {
    json!({"op": "split_task", "task_id": id,
        "into": [task_in(&format!("{id}x"), "mail/x")["task"].clone()]})
}

fn add_dep(id: &str) -> Value {
    json!({"op": "add_dep", "task_id": id, "dep": "t1"})
}

fn amend(id: &str) -> Value {
    json!({"op": "amend_task", "task_id": id, "brief": "A new brief"})
}

/// The plan: each task's spec and whether it is cancelled (scheduling may move a task
/// on in the same step; that is not a change of the plan).
fn tasks(fx: &Fixture) -> String {
    let plan: Vec<_> = fx
        .run()
        .tasks
        .iter()
        .map(|t| (&t.spec, t.state == TaskState::Cancelled))
        .collect();
    serde_json::to_string(&plan).unwrap()
}

/// Session 2's batch `edits` is refused, one error per edit naming its own round, and
/// changes nothing.
fn refused_for_its_round(fx: &mut Fixture, id: &str, edits: Value) {
    let before = tasks(fx);
    let n = edits.as_array().unwrap().len();
    let (ok, text) = reply(&second_submits(fx, edits));
    assert!(!ok, "{text}");
    let expected = format!("task {id}: {OWN_ROUND}");
    assert_eq!(text.matches(&expected).count(), n, "{text}");
    assert_eq!(tasks(fx), before, "the run is unchanged");
}

fn accepted(effects: &[crate::run::engine::Effect]) {
    assert_eq!(replies(effects), vec![Ok(EPIC_RECORDED.to_string())]);
}

/// Ruling 1: `cancel_task`, `split_task` and `add_dep` of an approved `t2`, not yet
/// started and then `Working`, are refused, and nothing changes.
#[test]
fn a_planner_cannot_cancel_split_or_add_a_dep_to_released_or_started_work() {
    let mut fx = replanned_with_t2_awaiting("mail/a");
    hold_verdict(&mut fx, "epic:mail", true);
    fx.task_mut("t2").state = TaskState::Queued;
    refused_for_its_round(
        &mut fx,
        "t2",
        json!([cancel("t2"), split("t2"), add_dep("t2")]),
    );
    fx.task_mut("t2").state = TaskState::Working;
    refused_for_its_round(
        &mut fx,
        "t2",
        json!([cancel("t2"), split("t2"), add_dep("t2")]),
    );
    assert_eq!(fx.task("t2").state, TaskState::Working);
}

/// Ruling 1: a later session may not amend, split or add a dependency to a task in an
/// earlier round awaiting the user, where the change would ride on that round's
/// approval; it may cancel it, which only reduces the unapproved work.
#[test]
fn a_later_session_only_cancels_an_earlier_awaiting_rounds_task() {
    let mut fx = replanned_with_t2_awaiting("mail/a");
    refused_for_its_round(
        &mut fx,
        "t2",
        json!([amend("t2"), split("t2"), add_dep("t2")]),
    );
    assert_eq!(fx.task("t2").spec.brief, "Brief t2");
    accepted(&second_submits(&mut fx, json!([cancel("t2")])));
    assert_eq!(fx.task("t2").state, TaskState::Cancelled);
}

/// Ruling 1: a session changes the tasks of its own round. Here the orchestrator's
/// `t2` sits in the `Drafting` round the re-plan joins (ruling 4, recorded: the user
/// approves both together), and a task the batch adds is the session's own.
#[test]
fn a_session_changes_its_own_rounds_tasks() {
    let mut fx = held(false);
    assert_eq!(spawn(&mut fx, "mail")["hold"], "epic:mail");
    planner_started(&mut fx, SECOND);
    let edits = json!([
        amend("t2"),
        add_dep("t2"),
        task_in("t3", "mail/c"),
        amend("t3")
    ]);
    accepted(&second_submits(&mut fx, edits));
    assert_eq!(fx.task("t2").spec.brief, "A new brief");
    assert_eq!(fx.task("t3").spec.brief, "A new brief");
    assert_eq!(fx.task("t3").orch.gate_hold.as_deref(), Some("epic:mail"));

    let mut fx = held(false);
    spawn(&mut fx, "mail");
    planner_started(&mut fx, SECOND);
    accepted(&second_submits(&mut fx, json!([split("t2")])));
    assert_eq!(fx.task("t2x").orch.gate_hold.as_deref(), Some("epic:mail"));
    assert_eq!(hold_state(&fx, "epic:mail"), Some(HoldState::Awaiting));
}

/// Ruling 2, the reviewer's probe: `t2` awaits the user under `epic:mail`, `mail` is
/// re-planned, and the orchestrator's split of `t2` (or a dependency added onto it) is
/// refused as a new task naming the live epic is.
#[test]
fn the_orchestrator_cannot_split_or_add_a_dep_to_a_live_epics_task() {
    let mut fx = replanned_with_t2_awaiting("mail/a");
    let before = tasks(&fx);
    for edit in [split("t2"), add_dep("t2")] {
        let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [edit]})));
        assert!(!ok, "{value}");
        assert!(
            value
                .to_string()
                .contains("epic mail is being planned by its sub-planner"),
            "{value}"
        );
    }
    assert_eq!(tasks(&fx), before);
}

/// Ruling 3: before the gate nothing is approved, so a re-plan's session changes any
/// task of its epic as the first session could.
#[test]
fn before_the_gate_a_replan_changes_any_of_its_epics_tasks() {
    let mut fx = launched(false);
    assert!(spawn(&mut fx, "mail")["hold"].is_null());
    planner_started(&mut fx, PLANNER);
    let edits = json!([task_in("t2", "mail/a"), task_in("t3", "mail/b")]);
    accepted(&submit_epic(&mut fx, edits));
    planner_ended(&mut fx, ("mail", 1), ScoutEnd::Reported);
    assert!(spawn(&mut fx, "mail")["hold"].is_null());
    planner_started(&mut fx, SECOND);
    let edits = json!([amend("t2"), split("t3"), cancel("t2")]);
    accepted(&second_submits(&mut fx, edits));
    assert_eq!(fx.task("t2").state, TaskState::Cancelled);
    assert_eq!(fx.task("t3").state, TaskState::Cancelled, "split");
    assert!(fx.run().task("t3x").is_some());
}
