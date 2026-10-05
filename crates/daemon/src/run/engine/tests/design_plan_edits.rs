//! Milestone 9.6 task M9.6.11: changes to a design run's plan at its gate, and the plan
//! submit's wait for the approved spec. A user's edit at the open gate is the next
//! version (ruling T7-7), a removal that uncovers a requirement blocks approval until
//! it is covered again (DF §5.1), an engine change is a version of its own (the task's
//! addendum), and a sub-planner at a revising gate leaves no stale gate (ruling T7-4's
//! carry). A plan submit waits for the approved spec's read-back (ruling T10-2), which
//! is retried on resume and halts on a second failure (ruling T10-3).

use proto::{ActionKind, DocAuthor, DocGateAction, DocGateKind, DocKind, PlanEdit, RunState, Size};
use serde_json::{Value, json};

use super::design_agents::launches;
use super::design_fixture::*;
use super::design_plan_fixture::*;
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use super::planners::{PLANNER, planner_started, submit_epic};
use crate::run::engine::actions::{self, ActionNode};
use crate::run::engine::{DocChecked, Effect, EventKind};

fn user_edit(fx: &mut Fixture, edit_json: Value) -> Vec<Effect> {
    let edit_value: PlanEdit = serde_json::from_value(edit_json).unwrap();
    edit(fx, vec![edit_value])
}

fn version(fx: &Fixture, n: u32) -> (DocAuthor, String) {
    let design = fx.run().orch.design.as_ref().unwrap();
    let v = design.find(DocKind::Plan, Some(n)).unwrap();
    (v.author.clone(), v.reason.clone())
}

/// A design run at its plan gate, v1, where `t1` covers R1 and `t2` covers R2.
fn split_plan_gate() -> Fixture {
    let mut fx = planning();
    let edits = json!([covering("t1", &["R1"]), covering("t2", &["R2"])]);
    plan_submit(&mut fx, edits, Value::Null).unwrap();
    plan_reviewed(&mut fx, json!([]));
    plan_submit(&mut fx, json!([]), Value::Null).unwrap();
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 1, None)));
    fx
}

/// DF §5.1: removing a task at the gate stores the next version and re-runs the
/// coverage check; approval is refused, by every path, while a requirement is
/// uncovered, and taken once it is covered again.
#[test]
fn removing_a_task_at_the_gate_rechecks_coverage_and_blocks_approve() {
    let mut fx = split_plan_gate();
    let effects = user_edit(&mut fx, json!({"op": "cancel_task", "task_id": "t2"}));
    let reply = replies(&effects).remove(0).unwrap();
    assert!(
        reply.ends_with("; the plan is now v2; approve waits: R2 are covered by no task"),
        "{reply}"
    );
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 2, None)));
    assert_eq!(version(&fx, 2), (DocAuthor::User, "edited by you".into()));
    let uncovered = "R2 are covered by no task".to_string();
    assert_eq!(replies(&fx.approve()), vec![Err(uncovered.clone())]);
    let check = actions::check(fx.run(), &ActionNode::Run, &ActionKind::Approve);
    assert_eq!(check, Err(uncovered.clone()));
    let gate_approve = act(&mut fx, DocGateKind::Plan, DocGateAction::APPROVE);
    assert_eq!(gate_approve, Err(uncovered));
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    user_edit(&mut fx, covering("t3", &["R2"]));
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 3, None)));
    assert_eq!(
        replies(&fx.approve()),
        vec![Ok(format!("run {RUN_ID} approved"))]
    );
}

/// Ruling T7-7: a user's edit that changes the plan at the open gate is v(n+1),
/// authored by the user; the gate stays open, and approval approves that version.
#[test]
fn approve_after_a_user_edit_approves_the_new_version() {
    let mut fx = at_plan_gate(false);
    user_edit(
        &mut fx,
        json!({"op": "amend_task", "task_id": "t1", "size": "M"}),
    );
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 2, None)));
    assert_eq!(version(&fx, 2), (DocAuthor::User, "edited by you".into()));
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    let reply = fx.approve();
    assert_eq!(replies(&reply), vec![Ok(format!("run {RUN_ID} approved"))]);
    assert!(notes(&fx).contains(&"the user approved the plan v2".to_string()));
}

/// The addendum's engine updates: a change the engine makes at the open gate (a size
/// or route a decider's answer set) is the next version, authored by the engine, and
/// the gate stays open; no cap refuses it.
#[test]
fn an_engine_change_at_the_open_gate_is_the_next_version() {
    let mut fx = at_plan_gate(false);
    for n in 2..=6 {
        let changes = DocGateAction::Changes {
            note: "again".into(),
            review: false,
        };
        act(&mut fx, DocGateKind::Plan, changes).unwrap();
        let args = json!({"edits": [add_task(&format!("c{n}"))], "submit": true});
        assert!(replies(&orch_tool(&mut fx, ORCH, "edit_plan", args))[0].is_ok());
    }
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 6, None)));
    let t1 = (fx.run_mut().tasks.iter_mut())
        .find(|t| t.id() == "t1")
        .unwrap();
    t1.size = Size::M;
    let now = fx.now + 1;
    fx.send(now, EventKind::Tick);
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 7, None)));
    let reason = "updated by anthrex: sizes and routes".to_string();
    assert_eq!(version(&fx, 7), (DocAuthor::Engine, reason));
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    fx.send(now + 1, EventKind::Tick);
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 7, None)));
}

