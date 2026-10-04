//! Milestone 9 task M9.8: sub-planners (decisions 21, 22, 31, 32). `spawn_subplanner`'s
//! checks, the reader slot and its order, `submit_epic` accepted and rejected, a
//! re-plan, a failed planner. Confinement is `planners_confine.rs`, the holds
//! `planners_holds.rs`, run scouts `run_scouts.rs`.

use proto::{AgentRole, HoldState, RunPath, RunState, ToolCall};
use serde_json::{Value, json};

use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, add, answer, edit_plan, error, launched, orch_tool};
use crate::run::engine::planners::EPIC_RECORDED;
use crate::run::engine::{Effect, EventKind, OpKind, OpResult, OrchEvent, ScoutEnd};
use crate::run::orch::{EpicRecord, PlannerPhase, RunScout, RunScoutState};

/// The sub-planner of epic `mail`'s window in these tests.
pub(super) const PLANNER: u32 = 91;

pub(super) fn spawn_args(epic: &str, area: &[&str], brief: &str) -> Value {
    json!({"epic": epic, "title": format!("Epic {epic}"), "area": area, "brief": brief})
}

/// `spawn_subplanner` of `epic` over `crates/<epic>/**`, which must be accepted.
pub(super) fn spawn(fx: &mut Fixture, epic: &str) -> Value {
    let area = format!("crates/{epic}/**");
    let args = spawn_args(epic, &[&area], &format!("Plan {epic}"));
    let (ok, value) = answer(&orch_tool(fx, ORCH, "spawn_subplanner", args));
    assert!(ok, "{value}");
    value
}

/// The latest `StartPlanner` answered with `window`.
pub(super) fn planner_started(fx: &mut Fixture, window: u32) -> Vec<Effect> {
    let (op, _) = fx.op("StartPlanner");
    fx.done(op, OpResult::PlannerStarted { window_id: window })
}

/// A running run (`t1`, approved by the user or `--yes`) whose orchestrator spawned
/// epic `mail`; its sub-planner is live in window [`PLANNER`].
pub(super) fn planning_mail(yes: bool) -> Fixture {
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
    planner_started(&mut fx, PLANNER);
    fx
}

/// A sub-planner's tool call for `epic` from `window`.
pub(super) fn planner_tool(
    fx: &mut Fixture,
    (window, epic): (u32, &str),
    tool: &str,
    args: Value,
) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Orch(OrchEvent::Tool {
        reply,
        call: ToolCall {
            run_id: RUN_ID.into(),
            task_id: None,
            role: AgentRole::Planner,
            window_id: window,
            tool: tool.into(),
            args,
            scout_id: None,
            epic: Some(epic.into()),
            chain: None,
            lane: None,
        },
        refusals: Vec::new(),
    }))
}

/// `mail`'s planner submits `edits`.
pub(super) fn submit_epic(fx: &mut Fixture, edits: Value) -> Vec<Effect> {
    planner_tool(
        fx,
        (PLANNER, "mail"),
        "submit_epic",
        json!({"edits": edits}),
    )
}

/// An `add_task` of `id` owning `crates/<module>/**`, with no epic.
pub(super) fn task_in(id: &str, module: &str) -> Value {
    add(id, module)
}

pub(super) fn epic<'a>(fx: &'a Fixture, name: &str) -> &'a EpicRecord {
    fx.run().orch.epics.iter().find(|e| e.epic == name).unwrap()
}

pub(super) fn hold_state(fx: &Fixture, id: &str) -> Option<HoldState> {
    let run = fx.run();
    run.orch
        .gate_holds
        .iter()
        .find(|h| h.id == id)
        .map(|h| h.state)
}

/// The ended-session event of `epic`'s session `session`.
pub(super) fn planner_ended(
    fx: &mut Fixture,
    (epic, session): (&str, u32),
    outcome: ScoutEnd,
) -> Vec<Effect> {
    fx.next(EventKind::Orch(OrchEvent::PlannerEnded {
        run_id: RUN_ID.into(),
        epic: epic.into(),
        session,
        outcome,
        usage: proto::TokenUsage {
            input: 10,
            output: 5,
            ..Default::default()
        },
    }))
}

