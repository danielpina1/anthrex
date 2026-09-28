//! Milestone 9 task M9.7: decision 28's approval holds. Work the orchestrator adds for a
//! new epic of a running run waits for the user; it is neither runnable nor pre-warmed;
//! the user's verdict releases or cancels it. M8a ruling N5's dependency hold is left
//! alone.

use proto::{BlockReason, HoldState, RunState, TaskState};
use serde_json::{Value, json};

use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, add, answer, edit_plan, launched, orch_tool};
use crate::run::engine::gate_holds::{PROMOTION, submitted};
use crate::run::engine::{Effect, EventKind, OrchEvent};
use crate::run::orch::PlannerPhase;

fn in_epic(id: &str, module: &str, epic: &str) -> Value {
    let mut edit = add(id, module);
    edit["task"]["epic"] = json!(epic);
    edit
}

fn spawn(fx: &mut Fixture, epic: &str) -> Value {
    let args = json!({"epic": epic, "title": format!("Epic {epic}"),
        "area": [format!("crates/{epic}/**")], "brief": format!("Plan {epic}")});
    let (ok, value) = answer(&orch_tool(fx, ORCH, "spawn_subplanner", args));
    assert!(ok, "{value}");
    value
}

/// A running run (`t1`, approved by the user or `--yes`) whose orchestrator started a
/// new epic `mail` and, once its sub-planner finished, added `t2` to it.
fn held(yes: bool) -> Fixture {
    let mut fx = launched(yes);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    if !yes {
        fx.approve();
    }
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(spawn(&mut fx, "mail")["hold"], "epic:mail");
    // Task M9.8's sub-planner would add the epic's tasks; here it has finished.
    fx.run_mut().orch.epics[0].phase = PlannerPhase::Finished;
    let (ok, value) = answer(&edit_plan(
        &mut fx,
        json!({"edits": [in_epic("t2", "mail", "mail")]}),
    ));
    assert!(ok, "{value}");
    assert_eq!(value["held"], "epic:mail");
    fx
}

fn hold_state(fx: &Fixture, id: &str) -> HoldState {
    fx.run()
        .orch
        .gate_holds
        .iter()
        .find(|h| h.id == id)
        .unwrap()
        .state
}

fn verdict(fx: &mut Fixture, hold: &str, approve: bool) -> Vec<Effect> {
    let reply = fx.reply();
    let (run_id, hold) = (RUN_ID.to_string(), hold.to_string());
    fx.next(EventKind::Orch(if approve {
        OrchEvent::ApproveHold {
            reply,
            run_id,
            hold,
        }
    } else {
        OrchEvent::RejectHold {
            reply,
            run_id,
            hold,
        }
    }))
}

fn awaiting(fx: &mut Fixture) {
    let now = fx.now;
    submitted(fx.run_mut(), "epic:mail", now);
    assert_eq!(hold_state(fx, "epic:mail"), HoldState::Awaiting);
}

#[test]
fn new_epic_on_a_running_run_is_held() {
    let fx = held(false);
    let run = fx.run();
    let hold = &run.orch.gate_holds[0];
    assert_eq!(hold.id, "epic:mail");
    assert_eq!(hold.state, HoldState::Drafting);
    assert_eq!(hold.tasks, vec!["t2".to_string()]);
    assert_eq!(run.orch.epics[0].gate_hold.as_deref(), Some("epic:mail"));
    assert_eq!(fx.task("t2").orch.gate_hold.as_deref(), Some("epic:mail"));
    // Its tasks carry it; the orchestrator's other additions do not.
    let mut fx = fx;
    edit_plan(&mut fx, json!({"edits": [add("t3", "sms")]}));
    assert_eq!(fx.task("t3").orch.gate_hold, None);
    // The control: a new epic while planning gets no hold.
    let mut planning = launched(false);
    assert_eq!(spawn(&mut planning, "mail")["hold"], Value::Null);
    assert!(planning.run().orch.gate_holds.is_empty());
}

#[test]
fn held_tasks_are_not_runnable_or_prewarmed() {
    let mut fx = held(false);
    awaiting(&mut fx);
    fx.tick();
    assert_eq!(fx.task("t2").state, TaskState::Queued);
    assert!(
        fx.ops("PrepareWorktree")
            .iter()
            .all(|(_, k)| op_task(k) != "t2")
    );
    // At the gate, a held task is not pre-warmed either.
    let mut gate = launched(false);
    let effects = edit_plan(
        &mut gate,
        json!({"edits": [add("t1", "auth"), add("t9", "mail")]}),
    );
    assert!(answer(&effects).0);
    gate.run_mut()
        .orch
        .gate_holds
        .push(crate::run::orch::GateHoldRecord {
            id: "epic:x".into(),
            kind: proto::HoldKind::Epic { epic: "x".into() },
            state: HoldState::Awaiting,
            tasks: vec!["t9".into()],
            created_at: 0,
            decided_at: None,
            decided_by: None,
        });
    gate.task_mut("t9").orch.gate_hold = Some("epic:x".into());
    let effects = edit_plan(&mut gate, json!({"edits": [], "submit": true}));
    assert_eq!(tasks_of(&effects, "PrepareWorktree"), vec!["t1"]);
}