/// Ruling T7-4's carry: a sub-planner started at a revising plan gate returns the run to
/// planning and clears the stale gate; the next passing submit opens v2 with the
/// user's note, and the plan review is not repeated.
#[test]
fn a_sub_planner_at_a_revising_gate_clears_it_and_the_review_is_not_repeated() {
    let mut fx = at_plan_gate(false);
    let changes = DocGateAction::Changes {
        note: "Add mail.".into(),
        review: false,
    };
    act(&mut fx, DocGateKind::Plan, changes).unwrap();
    spawn_covering(&mut fx, "mail", &["R2"]);
    assert_eq!(fx.run().state, RunState::Planning);
    assert_eq!(gate(&fx), None);
    planner_started(&mut fx, PLANNER);
    assert!(replies(&submit_epic(&mut fx, json!([add_task("mail")])))[0].is_ok());
    let reply = plan_submit(&mut fx, json!([]), Value::Null).unwrap();
    assert!(reply.get("awaiting_review").is_none(), "{reply}");
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 2, None)));
    let reason = "revised: Add mail.".to_string();
    assert_eq!(version(&fx, 2), (DocAuthor::Orchestrator, reason));
    let reviews = (launches(&fx).iter())
        .filter(|(_, s)| s.kind.label().starts_with("plan-r"))
        .count();
    assert_eq!(reviews, 1);
}

/// Decision 18: the user's `run edit --submit` passes the same checks, and opens the
/// gate at once (the plan review is the orchestrator's).
#[test]
fn a_users_submit_passes_the_same_checks() {
    let mut fx = planning();
    let submit = |fx: &mut Fixture, edits: Vec<PlanEdit>| {
        let reply = fx.reply();
        let effects = fx.next(EventKind::Edit {
            reply,
            run_id: RUN_ID.into(),
            edits,
            scope: crate::run::validate::EditScope::Run,
            refusals: Vec::new(),
            submit: true,
        });
        replies(&effects).remove(0)
    };
    let t1: PlanEdit = serde_json::from_value(covering("t1", &["R1"])).unwrap();
    let refused = submit(&mut fx, vec![t1]);
    assert_eq!(refused, Err("R2 are covered by no task".to_string()));
    let t1: PlanEdit = serde_json::from_value(covering("t1", &["R1", "R2"])).unwrap();
    submit(&mut fx, vec![t1]).unwrap();
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 1, None)));
    assert_eq!(version(&fx, 1), (DocAuthor::User, "submitted".into()));
    // Ruling T11-3 (m2): the user's v1 never had the plan review, and says so.
    let design = fx.run().orch.design.as_ref().unwrap();
    let v1 = design.find(DocKind::Plan, Some(1)).unwrap();
    assert_eq!(
        v1.not_reviewed.as_deref(),
        Some("you submitted it yourself")
    );
}

const PENDING: &str =
    "the approved spec is still being read back; submit the plan again in a moment";

/// Ruling T10-2: until the approved spec's text is read back, a plan submit is refused
/// (its requirements are not stored yet); once it is, the submit is taken.
#[test]
fn a_plan_submit_waits_for_the_approved_specs_read_back() {
    let mut fx = at_spec_gate(false);
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    let edits = json!([covering("t1", &["R1", "R2"])]);
    let refused = plan_submit(&mut fx, edits.clone(), Value::Null);
    assert_eq!(refused.unwrap_err(), PENDING);
    read_back(&mut fx, 1, SPEC);
    let reply = plan_submit(&mut fx, edits, Value::Null).unwrap();
    assert_eq!(reply["awaiting_review"], true);
}

const UNREAD: &str = "design flow: the approved spec could not be read back: its file is missing";

fn read_failed(fx: &mut Fixture) -> Vec<Effect> {
    let checked = vec![DocChecked {
        kind: DocKind::Spec,
        n: 1,
        read: Err("its file is missing".into()),
    }];
    fx.next(EventKind::DesignChecked {
        run_id: RUN_ID.into(),
        checked,
    })
}

fn resume(fx: &mut Fixture) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    })
}

fn asks_read_back(effects: &[Effect]) -> bool {
    effects.iter().any(|e| matches!(e, Effect::ReadBack { .. }))
}

