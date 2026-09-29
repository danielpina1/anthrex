//! Milestone 9 task M9.8's review fixes:
//! - a re-plan's work waits for its own round, never an `Awaiting` one (ruling 1);
//! - a sub-planner amends only its epic's unapproved tasks (ruling 2);
//! - a stale session's tool call is refused (ruling 3);
//! - the re-plan and epic bounds (ruling 4).

use proto::{HoldState, TaskState};
use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use super::gate_holds::held;
use super::orch::{ORCH, edit_plan, error, orch_tool};
use super::planners::{
    PLANNER, epic, hold_state, planner_ended, planner_started, planner_tool, planning_mail, spawn,
    spawn_args, submit_epic, task_in,
};
use super::planners_holds::reply;
use super::promote::{hold_verdict, prepared};
use crate::run::engine::planners::{EPIC_RECORDED, MAX_EPICS, MAX_REPLANS_PER_EPIC};
use crate::run::engine::{Effect, ScoutEnd};
use crate::run::orch::PlannerPhase;

pub(super) const SECOND: u32 = PLANNER + 1;

/// `planning_mail` whose planner submitted `t2` (owning `module`): `epic:mail` awaits
/// the user, the session has ended, and `mail` is re-planned (session 2 live in
/// [`SECOND`]).
pub(super) fn replanned_with_t2_awaiting(module: &str) -> Fixture {
    let mut fx = planning_mail(false);
    submit_epic(&mut fx, json!([task_in("t2", module)]));
    assert_eq!(hold_state(&fx, "epic:mail"), Some(HoldState::Awaiting));
    planner_ended(&mut fx, ("mail", 1), ScoutEnd::Reported);
    assert_eq!(spawn(&mut fx, "mail")["hold"], "epic:mail.2");
    assert_eq!(hold_state(&fx, "epic:mail.2"), Some(HoldState::Drafting));
    planner_started(&mut fx, SECOND);
    fx
}

pub(super) fn second_submits(fx: &mut Fixture, edits: serde_json::Value) -> Vec<Effect> {
    let args = json!({"edits": edits});
    planner_tool(fx, (SECOND, "mail"), "submit_epic", args)
}

/// Ruling 1, the reviewer's sequence: `t2` awaits the user under `epic:mail`; a
/// re-plan opens `epic:mail.2`; the user approves `epic:mail`; session 2's `t3` still
/// waits, under its own round.
#[test]
fn a_replan_never_joins_an_awaiting_round() {
    let mut fx = replanned_with_t2_awaiting("mail/a");
    hold_verdict(&mut fx, "epic:mail", true);
    assert!(prepared(&fx, "t2"));
    let effects = second_submits(&mut fx, json!([task_in("t3", "mail/b")]));
    assert_eq!(replies(&effects), vec![Ok(EPIC_RECORDED.to_string())]);
    assert_eq!(fx.task("t3").orch.gate_hold.as_deref(), Some("epic:mail.2"));
    assert_eq!(hold_state(&fx, "epic:mail.2"), Some(HoldState::Awaiting));
    fx.tick();
    assert!(
        !prepared(&fx, "t3"),
        "approving epic:mail releases none of t3"
    );
    hold_verdict(&mut fx, "epic:mail.2", true);
    assert!(prepared(&fx, "t3"));
}

/// Ruling 1 for a round the orchestrator's own addition opened: `t2` (added to the
/// finished epic) awaits the user after the orchestrator's `submit`; a re-plan opens
/// `epic:mail.2` for session 2's work, which approving `epic:mail` does not release.
#[test]
fn a_replan_opens_a_round_past_an_orchestrator_round_awaiting_the_user() {
    let mut fx = held(false);
    edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    assert_eq!(hold_state(&fx, "epic:mail"), Some(HoldState::Awaiting));
    assert_eq!(spawn(&mut fx, "mail")["hold"], "epic:mail.2");
    planner_started(&mut fx, SECOND);
    let effects = second_submits(&mut fx, json!([task_in("t3", "mail/b")]));
    assert_eq!(replies(&effects), vec![Ok(EPIC_RECORDED.to_string())]);
    assert_eq!(fx.task("t3").orch.gate_hold.as_deref(), Some("epic:mail.2"));
    hold_verdict(&mut fx, "epic:mail", true);
    fx.tick();
    assert!(prepared(&fx, "t2"));
    assert!(!prepared(&fx, "t3"));
}

fn amend(id: &str) -> serde_json::Value {
    json!({"op": "amend_task", "task_id": id, "brief": "A new brief"})
}

