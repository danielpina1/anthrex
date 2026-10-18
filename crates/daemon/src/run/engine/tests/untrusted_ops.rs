//! Milestone 9.13 task M9.13.9: decision 21, the downgrade. While the session is
//! tainted (`RunOrch.untrusted`), the orchestrator's `override` and `accept_red` become
//! the user's question, carrying the op as its proposal; the user's answer runs it as
//! the user's own, declines it, or drops it. `approve_hold` is refused; `retry` is not
//! affected. A refused op is refused, never asked.

use proto::{HoldState, PlanEdit, TaskState};
use serde_json::json;

use super::control::blocked;
use super::dispatch::replies;
use super::fixture::*;
use super::merge::commit;
use super::orch::{ORCH, edit_plan, error, orch_tool};
use super::orch_ops_resume::red;
use super::wake_notes::{approved, clear, notes};
use crate::run::engine::{Effect, EventKind, OpResult, OrchEvent};
use crate::run::orch::untrusted::UntrustedRecord;

const SOURCE: &str = "review comments on stage 1's pull request";
const ASKED: &str = "asked the user to decide: this session read text from outside the run, so override and accept_red are theirs now; their answer arrives as a message";
/// 14:07 UTC on 2026-10-12.
const AT: u64 = 1_791_814_079;

fn taint(fx: &mut Fixture) {
    fx.run_mut().orch.untrusted = Some(UntrustedRecord {
        at: AT,
        source: SOURCE.into(),
    });
}

/// [`approved`] with `t1` blocked after it started (so it has a start commit), tainted.
fn tainted_blocked_t1() -> Fixture {
    let mut fx = approved();
    let windows = fx.launch_all();
    let (_, window) = windows.iter().find(|(t, _)| t == "t1").cloned().unwrap();
    blocked(&mut fx, window, "environment", "the build cache is locked");
    fx.task_mut("t1").start_commit = Some(BASE.into());
    taint(&mut fx);
    clear(&mut fx);
    fx
}

fn op(fx: &mut Fixture, edit: serde_json::Value) -> Vec<Effect> {
    edit_plan(fx, json!({"edits": [edit]}))
}

fn answer(fx: &mut Fixture, id: u64, choice: Option<u32>) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Orch(OrchEvent::AnswerAsk {
        reply,
        run_id: RUN_ID.into(),
        ask: id,
        choice,
    }))
}

fn override_t1(fx: &mut Fixture) -> Vec<Effect> {
    op(
        fx,
        json!({"op": "override", "task_id": "t1", "reason": "the fix is in"}),
    )
}

fn context() -> String {
    format!("the orchestrator read {SOURCE} at 2026-10-12 14:07 UTC, so you decide")
}

#[test]
fn tainted_override_becomes_an_ask_and_choice_one_overrides_as_the_user() {
    let mut fx = tainted_blocked_t1();
    let edits = fx.run().plan_edits.len();
    let effects = override_t1(&mut fx);
    assert_eq!(replies(&effects), vec![Ok(ASKED.to_string())]);
    assert!(fx.ops("CountCommits").is_empty(), "nothing runs yet");
    let ask = fx.run().orch.ask.clone().expect("asked");
    assert_eq!(ask.question, "override t1? the fix is in");
    assert_eq!(ask.options, ["do it", "leave it"]);
    assert_eq!(ask.context, context());
    assert_eq!(
        ask.proposal,
        Some(PlanEdit::Override {
            task_id: "t1".into(),
            reason: "the fix is in".into()
        })
    );
    assert_eq!(fx.run().plan_edits.len(), edits, "no plan edit recorded");
    assert!(fx.run().orch.handled.is_empty());
    assert_eq!(fx.task("t1").state, TaskState::Blocked);

    // "do it": the user's override, counting first; the reply waits for the count.
    let effects = answer(&mut fx, ask.id, Some(0));
    assert!(replies(&effects).is_empty(), "{effects:#?}");
    assert_eq!(fx.run().orch.ask, None);
    let (count, _) = fx.op("CountCommits");
    let count_t = fx.task("t1").override_count.clone().unwrap();
    assert!(count_t.proposal);
    assert_eq!(count_t.actor, crate::run::engine::Actor::User);
    assert!(
        !notes(&fx)
            .iter()
            .any(|n| n.starts_with("the user approved")),
        "{:?}",
        notes(&fx)
    );
    let effects = fx.done(
        count,
        OpResult::Commits {
            count: 2,
            head: HEAD.into(),
            protected: Vec::new(),
        },
    );
    assert_eq!(
        replies(&effects),
        vec![Ok(
            "task t1 goes to the merge queue without review: the fix is in".to_string()
        )]
    );
    assert_ne!(fx.task("t1").state, TaskState::Blocked);
    assert!(fx.task("t1").merged_without_approval.is_some());
    assert!(
        notes(&fx).contains(&"the user approved: override t1".to_string()),
        "{:?}",
        notes(&fx)
    );
    assert!(!notes(&fx).iter().any(|n| n.starts_with("the user chose")));
    assert!(fx.run().orch.handled.is_empty());
}

