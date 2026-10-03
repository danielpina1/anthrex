//! Milestone 9.3's final fix wave (W1), the rounds engine: a `pr` run's completion
//! summary leaves its last round's own (review A, I4); iterate refuses a run at the
//! stage limit (A-M5).

use proto::{PrState, RunState};
use serde_json::json;

use super::fixture::*;
use super::goal_rounds_pr::{deliver_round_two, delivering};
use super::goal_rounds_start::{complete, iterate, reply, started};
use super::orch::{answer, edit_plan};
use crate::run::engine::actions::{self, ActionNode};

/// The orchestrator's `edit_plan summary`, accepted.
fn summary(fx: &mut Fixture, text: &str) {
    let effects = edit_plan(fx, json!({"edits": [], "summary": text}));
    assert!(answer(&effects).0, "{effects:#?}");
}

/// I4: round 2 of a `pr` run ends delivered and the orchestrator writes its summary;
/// later every PR lands and the run completes. The completion's summary is the run's
/// and leaves round 2's own.
#[test]
fn a_pr_runs_completion_summary_leaves_the_last_rounds_own() {
    let mut fx = delivering();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    deliver_round_two(&mut fx);
    assert!(fx.run().rounds[1].ended_at.is_some());
    summary(&mut fx, "round two");
    // Every PR landed and the run complete (set in place: landing is 9.2's, pinned by
    // `delivery_land`; only the summary's record matters here).
    let run = fx.run_mut();
    for stage in &mut run.delivery.stages {
        if let Some(pr) = stage.pr.as_mut() {
            pr.state = PrState::Merged;
        }
        stage.landed = Some(PrState::Merged);
    }
    run.state = RunState::Complete;
    summary(&mut fx, "the whole run");
    let run = fx.run();
    assert_eq!(run.rounds[1].summary.as_deref(), Some("round two"));
    let o = run.orch.orchestrator.as_ref().unwrap();
    assert_eq!(o.summary.as_deref(), Some("the whole run"));
}

/// A-M5 (D6): a run whose stages reach `STAGES_MAX` (32) cannot plan a round, which
/// needs a new stage: the iterate is refused at once, the action menu does not offer
/// it, and no round starts.
#[test]
fn iterate_refuses_a_run_at_the_stage_limit() {
    let mut fx = complete();
    // A run at the limit (set in place: 32 planned stages is a long fixture).
    fx.task_mut("t1").spec.stage = proto::STAGES_MAX;
    let text = "run 3f9a has 32 stages, the most a run can have; accept or discard it \
                and start a new goal";
    let listed = |fx: &Fixture| {
        let menu = actions::available(fx.run(), &ActionNode::Run);
        menu.iter().any(|a| a.kind == proto::ActionKind::Iterate)
    };
    assert!(!listed(&fx), "the menu does not offer it");
    assert_eq!(reply(&iterate(&mut fx, "more")), Err(text.to_string()));
    assert_eq!(fx.run().rounds.len(), 1);
    // One stage under the limit, a round still starts.
    fx.task_mut("t1").spec.stage = proto::STAGES_MAX - 1;
    assert!(listed(&fx));
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
}
