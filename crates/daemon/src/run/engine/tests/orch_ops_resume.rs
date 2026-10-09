//! Milestone 9.9 (OFA §4.2, decision 6): the orchestrator's resume_run and accept_red.

use proto::RunState;
use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use super::full::verify_ok;
use super::merge::commit;
use super::orch::{edit_plan, error};

fn with_orchestrator(fx: &mut Fixture) {
    let orch = super::orch::launched(false).run().orch.orchestrator.clone();
    fx.run_mut().orch.orchestrator = orch;
}

fn op(fx: &mut Fixture, edit: serde_json::Value) -> Vec<crate::run::engine::Effect> {
    edit_plan(fx, json!({"edits": [edit]}))
}

/// A tiered run whose one task merged at `commit(1)`, its tier 3 red on that head with
/// no bisect (`full.rs`'s `red_at_completion`: no `single_test`, so no single culprit).
fn red() -> Fixture {
    let mut fx = super::full::red_at_completion();
    assert_eq!(
        fx.run().stage(1).unwrap().full.red_at.as_deref(),
        Some(commit(1).as_str())
    );
    with_orchestrator(&mut fx);
    // With an orchestrator a run completes only once its plan is submitted (decision 38,
    // `integration::may_complete`); the launched fixture's has not been.
    fx.run_mut()
        .orch
        .orchestrator
        .as_mut()
        .unwrap()
        .plan_submitted = true;
    fx
}

#[test]
fn accept_red_lets_the_run_complete_and_is_recorded() {
    let mut fx = red();
    assert_ne!(
        fx.run().state,
        RunState::Complete,
        "a red head holds completion"
    );
    let effects = op(
        &mut fx,
        json!({"op": "accept_red", "stage": 1, "reason": "red on main too"}),
    );
    let sha7 = &commit(1)[..7];
    assert_eq!(
        replies(&effects)[0].clone().unwrap(),
        format!("stage 1's red tier 3 on {sha7} is accepted; completion and delivery go on")
    );
    fx.tick();
    if !super::merge::pending(&fx, "VerifyRefs", None).is_empty() {
        verify_ok(&mut fx);
    }
    assert_eq!(fx.run().state, RunState::Complete);
    let info = crate::run::snapshot::snapshot(&fx.state, fx.now);
    let stage = &info.runs[0].stages[0];
    assert!(stage.full.accepted);
    assert_eq!(
        stage.full.state,
        proto::FullState::Red,
        "still red, accepted"
    );
    assert_eq!(fx.run().orch.handled[0].target, "stage 1");
}

#[test]
fn accept_red_is_refused_without_a_red_head_and_twice() {
    let mut fx = red();
    let effects = op(
        &mut fx,
        json!({"op": "accept_red", "stage": 2, "reason": "r"}),
    );
    assert_eq!(error(&effects), format!("run {RUN_ID} has no stage 2"));
    assert!(
        replies(&op(
            &mut fx,
            json!({"op": "accept_red", "stage": 1, "reason": "r"})
        ))[0]
            .is_ok()
    );
    let effects = op(
        &mut fx,
        json!({"op": "accept_red", "stage": 1, "reason": "r"}),
    );
    assert_eq!(
        error(&effects),
        format!("stage 1's red on {} is already accepted", &commit(1)[..7])
    );
}

#[test]
fn a_halted_run_takes_a_lone_resume_run_and_nothing_else() {
    let mut fx = super::actions_fixtures::halted_retryable();
    with_orchestrator(&mut fx);
    // Another edit on a halted run: today's gate.
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [{"op": "cancel_task", "task_id": "t1"}]}),
    );
    assert_eq!(error(&effects), format!("run {RUN_ID} is halted"));
    let effects = op(
        &mut fx,
        json!({"op": "resume_run", "reason": "the refs read again"}),
    );
    assert_eq!(
        replies(&effects)[0].clone().unwrap(),
        format!("run {RUN_ID} resumed")
    );
    assert_ne!(fx.run().state, RunState::Halted);
    assert!(
        fx.run()
            .log
            .iter()
            .any(|e| e.text == "orchestrator: resumed the run — the refs read again")
    );
}

#[test]
fn a_halt_that_needs_a_rebaseline_is_the_users() {
    let mut fx = super::actions_fixtures::halted();
    with_orchestrator(&mut fx);
    let effects = op(&mut fx, json!({"op": "resume_run", "reason": "try again"}));
    let text = error(&effects);
    assert!(
        text.ends_with("check the refs, then resume with --rebaseline"),
        "{text}"
    );
    assert_eq!(fx.run().state, RunState::Halted);
}

#[test]
fn resume_run_with_a_stage_releases_only_a_held_stage() {
    let mut fx = super::actions_rules::running();
    with_orchestrator(&mut fx);
    let effects = op(
        &mut fx,
        json!({"op": "resume_run", "reason": "r", "stage": 1}),
    );
    assert_eq!(
        error(&effects),
        "stage 1 is not held; resume_run with a stage releases a held stage only"
    );
    super::actions_twins::hold_stage(fx.run_mut());
    let effects = op(
        &mut fx,
        json!({"op": "resume_run", "reason": "the executor is back", "stage": 1}),
    );
    assert_eq!(
        replies(&effects)[0].clone().unwrap(),
        format!("run {RUN_ID}: stage 1 released")
    );
    assert_eq!(fx.run().stage(1).unwrap().full.infra, None);
}

#[test]
fn a_paused_run_refuses_every_op() {
    let mut fx = super::actions_rules::running();
    with_orchestrator(&mut fx);
    assert!(
        replies(&super::dispatch::edit(
            &mut fx,
            vec![proto::PlanEdit::Pause]
        ))[0]
            .is_ok()
    );
    let effects = op(&mut fx, json!({"op": "resume_run", "reason": "r"}));
    assert_eq!(
        error(&effects),
        format!("run {RUN_ID} is paused; the user must resume it")
    );
}

fn call(tool: &str, args: serde_json::Value) -> proto::ToolCall {
    serde_json::from_value(json!({
        "run_id": RUN_ID, "task_id": null, "role": "orchestrator", "window_id": 1,
        "tool": tool, "args": args,
    }))
    .unwrap()
}

#[test]
fn the_halted_gate_admits_a_lone_resume_run_and_ask_user_only() {
    use crate::run::engine::orch_ops::admitted;
    let resume = json!({"edits": [{"op": "resume_run", "reason": "r"}]});
    assert!(admitted(
        RunState::Halted,
        &call("edit_plan", resume.clone())
    ));
    assert!(!admitted(RunState::Paused, &call("edit_plan", resume)));
    let two = json!({"edits": [{"op": "resume_run", "reason": "r"}, {"op": "pause"}]});
    assert!(!admitted(RunState::Halted, &call("edit_plan", two)));
    let extra = json!({"edits": [{"op": "resume_run", "reason": "r"}], "summary": "s"});
    assert!(!admitted(RunState::Halted, &call("edit_plan", extra)));
    let other = json!({"edits": [{"op": "retry", "task_id": "t1", "reason": "r"}]});
    assert!(!admitted(RunState::Halted, &call("edit_plan", other)));
    assert!(admitted(RunState::Halted, &call("ask_user", json!({}))));
    assert!(admitted(RunState::Paused, &call("ask_user", json!({}))));
    assert!(!admitted(RunState::Halted, &call("submit", json!({}))));
}