#[test]
fn approve_hold_releases_its_tasks() {
    let mut fx = held(false);
    awaiting(&mut fx);
    let effects = verdict(&mut fx, "epic:mail", true);
    assert_eq!(
        replies(&effects),
        vec![Ok(format!(
            "hold epic:mail of run {RUN_ID} approved: 1 task may start"
        ))]
    );
    let hold = fx.run().orch.gate_holds[0].clone();
    assert_eq!(hold.state, HoldState::Approved);
    assert_eq!(hold.decided_by.as_deref(), Some("user"));
    assert_eq!(hold.decided_at, Some(fx.now));
    assert_eq!(fx.task("t2").state, TaskState::Preparing, "dispatched");
    assert_eq!(tasks_of(&effects, "PrepareWorktree"), vec!["t2"]);
}

#[test]
fn reject_hold_cancels_its_tasks_and_blocks_dependents() {
    let mut fx = held(false);
    let mut t3 = add("t3", "sms");
    t3["task"]["deps"] = json!(["t2"]);
    edit_plan(&mut fx, json!({"edits": [t3]}));
    assert_eq!(fx.task("t3").orch.gate_hold, None);
    awaiting(&mut fx);
    let effects = verdict(&mut fx, "epic:mail", false);
    assert_eq!(
        replies(&effects),
        vec![Ok(format!(
            "hold epic:mail of run {RUN_ID} rejected: 1 task cancelled"
        ))]
    );
    assert_eq!(hold_state(&fx, "epic:mail"), HoldState::Rejected);
    assert_eq!(fx.task("t2").state, TaskState::Cancelled);
    let t3 = fx.task("t3");
    assert_eq!(t3.state, TaskState::Blocked);
    assert_eq!(t3.block.as_ref().unwrap().reason, BlockReason::DepCancelled);
    assert_eq!(
        fx.task("t1").state,
        TaskState::Preparing,
        "the rest runs on"
    );
}

#[test]
fn hold_verdict_errors() {
    let mut fx = held(false);
    let refused = |fx: &mut Fixture, hold: &str, approve: bool| {
        let effects = verdict(fx, hold, approve);
        replies(&effects).remove(0).unwrap_err()
    };
    assert_eq!(
        refused(&mut fx, "epic:nope", true),
        format!("run {RUN_ID} has no hold epic:nope")
    );
    assert_eq!(
        refused(&mut fx, "epic:mail", true),
        "hold epic:mail is drafting"
    );
    awaiting(&mut fx);
    verdict(&mut fx, "epic:mail", true);
    assert_eq!(
        refused(&mut fx, "epic:mail", true),
        "hold epic:mail is approved"
    );
    assert_eq!(
        refused(&mut fx, "epic:mail", false),
        "hold epic:mail is approved"
    );
    let reply = fx.reply();
    let effects = fx.next(EventKind::Orch(OrchEvent::ApproveHold {
        reply,
        run_id: "ghost".into(),
        hold: "epic:mail".into(),
    }));
    assert_eq!(
        replies(&effects),
        vec![Err("unknown run ghost".to_string())]
    );
}

#[test]
fn with_yes_an_epic_hold_is_approved_on_submit() {
    let mut fx = held(true);
    let now = fx.now;
    submitted(fx.run_mut(), "epic:mail", now);
    let hold = fx.run().orch.gate_holds[0].clone();
    assert_eq!(hold.state, HoldState::Approved);
    assert_eq!(hold.decided_by.as_deref(), Some("--yes"));
    fx.tick();
    assert_eq!(fx.task("t2").state, TaskState::Preparing);
    // The control: without `--yes` it awaits the user.
    let mut fx = held(false);
    awaiting(&mut fx);
    assert_eq!(PROMOTION, "promotion");
}

#[test]
fn gate_holds_do_not_touch_dependency_holds() {
    let mut fx = held(false);
    awaiting(&mut fx);
    // M8a ruling N5's dependency hold, as a started task that gained a dependency
    // has it.
    let task = fx.task_mut("t2");
    task.awaiting_deps = true;
    task.held_answered = true;
    let deps_before = (
        task.awaiting_deps,
        task.held_answered,
        task.spec.deps.clone(),
    );
    verdict(&mut fx, "epic:mail", true);
    let task = fx.task("t2");
    assert_eq!(
        (
            task.awaiting_deps,
            task.held_answered,
            task.spec.deps.clone()
        ),
        deps_before
    );
    assert_eq!(task.orch.gate_hold.as_deref(), Some("epic:mail"));
}