/// A proposal pending in `run.json` keeps its op; one without the key loads as `None`.
/// The same for the count's `proposal` (C7), whose absence loads as `false`.
#[test]
fn a_pending_proposal_survives_save_and_restore() {
    let mut fx = tainted_blocked_t1();
    override_t1(&mut fx);
    let json = serde_json::to_value(fx.run()).unwrap();
    let back: crate::run::model::Run = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back.orch.ask, fx.run().orch.ask);
    let mut old = json;
    old["orch"]["ask"]
        .as_object_mut()
        .unwrap()
        .remove("proposal");
    let old: crate::run::model::Run = serde_json::from_value(old).unwrap();
    assert_eq!(old.orch.ask.unwrap().proposal, None);

    let id = fx.run().orch.ask.as_ref().unwrap().id;
    answer(&mut fx, id, Some(0));
    let json = serde_json::to_value(fx.run()).unwrap();
    let back: crate::run::model::Run = serde_json::from_value(json.clone()).unwrap();
    assert!(back.tasks[0].override_count.as_ref().unwrap().proposal);
    let mut old = json;
    old["tasks"][0]["override_count"]
        .as_object_mut()
        .unwrap()
        .remove("proposal");
    let old: crate::run::model::Run = serde_json::from_value(old).unwrap();
    assert!(!old.tasks[0].override_count.as_ref().unwrap().proposal);
}

#[test]
fn tainted_accept_red_declined_notes() {
    let mut fx = red();
    taint(&mut fx);
    clear(&mut fx);
    let before = fx.run().stage(1).unwrap().full.clone();
    let effects = op(
        &mut fx,
        json!({"op": "accept_red", "stage": 1, "reason": "red on main too"}),
    );
    assert_eq!(replies(&effects), vec![Ok(ASKED.to_string())]);
    let ask = fx.run().orch.ask.clone().unwrap();
    assert_eq!(ask.question, "accept_red stage 1? red on main too");
    assert_eq!(ask.context, context());
    let effects = answer(&mut fx, ask.id, Some(1));
    assert_eq!(
        replies(&effects),
        vec![Ok("declined: accept_red stage 1".to_string())]
    );
    assert_eq!(fx.run().stage(1).unwrap().full, before, "nothing changes");
    assert_eq!(notes(&fx), vec!["the user declined: accept_red stage 1"]);
    assert_eq!(fx.run().orch.ask, None);
}

#[test]
fn answer_in_window_drops_the_proposal() {
    let mut fx = tainted_blocked_t1();
    override_t1(&mut fx);
    let id = fx.run().orch.ask.as_ref().unwrap().id;
    let effects = answer(&mut fx, id, None);
    assert_eq!(
        replies(&effects),
        vec![Ok("answered in the window".to_string())]
    );
    assert_eq!(fx.run().orch.ask, None);
    assert!(fx.ops("CountCommits").is_empty());
    assert_eq!(fx.task("t1").state, TaskState::Blocked);
    assert!(notes(&fx).is_empty(), "{:?}", notes(&fx));
}

