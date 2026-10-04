//! Milestone 9.6 task M9.6.7, fix round 1: a phase budget's halt and its resume
//! (ruling T7-1: `--rebaseline` is refused, and no resume leaves `halted_from` behind),
//! the halt as every other halt (m1), and a daemon restart's downtime at a revising
//! gate (ruling T7-2).

use proto::{ActionKind, DocGateAction, DocGateKind, RunState};
use serde_json::json;

use super::design_fixture::*;
use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, orch_tool};
use super::orch_restore::{restart, resume};
use crate::run::engine::EventKind;
use crate::run::engine::actions::{self, ActionNode};
use crate::run::engine::stages::Rebaseline;

const PLANNING_HALT: &str = "design flow: the planning phase passed its 60 min budget";

/// A design run in planning (its spec approved), halted by the planning phase's budget.
fn halted_in_planning() -> Fixture {
    let mut fx = at_spec_gate(false);
    act(&mut fx, DocGateKind::Spec, DocGateAction::Approve).unwrap();
    assert_eq!(fx.run().state, RunState::Planning);
    let late = fx.now + 60 * 60 + 1;
    fx.send(late, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Halted);
    fx
}

fn resume_rebaselined(fx: &mut Fixture) -> Vec<Result<String, String>> {
    let reply = fx.reply();
    replies(&fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some(Rebaseline::from((BASE.to_string(), HEAD.to_string()))),
    }))
}

fn halted_from(fx: &Fixture) -> Option<RunState> {
    fx.run().orch.design.as_ref().and_then(|d| d.halted_from)
}

/// Fix round 1, m1: the budget's halt is `merge::halt`'s, so its log line has the
/// `halted:` prefix and the orchestrator is told, as for every other halt (milestone 9
/// decision 39); it stays retryable.
#[test]
fn a_budget_halt_is_logged_and_told_as_every_halt() {
    let fx = halted_in_planning();
    assert_eq!(fx.run().halted_reason.as_deref(), Some(PLANNING_HALT));
    assert!(fx.run().halt_retryable);
    let logged = format!("halted: {PLANNING_HALT}");
    assert!(log_lines(&fx).contains(&logged), "{:?}", log_lines(&fx));
    let wake = format!("the run halted: {PLANNING_HALT}");
    assert!(notes(&fx).contains(&wake), "{:?}", notes(&fx));
}

/// Ruling T7-1: a run a phase budget halted refuses `run resume --rebaseline` exactly
/// (the action menu still offers its plain resume); the plain resume returns it to its
/// phase and clears `halted_from`, so a later retryable halt's plain resume runs it
/// again instead of sending it back to planning.
#[test]
fn a_budget_halt_refuses_rebaseline_and_a_later_halt_resumes_to_running() {
    let mut fx = halted_in_planning();
    let refusal =
        format!("run {RUN_ID} halted in its planning phase; resume it without --rebaseline");
    assert_eq!(resume_rebaselined(&mut fx), vec![Err(refusal)]);
    assert_eq!(
        fx.run().state,
        RunState::Halted,
        "a refusal changes nothing"
    );
    assert_eq!(halted_from(&fx), Some(RunState::Planning));
    let check = actions::check(fx.run(), &ActionNode::Run, &ActionKind::Resume);
    assert_eq!(check, Ok(()), "the menu's resume is the plain one");
    assert_eq!(
        replies(&resume(&mut fx)),
        vec![Ok(format!("run {RUN_ID} resumed"))]
    );
    assert_eq!(fx.run().state, RunState::Planning);
    assert_eq!(halted_from(&fx), None);
    // The plan, its gate and the user's approve: the run runs.
    let args = json!({"edits": [add_task("t1")], "submit": true});
    assert!(replies(&orch_tool(&mut fx, ORCH, "edit_plan", args))[0].is_ok());
    assert_eq!(
        replies(&fx.approve()),
        vec![Ok(format!("run {RUN_ID} approved"))]
    );
    assert_eq!(fx.run().state, RunState::Running);
    // A halt on refs the guard could not read (the record `complete::refs_verified`
    // leaves, set in place: this run has no merge to verify) is retried by a plain
    // resume, as in a run without the flow.
    let run = fx.run_mut();
    run.state = RunState::Halted;
    run.halt_retryable = true;
    run.halted_reason = Some("could not read the refs: git timed out".into());
    assert_eq!(
        replies(&resume(&mut fx)),
        vec![Ok(format!("run {RUN_ID} resumed"))]
    );
    assert_eq!(fx.run().state, RunState::Running);
}