fn rejected_messages(effects: &[Effect]) -> Vec<String> {
    let (ok, value) = answer(effects);
    assert!(!ok, "{value}");
    assert_eq!(value["accepted"], false, "{value}");
    value["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["message"].as_str().unwrap().to_string())
        .collect()
}

#[test]
fn spawn_subplanner_validates_epic_and_area() {
    let mut fx = launched(false);
    let refused =
        |fx: &mut Fixture, args: Value| error(&orch_tool(fx, ORCH, "spawn_subplanner", args));
    // The epic's pattern (the tools' parser).
    let text = refused(&mut fx, spawn_args("Mail!", &["crates/mail/**"], "b"));
    assert!(text.starts_with("invalid arguments: epic: "), "{text}");
    // The area's form: a literal path or `<literal>/**`.
    for glob in ["crates/mail/*.rs", "crates/*/src/**", "crates/mail/**/x"] {
        let text = refused(&mut fx, spawn_args("mail", &[glob], "b"));
        assert_eq!(
            text,
            format!("epic mail: area: {glob} must be a literal path or end in /**")
        );
    }
    let text = refused(&mut fx, spawn_args("mail", &["/abs/**"], "b"));
    assert!(text.starts_with("epic mail: area: /abs/**: "), "{text}");
    assert!(fx.run().orch.epics.is_empty());
    // A literal path and a `/**` are accepted; another epic may not overlap them.
    spawn(&mut fx, "mail");
    let args = spawn_args("web", &["docs/web.md", "crates/mail/src/**"], "b");
    assert_eq!(
        refused(&mut fx, args),
        "epic web: area: overlaps epic mail's area (crates/mail/**)"
    );
    let args = spawn_args("web", &["docs/web.md", "crates/web/**"], "b");
    let (ok, value) = answer(&orch_tool(&mut fx, ORCH, "spawn_subplanner", args));
    assert!(ok, "{value}");
    assert_eq!(fx.run().orch.epics.len(), 2);
}

#[test]
fn spawn_subplanner_sets_path_large() {
    let mut fx = launched(false);
    assert_eq!(fx.run().path, Some(RunPath::Plan));
    spawn(&mut fx, "mail");
    assert_eq!(fx.run().path, Some(RunPath::Large));
}

/// Decision 31: a planner holds a reader slot while live, and when one frees the order
/// is deciders, reviewers, sub-planners, then run scouts. (Research and review tasks
/// come after them: task M9.9.)
#[test]
fn planner_waits_for_a_reader_slot_and_the_order_is_deciders_reviewers_planners_scouts_kinds() {
    use super::deciders::{decide_op, slot_taken, summary};
    use super::merge::pending_one;
    // `max_readers = 1`: t2's review holds the slot, t1's decider and t3's review wait.
    let (mut fx, decider_id, review) = slot_taken();
    let mut queued = EpicRecord::new("mail", PlannerPhase::Queued);
    queued.started_at = fx.now;
    fx.run_mut().orch.epics.push(queued);
    let now = fx.now;
    fx.run_mut().orch.run_scouts.push(RunScout {
        id: format!("{H4}-api"),
        question: "Where is the API?".into(),
        area: vec!["crates/api/**".into()],
        web: false,
        state: RunScoutState::Queued,
        queued_at: now,
        started_at: None,
        ended_at: None,
        window_id: None,
    });
    let effects = fx.tick();
    assert!(ops_in(&effects, "StartPlanner").is_empty(), "{effects:#?}");
    assert!(ops_in(&effects, "StartScout").is_empty(), "{effects:#?}");
    // The slot frees: the decider first.
    let effects = fx.done(
        review,
        OpResult::Failed {
            message: "x".into(),
        },
    );
    assert_eq!(decide_op(&effects).1, decider_id);
    assert!(ops_in(&effects, "StartPlanner").is_empty(), "{effects:#?}");
    // Then t3's reviewer.
    let (op, _) = pending_one(&fx, "Decide", Some("t1"));
    let effects = fx.decided(op, summary(&["one line"]));
    assert_eq!(ops_in(&effects, "PrepareReview").len(), 1, "{effects:#?}");
    assert!(ops_in(&effects, "StartPlanner").is_empty(), "{effects:#?}");
    // Then the sub-planner, not the scout.
    let (op, _) = pending_one(&fx, "PrepareReview", Some("t3"));
    let effects = fx.done(
        op,
        OpResult::Failed {
            message: "x".into(),
        },
    );
    assert_eq!(ops_in(&effects, "StartPlanner").len(), 1, "{effects:#?}");
    assert!(ops_in(&effects, "StartScout").is_empty(), "{effects:#?}");
    assert_eq!(epic(&fx, "mail").phase, PlannerPhase::Planning);
    assert_eq!(crate::run::engine::schedule::readers_busy(fx.run()), 1);
    // Then the run scout, once the planner's session ends.
    let failed = ScoutEnd::Failed {
        reason: "boom".into(),
    };
    let effects = planner_ended(&mut fx, ("mail", 1), failed);
    assert_eq!(ops_in(&effects, "StartScout").len(), 1, "{effects:#?}");
}

#[test]
fn accepted_submit_finishes_and_retires() {
    let mut fx = planning_mail(false);
    let edits = json!([task_in("t2", "mail")]);
    let note = "mail needs the auth token interface";
    let args = json!({"edits": edits, "note": note});
    let effects = planner_tool(&mut fx, (PLANNER, "mail"), "submit_epic", args);
    assert_eq!(replies(&effects), vec![Ok(EPIC_RECORDED.to_string())]);
    assert!(
        effects.contains(&Effect::PlannerAccepted { window_id: PLANNER }),
        "{effects:#?}"
    );
    let e = epic(&fx, "mail");
    assert_eq!(e.phase, PlannerPhase::Finished);
    assert_eq!(e.ended_at, Some(fx.now));
    assert_eq!(e.sessions[0].ended_at, Some(fx.now));
    assert_eq!((e.edits_accepted, e.edits_rejected), (1, 0));
    assert_eq!(e.note.as_deref(), Some(note));
    // Its task is the epic's, and the planner's slot is free again.
    assert_eq!(fx.task("t2").spec.epic.as_deref(), Some("mail"));
    assert_eq!(crate::run::engine::schedule::readers_busy(fx.run()), 0);
    let notes = &fx.run().orch.orchestrator.as_ref().unwrap().notes;
    assert!(
        notes.contains(&"sub-planner mail finished with 1 task".to_string()),
        "{notes:?}"
    );
}

#[test]
fn second_submit_is_refused() {
    let mut fx = planning_mail(false);
    submit_epic(&mut fx, json!([task_in("t2", "mail")]));
    let effects = submit_epic(&mut fx, json!([task_in("t3", "mail")]));
    assert_eq!(
        error(&effects),
        "submit_epic was already accepted for epic mail"
    );
    assert!(fx.run().task("t3").is_none());
}

#[test]
fn submit_epic_for_another_epic_is_refused() {
    let mut fx = planning_mail(false);
    spawn(&mut fx, "web");
    planner_started(&mut fx, PLANNER + 1);
    // `mail`'s window naming `web`, and `web`'s naming `mail`.
    let args = json!({"edits": [task_in("w1", "web")]});
    let effects = planner_tool(&mut fx, (PLANNER, "web"), "submit_epic", args.clone());
    assert_eq!(
        error(&effects),
        format!("this window is not the sub-planner of epic web of run {RUN_ID}")
    );
    let effects = planner_tool(&mut fx, (PLANNER + 1, "mail"), "submit_epic", args.clone());
    assert_eq!(
        error(&effects),
        format!("this window is not the sub-planner of epic mail of run {RUN_ID}")
    );
    // The orchestrator's window is no planner either, and an unknown epic is refused.
    let effects = planner_tool(&mut fx, (ORCH, "mail"), "submit_epic", args.clone());
    assert!(error(&effects).starts_with("this window is not the sub-planner"));
    let effects = planner_tool(&mut fx, (PLANNER, "sms"), "submit_epic", args);
    assert_eq!(error(&effects), "unknown epic sms");
    assert!(fx.run().task("w1").is_none());
    assert_eq!(epic(&fx, "web").edits_rejected, 0);
}

#[test]
fn submit_epic_outside_the_area_is_rejected_with_the_area_error() {
    let mut fx = planning_mail(false);
    let effects = submit_epic(&mut fx, json!([task_in("t2", "web")]));
    assert_eq!(
        rejected_messages(&effects),
        vec!["crates/web/** is outside the area crates/mail/**".to_string()]
    );
    let e = epic(&fx, "mail");
    assert_eq!(e.phase, PlannerPhase::Planning);
    assert_eq!(e.edits_rejected, 1);
    assert_eq!(
        e.last_rejection.as_deref(),
        Some("crates/web/** is outside the area crates/mail/**")
    );
    assert!(fx.run().task("t2").is_none());
}

#[test]
fn rejections_count_and_fail_at_max_rejections() {
    let mut fx = planning_mail(false);
    fx.run_mut().limits.orch.planners.max_rejections = 2;
    let effects = submit_epic(&mut fx, json!([task_in("t2", "web")]));
    rejected_messages(&effects);
    assert!(
        !effects
            .iter()
            .any(|e| matches!(e, Effect::StopPlanner { .. })),
        "{effects:#?}"
    );
    assert_eq!(epic(&fx, "mail").phase, PlannerPhase::Planning);
    let effects = submit_epic(&mut fx, json!([task_in("t2", "web")]));
    rejected_messages(&effects);
    let reason = "the sub-planner's epic was rejected 2 times".to_string();
    assert!(
        effects.contains(&Effect::StopPlanner {
            window_id: PLANNER,
            reason: reason.clone(),
        }),
        "{effects:#?}"
    );
    let e = epic(&fx, "mail");
    assert_eq!(e.phase, PlannerPhase::Failed { reason });
    assert_eq!(e.edits_rejected, 2);
    // A failed planner's window submits nothing more.
    let effects = submit_epic(&mut fx, json!([task_in("t2", "mail")]));
    assert!(error(&effects).starts_with("this window is not the sub-planner"));
}

#[test]
fn replan_of_a_live_planner_is_refused() {
    let mut fx = planning_mail(false);
    let args = spawn_args("mail", &["crates/mail/**"], "Again");
    let effects = orch_tool(&mut fx, ORCH, "spawn_subplanner", args);
    assert_eq!(
        error(&effects),
        "epic mail is being planned by its sub-planner; wait for it to finish"
    );
    assert_eq!(epic(&fx, "mail").sessions.len(), 1);
}

/// Decision 21's re-plan: a fresh session (never the old one), with the new brief as
/// its request and the epic's current tasks in its prompt.
#[test]
fn replan_starts_a_fresh_session_with_the_current_tasks() {
    let mut fx = planning_mail(false);
    submit_epic(&mut fx, json!([task_in("t2", "mail")]));
    let args = spawn_args(
        "mail",
        &["crates/other/**"],
        "Add retries to the mail sender",
    );
    let (ok, value) = answer(&orch_tool(&mut fx, ORCH, "spawn_subplanner", args));
    assert!(ok, "{value}");
    assert_eq!(value["state"], "planning");
    let (_, kind) = fx.op("StartPlanner");
    let OpKind::StartPlanner { spec } = kind else {
        unreachable!()
    };
    assert_eq!((spec.epic.as_str(), spec.session), ("mail", 2));
    assert_eq!(spec.headless.run_ref.as_ref().unwrap().session, 2);
    assert!(
        spec.first_turn.starts_with(&format!(
            "[anthrex] Re-plan epic mail \"Epic mail\" of run {RUN_ID}."
        )),
        "{}",
        spec.first_turn
    );
    assert!(
        spec.first_turn
            .contains("The epic's current tasks:\n- t2 S queued Title t2\nWhat to plan:\nAdd retries to the mail sender"),
        "{}",
        spec.first_turn
    );
    let e = epic(&fx, "mail");
    assert_eq!(
        e.replans,
        vec!["Add retries to the mail sender".to_string()]
    );
    assert_eq!(e.request, "Add retries to the mail sender");
    // The epic keeps its area.
    assert_eq!(e.area, vec!["crates/mail/**".to_string()]);
    assert_eq!(e.sessions.len(), 2);
    assert_eq!(e.phase, PlannerPhase::Planning);
}

#[test]
fn failed_planner_keeps_accepted_tasks_and_wakes_the_orchestrator() {
    let mut fx = planning_mail(false);
    submit_epic(&mut fx, json!([task_in("t2", "mail")]));
    planner_ended(&mut fx, ("mail", 1), ScoutEnd::Reported);
    spawn(&mut fx, "mail");
    planner_started(&mut fx, PLANNER + 5);
    let reason = "the sub-planner ran longer than 2400 s";
    let failed = ScoutEnd::Failed {
        reason: reason.into(),
    };
    planner_ended(&mut fx, ("mail", 2), failed);
    let e = epic(&fx, "mail");
    assert_eq!(
        e.phase,
        PlannerPhase::Failed {
            reason: reason.into()
        }
    );
    assert_ne!(fx.task("t2").state, proto::TaskState::Cancelled);
    let notes = &fx.run().orch.orchestrator.as_ref().unwrap().notes;
    assert!(
        notes.contains(&format!("sub-planner mail failed: {reason}")),
        "{notes:?}"
    );
    // Both sessions' usage counts.
    assert_eq!(fx.run().orch.planner_usage.input, 20);
    // A stale session's end changes nothing.
    let failed = ScoutEnd::Failed {
        reason: "late".into(),
    };
    planner_ended(&mut fx, ("mail", 1), failed);
    assert!(matches!(
        &epic(&fx, "mail").phase,
        PlannerPhase::Failed { reason: r } if r == reason
    ));
}

#[test]
fn a_planner_that_cannot_start_fails() {
    let mut fx = launched(false);
    spawn(&mut fx, "mail");
    let (op, _) = fx.op("StartPlanner");
    fx.done(
        op,
        OpResult::Failed {
            message: "no such binary".into(),
        },
    );
    assert_eq!(
        epic(&fx, "mail").phase,
        PlannerPhase::Failed {
            reason: "the sub-planner could not start: no such binary".into()
        }
    );
    // The orchestrator may start a fresh one.
    assert_eq!(spawn(&mut fx, "mail")["state"], "planning");
}
