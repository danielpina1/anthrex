//! Ruling WB-A-W1 (the final fix wave's FW-4): while a run is halted or paused in a
//! design phase, a live design agent's one write (a brainstormer's draft, a document
//! reviewer's findings) is held, as ruling T8-1 holds drafts, and applied in order on
//! resume; a discard drops it. The agent is not failed, and its one relaunch is not
//! spent.

use proto::RunState;
use serde_json::Value;

use super::design_agents::{
    CLAUDE, CODEX, DRAFT, answered, ended, launches, started, states, submit_draft,
};
use super::design_fixture::*;
use super::design_review_fixture::*;
use super::dispatch::replies;
use super::fixture::*;
use super::orch_restore::resume;
use crate::run::design::state::DesignAgentState;
use crate::run::engine::{EventKind, OpKind, OpResult, ScoutEnd};
use crate::run::model::PendingOp;

const IN: &str = "spec review 1 is in: 3 findings (1 blocking)";

/// A spec draft sent to review, its reviewer started in [`REVIEWER`]: its label.
fn reviewing(fx: &mut Fixture) -> String {
    outcome(&submit_spec(fx, false, Value::Null)).unwrap();
    let label = launches(fx).last().unwrap().1.kind.label();
    started(fx, &label, REVIEWER);
    label
}

/// The specifying budget passes: the run halts.
fn budget_halt(fx: &mut Fixture) {
    let late = fx.now + u64::from(fx.run().limits.orch.design.phase_minutes) * 60 + 1;
    fx.send(late, EventKind::Tick);
    assert_eq!(fx.run().state, RunState::Halted);
}

fn reviewer_state(fx: &Fixture) -> DesignAgentState {
    design(fx).reviewer.as_ref().unwrap().state.clone()
}

/// The ruling's test: a budget halt during a spec review, the findings submitted while
/// halted, the (Codex) reviewer's session ended; the resume gives the review's findings.
#[test]
fn a_budget_halt_during_a_spec_review_holds_its_findings() {
    let mut fx = specifying();
    let label = reviewing(&mut fx);
    budget_halt(&mut fx);
    let answer = outcome(&submit_findings(&mut fx, REVIEWER, three_findings()));
    assert_eq!(
        answer,
        Ok(serde_json::json!({"accepted": true, "findings": 3}))
    );
    let review = design(&fx).reviews.last().unwrap().clone();
    assert!(review.findings.is_empty(), "applied while halted");
    assert!(!log_lines(&fx).contains(&IN.to_string()));
    let starts = launches(&fx).len();
    reviewer_ended(&mut fx, &label, 1, ScoutEnd::Reported);
    let review = design(&fx).reviews.last().unwrap().clone();
    assert_eq!(review.failed, None, "the reviewer was failed");
    assert_eq!(launches(&fx).len(), starts, "its relaunch was spent");
    assert!(!design(&fx).reviewer.as_ref().unwrap().unsubmitted);
    resume(&mut fx);
    assert_eq!(fx.run().state, RunState::Specifying);
    let review = design(&fx).reviews.last().unwrap().clone();
    assert_eq!(review.findings.len(), 3);
    assert_eq!(reviewer_state(&fx), DesignAgentState::Done);
    assert!(
        log_lines(&fx).contains(&IN.to_string()),
        "{:#?}",
        log_lines(&fx)
    );
    let note = "spec review 1 is in: 3 findings (1 blocking); answer each in your next submit_doc";
    assert!(notes(&fx).contains(&note.to_string()), "{:?}", notes(&fx));
}

/// The same while paused: held, then applied when the run resumes.
#[test]
fn a_paused_spec_review_holds_its_findings() {
    let mut fx = specifying();
    reviewing(&mut fx);
    let run = fx.run_mut();
    run.paused_from = Some(RunState::Specifying);
    run.state = RunState::Paused;
    outcome(&submit_findings(&mut fx, REVIEWER, three_findings())).unwrap();
    assert!(design(&fx).reviews.last().unwrap().findings.is_empty());
    resume(&mut fx);
    assert_eq!(fx.run().state, RunState::Specifying);
    assert_eq!(design(&fx).reviews.last().unwrap().findings.len(), 3);
    assert!(log_lines(&fx).contains(&IN.to_string()));
}

/// A discard (after the cancel a halted design run needs) drops a held submit:
/// nothing of it is applied.
#[test]
fn a_discard_drops_a_held_submit() {
    let mut fx = specifying();
    reviewing(&mut fx);
    budget_halt(&mut fx);
    outcome(&submit_findings(&mut fx, REVIEWER, three_findings())).unwrap();
    assert!(super::goal_rounds_start::reply(&super::kinds_cancel::cancel(&mut fx)).is_ok());
    assert_eq!(design(&fx).held_findings, None, "the cancel dropped it");
    let reply = fx.reply();
    // The brainstormers' starts the fixture stubbed are never answered.
    let stubbed = |p: &PendingOp| matches!(p.kind, OpKind::StartDesignAgent { .. });
    fx.run_mut().pending_ops.retain(|_, p| !stubbed(p));
    let effects = fx.next(EventKind::Finish {
        reply,
        run_id: RUN_ID.into(),
        action: proto::FinishAction::Discard,
    });
    assert!(replies(&effects).iter().all(|r| r.is_ok()), "{effects:?}");
    let (op, _) = fx.op("Discard");
    fx.done(
        op,
        OpResult::Finished {
            outcome: "discarded".into(),
            kept_branches: Vec::new(),
        },
    );
    assert_eq!(fx.run().state, RunState::Discarded);
    assert!(design(&fx).reviews.last().unwrap().findings.is_empty());
    assert!(!log_lines(&fx).contains(&IN.to_string()));
}

/// Review A's W-1 and the T8 fix-2 carry (m1): a pack-unreadable halt holds the other
/// brainstormer's draft instead of refusing it; after the resume both drafts come in.
#[test]
fn a_pack_halt_holds_the_other_brainstormers_draft() {
    let mut fx = design_launched(false);
    start_brainstorm(&mut fx);
    started(&mut fx, "claude", CLAUDE);
    let (op, _) = launches(&fx)[1].clone();
    let reason = "design/brainstorm/pack-r1.md: No such file or directory".to_string();
    fx.done(op, OpResult::DesignPackUnreadable { reason });
    assert_eq!(fx.run().state, RunState::Halted);
    let (ok, value) = answered(&submit_draft(&mut fx, CLAUDE, DRAFT));
    assert!(ok, "{value}");
    ended(&mut fx, "claude", 1, ScoutEnd::Reported);
    assert_eq!(
        states(&fx),
        [DesignAgentState::Submitted, DesignAgentState::Queued]
    );
    assert_eq!(
        replies(&resume(&mut fx)),
        vec![Ok(format!("run {RUN_ID} resumed"))]
    );
    assert_eq!(fx.run().state, RunState::Brainstorming);
    started(&mut fx, "codex", CODEX);
    let (ok, value) = answered(&submit_draft(&mut fx, CODEX, DRAFT));
    assert!(ok, "{value}");
    ended(&mut fx, "codex", 2, ScoutEnd::Reported);
    assert_eq!(
        states(&fx),
        [DesignAgentState::Done, DesignAgentState::Done]
    );
    assert!(design(&fx).drafts_settled);
}
