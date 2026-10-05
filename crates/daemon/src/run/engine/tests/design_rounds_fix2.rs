//! Milestone 9.6 task M9.6.15, fix round 2 (rulings T15-8 to T15-12): a dropped round's
//! reviews are kept and never renumbered; a round cancelled after its plan's approval
//! keeps its approval and its documents commit; a stage adopt waits for a round's
//! commit; an `off` round halts and resumes as 9.3's; and an `off` round's plan gate
//! is skipped with `--yes` only when the user started it.

use proto::{DocGateAction, DocGateKind, DocKind, RoundDesign, RoundOrigin, RunState};
use serde_json::{Value, json};

use super::delivery_open::host_ops_in;
use super::design_agents::{launches, started};
use super::design_fixture::*;
use super::design_review_fixture::{outcome, submit_findings, written};
use super::design_rounds_commit::{
    ROUND_DOCS, approved_round, assert_committed, pr_round_approved,
};
use super::design_rounds_fixture::*;
use super::dispatch::replies;
use super::fixture::*;
use super::goal_rounds_start::reply;
use super::kinds_cancel::cancel;
use super::kinds_integration::C1;
use super::orch::{ORCH, orch_tool};
use super::orch_restore::resume;
use crate::run::delivery::ops::HostOp;
use crate::run::engine::stages::Rebaseline;
use crate::run::engine::{EventKind, OpResult};

/// The spec reviewer's window in these tests.
const SPEC_REVIEWER: u32 = 913;

/// The orchestrator's amendment `text` sent to review: the review's number and the
/// files written.
fn drafted(fx: &mut Fixture, text: &str) -> (u32, Vec<String>) {
    let args = json!({"kind": "spec", "text": text, "ready": false, "amend": true});
    let effects = orch_tool(fx, ORCH, "submit_doc", args);
    let answer = outcome(&effects).unwrap();
    (answer["review"].as_u64().unwrap() as u32, written(&effects))
}

/// Ruling T15-9 (N1): round 2's spec review, then round 2 rejected at its spec gate;
/// round 3's review takes the next number, writes its own draft file, and its reviewer
/// reads that draft. Round 2's review is kept, marked dropped.
#[test]
fn a_dropped_rounds_review_number_is_never_reused() {
    let mut fx = design_complete();
    iterate_with(&mut fx, None).unwrap();
    let (k2, files2) = drafted(&mut fx, AMENDMENT);
    let label = launches(&fx).last().unwrap().1.kind.label();
    assert_eq!(label, format!("spec-r{k2}"));
    started(&mut fx, &label, SPEC_REVIEWER);
    outcome(&submit_findings(&mut fx, SPEC_REVIEWER, json!([]))).unwrap();
    submit_amendment(&mut fx, AMENDMENT).unwrap();
    act(&mut fx, DocGateKind::Spec, DocGateAction::Reject).unwrap();
    assert_eq!(fx.run().state, RunState::Complete);
    iterate_with(&mut fx, None).unwrap();
    let other = AMENDMENT.replace("in the app too", "on every device");
    let (k3, files3) = drafted(&mut fx, &other);
    assert_eq!(k3, k2 + 1, "the next number");
    assert_eq!(files3, [format!("spec-draft-r{k3}.md")]);
    assert_ne!(files3, files2);
    let label = launches(&fx).last().unwrap().1.kind.label();
    assert_eq!(label, format!("spec-r{k3}"));
    let design = fx.run().orch.design.as_ref().unwrap();
    let newest = design.versions.last().unwrap();
    assert_eq!(design.draft(DocKind::Spec, k3), Some(newest));
    let kept: Vec<(u32, bool)> = (design.reviews.iter())
        .filter(|r| r.doc == DocKind::Spec)
        .map(|r| (r.n, r.dropped))
        .collect();
    assert_eq!(kept, [(k2, true), (k3, false)]);
}

/// The design state a cancel must leave alone: requirements, approval, commit state.
fn design_of(fx: &Fixture) -> (usize, Option<u32>, bool, u32, bool) {
    let d = fx.run().orch.design.as_ref().unwrap();
    let dropped = d.reviews.iter().any(|r| r.dropped);
    (
        d.requirements.len(),
        d.approved_spec,
        d.commit_due,
        d.committed_round,
        dropped,
    )
}

/// Ruling T15-10 (N2): round 2 cancelled after its plan's approval, its documents commit
/// in flight: the approval and the commit stay; the commit's reply is recorded.
#[test]
fn a_round_cancelled_with_its_commit_in_flight_keeps_its_approval() {
    let (mut fx, op, _) = approved_round(design_complete());
    let before = design_of(&fx);
    assert_eq!(before, (3, Some(2), true, 1, false));
    assert!(reply(&cancel(&mut fx)).is_ok());
    assert_eq!(design_of(&fx), before);
    let result = OpResult::DocsCommitted {
        head: ROUND_DOCS.into(),
        spec: "docs/anthrex/specs/1970-01-01-password-reset.md".into(),
    };
    fx.done(op, result);
    assert_eq!(design_of(&fx), (3, Some(2), false, 2, false));
    let run = fx.run();
    assert_eq!(run.stage(2).map(|s| s.head.as_str()), Some(ROUND_DOCS));
    assert_eq!(run.run_head, ROUND_DOCS, "the commit's reply is recorded");
}