/// M9.7 review fixes, ruling 2: the children of a held task inherit its hold, although
/// they name no epic.
#[test]
fn split_children_of_an_epic_held_task_stay_held() {
    let mut fx = held(false);
    let child = |id: &str, module: &str| add(id, module)["task"].clone();
    let split = json!({"op": "split_task", "task_id": "t2",
        "into": [child("t2a", "mail_a"), child("t2b", "mail_b")]});
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [split]})));
    assert!(ok, "{value}");
    assert_eq!(value["held"], "epic:mail");
    awaiting(&mut fx);
    fx.tick();
    for id in ["t2a", "t2b"] {
        assert_eq!(fx.task(id).spec.epic, None, "{id} names no epic");
        assert_eq!(fx.task(id).orch.gate_hold.as_deref(), Some("epic:mail"));
        assert_eq!(fx.task(id).state, TaskState::Queued, "{id} waits");
        assert!(
            fx.ops("PrepareWorktree")
                .iter()
                .all(|(_, k)| op_task(k) != id),
            "{id} is not dispatched"
        );
    }
    let effects = verdict(&mut fx, "epic:mail", false);
    assert_eq!(
        replies(&effects),
        vec![Ok(format!(
            "hold epic:mail of run {RUN_ID} rejected: 2 tasks cancelled"
        ))]
    );
    for id in ["t2a", "t2b"] {
        assert_eq!(fx.task(id).state, TaskState::Cancelled, "{id}");
    }
}

/// M9.7 review fixes, ruling 2 (**pinning**): amending a held task keeps its hold,
/// whether the amend re-resolves the task or not.
#[test]
fn amending_a_held_task_keeps_its_hold() {
    let mut fx = held(false);
    let amend = json!({"op": "amend_task", "task_id": "t2", "brief": "New brief"});
    let resize = json!({"op": "amend_task", "task_id": "t2", "size": "M"});
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [amend, resize]})));
    assert!(ok, "{value}");
    assert_eq!(fx.task("t2").orch.gate_hold.as_deref(), Some("epic:mail"));
    assert_eq!(fx.run().orch.gate_holds[0].tasks, vec!["t2".to_string()]);
    awaiting(&mut fx);
    fx.tick();
    assert_eq!(fx.task("t2").state, TaskState::Queued);
}

/// M9.7 second review, ruling 3: a user's own `run edit` split of a held task releases
/// the work (the user's edit is theirs: the children carry no hold). The split parent
/// is cancelled, so an approval counts none of it.
#[test]
fn a_users_split_of_a_held_task_releases_it_and_the_approval_counts_none() {
    use super::holds::plan_task;
    let mut fx = held(false);
    let split = proto::PlanEdit::SplitTask {
        task_id: "t2".into(),
        into: vec![
            plan_task("t2a", "[\"crates/mail_a/**\"]"),
            plan_task("t2b", "[\"crates/mail_b/**\"]"),
        ],
    };
    let effects = super::dispatch::edit(&mut fx, vec![split]);
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    assert_eq!(fx.task("t2").state, TaskState::Cancelled);
    for id in ["t2a", "t2b"] {
        assert_eq!(fx.task(id).orch.gate_hold, None, "{id} is the user's");
    }
    awaiting(&mut fx);
    let effects = verdict(&mut fx, "epic:mail", true);
    assert_eq!(
        replies(&effects),
        vec![Ok(format!(
            "hold epic:mail of run {RUN_ID} approved: 0 tasks may start"
        ))]
    );
}

/// M9.7 second review, ruling 3: a verdict counts only the hold's unfinished tasks,
/// approve as reject already did.
#[test]
fn verdicts_count_only_unfinished_tasks() {
    for approve in [true, false] {
        let mut fx = held(false);
        edit_plan(&mut fx, json!({"edits": [in_epic("t3", "sms", "mail")]}));
        assert_eq!(fx.run().orch.gate_holds[0].tasks.len(), 2);
        let cancel = proto::PlanEdit::CancelTask {
            task_id: "t2".into(),
        };
        let effects = super::dispatch::edit(&mut fx, vec![cancel]);
        assert!(replies(&effects)[0].is_ok(), "{effects:?}");
        awaiting(&mut fx);
        let effects = verdict(&mut fx, "epic:mail", approve);
        let text = if approve {
            "approved: 1 task may start"
        } else {
            "rejected: 1 task cancelled"
        };
        assert_eq!(
            replies(&effects),
            vec![Ok(format!("hold epic:mail of run {RUN_ID} {text}"))]
        );
    }
}