fn amend_refused(fx: &mut Fixture, id: &str) {
    let before = fx.task(id).spec.brief.clone();
    let outbox = fx.run().outbox.len();
    let effects = second_submits(fx, json!([amend(id)]));
    let (ok, text) = reply(&effects);
    assert!(!ok, "{text}");
    let expected = format!("task {id}: a sub-planner changes only its own round's tasks");
    assert!(text.contains(&expected), "{text}");
    assert_eq!(fx.task(id).spec.brief, before);
    assert_eq!(fx.run().outbox.len(), outbox, "nothing is delivered");
    assert!(
        !effects.iter().any(|e| matches!(e, Effect::Deliver { .. })),
        "{effects:#?}"
    );
}

/// Ruling 2: session 2 may not amend its epic's `Working` task, nor an approved one
/// that has not started. (Its amend of a task still held in an earlier round is refused
/// too since the second review: `planners_rounds.rs`.)
#[test]
fn a_planner_amends_only_its_epics_unapproved_tasks() {
    let mut fx = replanned_with_t2_awaiting("mail/a");
    hold_verdict(&mut fx, "epic:mail", true);
    // Approved, not yet started.
    fx.task_mut("t2").state = TaskState::Queued;
    amend_refused(&mut fx, "t2");
    fx.task_mut("t2").state = TaskState::Working;
    amend_refused(&mut fx, "t2");
}

/// Ruling 3, the reviewer's case: no reader slot is free, session 1 has failed and a
/// re-plan is queued; session 1's window cannot submit, and the re-plan stays queued.
#[test]
fn a_stale_sessions_submit_is_refused_and_the_queued_replan_stays_queued() {
    let mut fx = planning_mail(false);
    planner_ended(
        &mut fx,
        ("mail", 1),
        ScoutEnd::Failed {
            reason: "gone".into(),
        },
    );
    fx.run_mut().limits.max_readers = 0;
    assert_eq!(spawn(&mut fx, "mail")["state"], "queued");
    assert_eq!(epic(&fx, "mail").phase, PlannerPhase::Queued);
    let effects = submit_epic(&mut fx, json!([task_in("t2", "mail")]));
    let text = error(&effects);
    assert!(
        text.contains("is not the sub-planner of epic mail"),
        "{text}"
    );
    assert_eq!(epic(&fx, "mail").phase, PlannerPhase::Queued);
    assert!(fx.run().task("t2").is_none());
    // Refused for the queued phase alone, even were the old session's end not
    // recorded.
    fx.run_mut().orch.epics[0]
        .sessions
        .last_mut()
        .unwrap()
        .ended_at = None;
    let effects = submit_epic(&mut fx, json!([task_in("t2", "mail")]));
    assert!(error(&effects).contains("is not the sub-planner of epic mail"));
    assert_eq!(epic(&fx, "mail").phase, PlannerPhase::Queued);
    assert!(fx.run().task("t2").is_none());
}

/// Ruling 3: a session that has ended (its `ended_at` set) cannot call a tool, even
/// from the latest session's window.
#[test]
fn an_ended_sessions_call_is_refused() {
    let mut fx = planning_mail(false);
    fx.run_mut().orch.epics[0]
        .sessions
        .last_mut()
        .unwrap()
        .ended_at = Some(1);
    let effects = submit_epic(&mut fx, json!([task_in("t2", "mail")]));
    assert!(error(&effects).contains("is not the sub-planner of epic mail"));
    assert!(fx.run().task("t2").is_none());
}

/// Ruling 4: at most [`MAX_REPLANS_PER_EPIC`] re-plans of one epic.
#[test]
fn replans_per_epic_are_bounded() {
    let mut fx = planning_mail(false);
    for n in 1..=MAX_REPLANS_PER_EPIC {
        planner_ended(&mut fx, ("mail", n), ScoutEnd::Reported);
        fx.run_mut().orch.epics[0].phase = PlannerPhase::Finished;
        spawn(&mut fx, "mail");
        planner_started(&mut fx, PLANNER + n);
    }
    fx.run_mut().orch.epics[0].phase = PlannerPhase::Finished;
    let args = spawn_args("mail", &["crates/mail/**"], "Once more");
    let text = error(&orch_tool(&mut fx, ORCH, "spawn_subplanner", args));
    assert_eq!(
        text,
        format!(
            "epic mail was already re-planned {MAX_REPLANS_PER_EPIC} times, the most one epic allows"
        )
    );
}

/// Ruling 4: at most [`MAX_EPICS`] epics in one run.
#[test]
fn epics_per_run_are_bounded() {
    let mut fx = planning_mail(false);
    for n in 1..MAX_EPICS {
        spawn(&mut fx, &format!("e{n}"));
    }
    assert_eq!(fx.run().orch.epics.len(), MAX_EPICS);
    let args = spawn_args("last", &["crates/last/**"], "One too many");
    let text = error(&orch_tool(&mut fx, ORCH, "spawn_subplanner", args));
    assert_eq!(
        text,
        format!("run {RUN_ID} already has {MAX_EPICS} epics, the most one run allows")
    );
}