#[test]
fn a_refused_proposal_is_not_asked() {
    let mut fx = approved();
    fx.launch_all();
    taint(&mut fx);
    let user = super::control::override_task(&mut fx, "t1", "r");
    let user = replies(&user)[0].clone().unwrap_err();
    let effects = override_t1(&mut fx);
    assert_eq!(error(&effects), user);
    assert_eq!(fx.run().orch.ask, None);
    // accept_red with no red stage: the rule's refusal, no ask.
    let effects = op(
        &mut fx,
        json!({"op": "accept_red", "stage": 2, "reason": "r"}),
    );
    assert_eq!(error(&effects), format!("run {RUN_ID} has no stage 2"));
    assert_eq!(fx.run().orch.ask, None);
}

#[test]
fn tainted_approve_hold_is_refused() {
    let mut fx = super::gate_holds::held(false);
    super::gate_holds::awaiting(&mut fx);
    taint(&mut fx);
    let effects = op(
        &mut fx,
        json!({"op": "approve_hold", "hold": "epic:mail", "reason": "the epic is in the goal"}),
    );
    assert_eq!(
        error(&effects),
        "hold epic:mail waits for the user: this session read text from outside the run"
    );
    let hold = (fx.run().orch.gate_holds.iter())
        .find(|h| h.id == "epic:mail")
        .unwrap();
    assert_eq!(hold.state, HoldState::Awaiting);
    assert_eq!(fx.run().orch.ask, None);
    assert!(fx.run().orch.handled.is_empty());
}

#[test]
fn retry_is_not_affected_by_taint() {
    let mut fx = tainted_blocked_t1();
    let effects = op(
        &mut fx,
        json!({"op": "retry", "task_id": "t1", "reason": "transient"}),
    );
    let text = replies(&effects)[0].clone().unwrap();
    assert!(text.starts_with("task t1 retried at rung 2: "), "{text}");
    assert_eq!(fx.run().orch.ask, None);
    assert_eq!(fx.task("t1").orch.orch_retries, 1);
}

#[test]
fn do_it_on_accept_red_logs_one_line() {
    let mut fx = red();
    taint(&mut fx);
    op(
        &mut fx,
        json!({"op": "accept_red", "stage": 1, "reason": "red on main too"}),
    );
    let id = fx.run().orch.ask.as_ref().unwrap().id;
    let effects = answer(&mut fx, id, Some(0));
    let sha7 = &commit(1)[..7];
    assert_eq!(
        replies(&effects),
        vec![Ok(format!(
            "stage 1's red tier 3 on {sha7} is accepted; completion and delivery go on"
        ))]
    );
    let lines: Vec<&str> = (fx.run().log.iter())
        .map(|e| e.text.as_str())
        .filter(|t| t.contains("tier 3 red on"))
        .collect();
    assert_eq!(
        lines,
        [format!(
            "stage 1: tier 3 red on {sha7} accepted by the user"
        )]
    );
    assert!(notes(&fx).contains(&"the user approved: accept_red stage 1".to_string()));
    assert!(fx.run().orch.handled.is_empty());
    assert!(fx.run().stage(1).unwrap().full.accepted_red.is_some());
}

#[test]
fn a_new_ask_replaces_a_proposal() {
    let mut fx = tainted_blocked_t1();
    override_t1(&mut fx);
    let old = fx.run().orch.ask.as_ref().unwrap().id;
    orch_tool(&mut fx, ORCH, "ask_user", json!({"question": "go on?"}));
    let ask = fx.run().orch.ask.clone().unwrap();
    assert_eq!(ask.proposal, None);
    let effects = answer(&mut fx, old, Some(0));
    assert_eq!(
        replies(&effects)[0].clone().unwrap_err(),
        "that question was replaced; answer the new one"
    );
    assert!(fx.ops("CountCommits").is_empty());
}