/// Ruling T10-3: a failed read-back of the approved spec while the run is paused (a
/// restore) is logged and retried when the run resumes; a second failure halts the run
/// with the exact text, and its resume retries the read once more.
#[test]
fn a_failed_read_back_is_retried_on_resume_and_a_second_failure_halts() {
    let mut fx = at_spec_gate(false);
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    let run = fx.run_mut();
    run.state = RunState::Paused;
    run.paused_from = Some(RunState::Planning);
    read_failed(&mut fx);
    assert_eq!(fx.run().state, RunState::Paused);
    let failed: Vec<String> = (log_lines(&fx).into_iter())
        .filter(|l| l.contains("could not be read back"))
        .collect();
    assert_eq!(failed, [UNREAD]);
    assert!(asks_read_back(&resume(&mut fx)));
    assert_eq!(fx.run().state, RunState::Planning);
    read_failed(&mut fx);
    assert_eq!(fx.run().state, RunState::Halted);
    assert_eq!(fx.run().halted_reason.as_deref(), Some(UNREAD));
    let effects = resume(&mut fx);
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    assert!(asks_read_back(&effects));
    assert_eq!(fx.run().state, RunState::Planning);
    read_back(&mut fx, 1, SPEC);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert_eq!(design.requirements.len(), 2);
    let reply = plan_submit(&mut fx, json!([covering("t1", &["R1", "R2"])]), Value::Null);
    assert_eq!(reply.unwrap()["awaiting_review"], true);
}

/// Ruling T11-2: a run the unread spec halted, restored, whose read-back fails again,
/// stays halted as it was: the phase it left, its reason and its retry are kept, and the
/// failure is only logged. Its `run resume` then really resumes, and reads the spec
/// again.
#[test]
fn a_second_unread_halt_after_a_restore_keeps_the_halt_and_resumes() {
    let mut fx = at_spec_gate(false);
    act(&mut fx, DocGateKind::Spec, DocGateAction::APPROVE).unwrap();
    super::control_restore::restart(&mut fx, Vec::new());
    assert_eq!(fx.run().state, RunState::Paused);
    read_failed(&mut fx);
    resume(&mut fx);
    read_failed(&mut fx);
    assert_eq!(fx.run().state, RunState::Halted);
    let halted_from = |fx: &Fixture| fx.run().orch.design.as_ref().unwrap().halted_from;
    assert_eq!(halted_from(&fx), Some(RunState::Planning));
    let woken = notes(&fx).len();
    super::control_restore::restart(&mut fx, Vec::new());
    read_failed(&mut fx);
    assert_eq!(fx.run().state, RunState::Halted);
    assert_eq!(halted_from(&fx), Some(RunState::Planning));
    assert_eq!(fx.run().halted_reason.as_deref(), Some(UNREAD));
    assert!(fx.run().halt_retryable);
    assert_eq!(notes(&fx).len(), woken, "no second halt note");
    assert_eq!(log_lines(&fx).last().map(String::as_str), Some(UNREAD));
    let effects = resume(&mut fx);
    assert_eq!(replies(&effects), vec![Ok(format!("run {RUN_ID} resumed"))]);
    assert!(asks_read_back(&effects));
    assert_eq!(fx.run().state, RunState::Planning);
    read_back(&mut fx, 1, SPEC);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert_eq!(design.requirements.len(), 2);
}

/// Ruling T11-3 (m1): the open gate's engine pass renders `plan.md` again only when a
/// task's size, route or test mode changed; a change of anything else outside the
/// user's edits makes no version.
#[test]
fn only_a_size_route_or_test_mode_change_makes_an_engine_version() {
    let mut fx = at_plan_gate(false);
    let t1 = (fx.run_mut().tasks.iter_mut())
        .find(|t| t.id() == "t1")
        .unwrap();
    t1.spec.brief.push_str("\nmore");
    let now = fx.now + 1;
    fx.send(now, EventKind::Tick);
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 1, None)));
    let t1 = (fx.run_mut().tasks.iter_mut())
        .find(|t| t.id() == "t1")
        .unwrap();
    t1.size = Size::M;
    fx.send(now + 1, EventKind::Tick);
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 2, None)));
    let reason = "updated by anthrex: sizes and routes".to_string();
    assert_eq!(version(&fx, 2), (DocAuthor::Engine, reason));
}

/// Ruling T11-3 (m5): a user's edit whose `plan.md` has the stored version's SHA-256
/// stores nothing, also when a restore has not read the gate's text back yet.
#[test]
fn a_user_edit_that_leaves_plan_md_unchanged_stores_nothing() {
    let mut fx = at_plan_gate(false);
    super::control_restore::restart(&mut fx, Vec::new());
    // The restore's read-back of the texts has not landed (the fixture's run is a
    // clone, so its cache is cleared by hand, as a real restore starts without one).
    let design = fx.run_mut().orch.design.as_mut().unwrap();
    design.texts.clear();
    assert!(
        design.text_of(DocKind::Plan).is_none(),
        "nothing read back yet"
    );
    let amend = json!({"op": "amend_task", "task_id": "t1", "acceptance": ["Other"]});
    let reply = replies(&user_edit(&mut fx, amend)).remove(0).unwrap();
    assert!(!reply.contains("the plan is now"), "{reply}");
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 1, None)));
    user_edit(
        &mut fx,
        json!({"op": "amend_task", "task_id": "t1", "size": "M"}),
    );
    assert_eq!(gate(&fx), Some((DocGateKind::Plan, 2, None)));
}