/// Ruling T7-2: a daemon restart's downtime never counts toward a phase's budget. A
/// revising gate is not paused by the restore, so its clock is shifted by the downtime:
/// ten hours down, then the first tick does not halt it; the clock still runs after.
#[test]
fn a_revising_gate_across_a_long_restart_does_not_halt_on_its_first_tick() {
    let mut fx = at_spec_gate(false);
    let changes = DocGateAction::Changes {
        note: "Name the token store.".into(),
        review: false,
    };
    act(&mut fx, DocGateKind::Spec, changes).unwrap();
    let asked = fx.now;
    fx.now = asked + 10 * 3600;
    restart(&mut fx);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    fx.tick();
    assert_eq!(
        fx.run().state,
        RunState::AwaitingApproval,
        "{:?}",
        log_lines(&fx)
    );
    let back = fx.now;
    fx.send(back + 59 * 60, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    fx.send(back + 61 * 60, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Halted);
    let text = "design flow: the specifying phase passed its 60 min budget";
    assert_eq!(fx.run().halted_reason.as_deref(), Some(text));
}

/// Fix round 2: two restarts in a row, with no step between them, credit each downtime
/// once. The clock ran 40 minutes before the first stop (the orchestrator read its note
/// at minute 40, its last change); after the second restart, 21 more minutes pass the
/// hour.
#[test]
fn two_restarts_in_a_row_credit_each_downtime_once() {
    let mut fx = at_spec_gate(false);
    let changes = DocGateAction::Changes {
        note: "Name the token store.".into(),
        review: false,
    };
    act(&mut fx, DocGateKind::Spec, changes).unwrap();
    let asked = fx.now;
    fx.now = asked + 40 * 60;
    let seq = crate::run::engine::notes_seq(fx.run());
    fx.next(EventKind::Orch(crate::run::engine::OrchEvent::DigestRead {
        run_id: RUN_ID.into(),
        digest_revision: 0,
        notes_seq: seq,
    }));
    assert_eq!(fx.run().last_step_at, fx.now, "the run's last change");
    fx.now += 10 * 3600;
    restart(&mut fx);
    fx.now += 3600;
    restart(&mut fx);
    let back = fx.now;
    fx.send(back + 19 * 60, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    fx.send(back + 21 * 60, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Halted, "{:?}", log_lines(&fx));
}

/// Ruling T7-9: a run halted before its plan was approved and then cancelled is not
/// resumed into its phase: either resume is refused exactly (the menu's too), and the
/// cancel's reply says to discard it (review m2's halted, cancelled discard, which
/// waits for its sessions' ops as for any cancelled run).
#[test]
fn a_cancelled_run_halted_before_its_plan_is_discarded_not_resumed() {
    let mut fx = budget_halted();
    let reply = fx.reply();
    let effects = fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    let discard = format!("discard it with anthrex run discard {RUN_ID}");
    let cancelled = format!("run {RUN_ID} cancelled before its plan was approved; {discard}");
    assert_eq!(replies(&effects), vec![Ok(cancelled)]);
    let refusal = format!("run {RUN_ID} was cancelled before its plan was approved; {discard}");
    assert_eq!(replies(&resume(&mut fx)), vec![Err(refusal.clone())]);
    assert_eq!(resume_rebaselined(&mut fx), vec![Err(refusal.clone())]);
    assert_eq!(fx.run().state, RunState::Halted);
    let check = actions::check(fx.run(), &ActionNode::Run, &ActionKind::Resume);
    assert_eq!(check, Err(refusal));
}
