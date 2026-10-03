//! Milestone 9.3's final fix wave (W1), the rounds engine: a `pr` run's completion
//! summary leaves its last round's own (review A, I4); iterate refuses a run at the
//! stage limit (A-M5); a landing during a round holds its first stage for the base
//! fetch (task 5's re-review).

use proto::{PrState, RunState};
use serde_json::json;

use super::delivery_land::merged_view;
use super::delivery_sync::{base_sync, fetched};
use super::delivery_watch_adopt::poll_stage;
use super::fixture::*;
use super::goal_rounds_pr::{deliver_round_two, delivering};
use super::goal_rounds_stages::{add_in, creating, plan_round};
use super::goal_rounds_start::{complete, iterate, reply, started};
use super::merge::commit;
use super::orch::{answer, edit_plan};
use crate::run::engine::OpResult;
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

/// Task 5's re-review (decision 13, KG §2.5): round 2 is iterated while stage 1's PR is
/// open, so no base fetch is due then; the user merges that PR while round 2 is
/// approved. Round 2's first stage is not created until that landing's base fetch has
/// answered, and the fetched base goes into it before any of the round's work.
#[test]
fn a_landing_during_a_round_holds_its_first_stage_for_the_fetch() {
    let mut fx = delivering();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    assert!(!fx.run().delivery.base_fetch_due, "stage 1's PR is open");
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    // The widened run's stage 1 is being created at its PR's head when the user merges
    // that PR on the host.
    let ops = creating(&fx);
    assert_eq!(ops.len(), 1, "{ops:#?}");
    assert!(ops[0].1.ends_with("/stage-1"), "{ops:#?}");
    let now = fx.now;
    let pr = fx.run_mut().delivery.stages[0].pr.as_mut().unwrap();
    pr.next_poll_at = now;
    let head = pr.pushed_head.clone();
    poll_stage(&mut fx, 1, merged_view(12, &head, &commit(70)));
    assert_eq!(fx.run().delivery.pr(1).unwrap().state, PrState::Merged);
    // Stage 1 is created; stage 2 waits for the landing's base fetch.
    fx.done(ops[0].0, OpResult::StageCreated);
    fx.tick();
    assert!(creating(&fx).is_empty(), "{:#?}", fx.run().log);
    let base = commit(71);
    fetched(&mut fx, &base, Some(1));
    fx.tick();
    let ops = creating(&fx);
    assert_eq!(ops.len(), 1, "stage 2 once the landing's fetch answered");
    assert!(ops[0].1.ends_with("/stage-2"), "{ops:#?}");
    fx.done(ops[0].0, OpResult::StageCreated);
    let (_, spec) = base_sync(&fx);
    assert_eq!((spec.to, spec.from_head), (2, base));
}