/// Ruling T15-10 (N2): round 2 cancelled after its commit landed: nothing of the design
/// state goes back.
#[test]
fn a_round_cancelled_after_its_commit_keeps_its_approval() {
    let (mut fx, op, _) = approved_round(design_complete());
    assert_committed(&mut fx, op, C1);
    let before = design_of(&fx);
    assert!(reply(&cancel(&mut fx)).is_ok());
    assert_eq!(design_of(&fx), before);
    let design = fx.run().orch.design.as_ref().unwrap();
    assert!(design.round.is_some(), "the round's record stays");
}

/// Ruling T15-11 (N3): a stage adopt due while round 2's commit is in flight waits for
/// the commit's reply, then fetches.
#[test]
fn a_stage_adopt_waits_for_the_round_commit() {
    let (mut fx, op, _) = pr_round_approved();
    let stage1 = fx.run().stage(1).unwrap().head.clone();
    fx.run_mut().delivery.stages[0].remote_head = Some("abab".repeat(10));
    let adopts = |effects: &[crate::run::engine::Effect]| {
        (host_ops_in(effects).into_iter())
            .filter(|op| matches!(op, HostOp::Fetch { adopt: Some(_), .. }))
            .count()
    };
    let effects = fx.tick();
    assert_eq!(adopts(&effects), 0, "{effects:?}");
    let mut effects = assert_committed(&mut fx, op, &stage1);
    effects.extend(fx.tick());
    assert_eq!(adopts(&effects), 1, "{effects:?}");
    assert_eq!(fx.run().run_head, ROUND_DOCS);
}

/// `fx` halted, not retryably, as a delivery result can halt it.
fn halted(fx: &mut Fixture) -> String {
    let (now, reason) = (fx.now, "remote stage branch moved".to_string());
    crate::run::engine::merge::halt(fx.run_mut(), reason.clone(), now);
    reason
}

fn rebaselined(fx: &mut Fixture) -> Vec<Result<String, String>> {
    let reply = fx.reply();
    replies(&fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some(Rebaseline::from((BASE.to_string(), C1.to_string()))),
    }))
}

/// Ruling T15-12: an `off` round's halt is 9.3's: no phase is kept, a plain resume of a
/// halt that is not retryable is refused with 9.3's text, and `--rebaseline` resumes it.
#[test]
fn an_off_rounds_halt_is_9_3s() {
    let mut fx = design_complete();
    iterate_with(&mut fx, Some(RoundDesign::Off)).unwrap();
    assert_eq!(fx.run().state, RunState::Planning);
    let reason = halted(&mut fx);
    assert_eq!(fx.run().orch.design.as_ref().unwrap().halted_from, None);
    let refusal =
        format!("run {RUN_ID} is halted: {reason}; check the refs, then resume with --rebaseline");
    assert_eq!(replies(&resume(&mut fx)), vec![Err(refusal)]);
    assert!(rebaselined(&mut fx)[0].is_ok());
}

/// Ruling T15-12: a stale `halted_from` (an off round's, from before the fix) is
/// cleared by the `--rebaseline` resume that the 9.3 gate takes.
#[test]
fn a_rebaseline_clears_a_stale_halted_from() {
    let mut fx = design_complete();
    iterate_with(&mut fx, Some(RoundDesign::Off)).unwrap();
    round_submit(&mut fx, json!([off_task()])).unwrap();
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    halted(&mut fx);
    let design = fx.run_mut().orch.design.as_mut().unwrap();
    design.halted_from = Some(RunState::AwaitingApproval);
    assert!(rebaselined(&mut fx)[0].is_ok());
    assert_eq!(fx.run().orch.design.as_ref().unwrap().halted_from, None);
}

/// A 9.3 task of round 2's first stage.
fn off_task() -> Value {
    let mut t2 = super::orch::add("t2", "mail");
    t2["task"]["stage"] = json!(2);
    t2
}

/// Ruling T15-8 (m3): with `--yes`, a user-started `off` round's plan skips its gate as
/// in 9.3; one the orchestrator started stops there.
#[test]
fn an_off_rounds_gate_is_skipped_with_yes_only_when_the_user_started_it() {
    for (origin, gate) in [
        (RoundOrigin::User, false),
        (RoundOrigin::Orchestrator, true),
    ] {
        let mut fx = design_complete_with(true);
        iterate_with(&mut fx, Some(RoundDesign::Off)).unwrap();
        fx.run_mut().rounds.last_mut().unwrap().origin = origin;
        let answer = round_submit(&mut fx, json!([off_task()])).unwrap();
        assert_eq!(answer["awaiting_approval"], gate, "{origin:?}: {answer}");
        let state = fx.run().state;
        assert_eq!(
            state == RunState::AwaitingApproval,
            gate,
            "{origin:?}: {state:?}"
        );
    }
}
