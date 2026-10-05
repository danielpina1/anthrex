//! Ruling T15-18 (the final fix wave's FW-2): a later round's documents commit that is
//! due, with none in flight and no live task of the round left, is dropped with the
//! round, as a reject drops it: through the `finish` edit before the commit is sent,
//! or every task of the round cancelled during a commit-failure halt. Nothing is left
//! due, and the next round's commit carries only its own amendment.

use proto::{PlanEdit, RunState};
use serde_json::json;

use super::control::resume;
use super::delivery_ci::logged;
use super::design_commit::{approve, commits};
use super::design_rounds::round_task;
use super::design_rounds_commit::{approved_round, pr_complete};
use super::design_rounds_fix3::round_three_at_plan_gate;
use super::design_rounds_fixture::*;
use super::dispatch::edit;
use super::fixture::*;
use super::goal_rounds_cancel::settle;
use super::goal_rounds_start::reply;
use crate::run::design::state::DesignRound;
use crate::run::engine::OpResult;

const DROPPED: &str =
    "round 2's documents were not committed: no task of the round remains; the round is dropped";

/// The pending ops named `name`.
fn pending(fx: &Fixture, name: &str) -> Vec<u64> {
    (fx.run().pending_ops.values())
        .filter(|p| format!("{:?}", p.kind).starts_with(name))
        .map(|p| p.op)
        .collect()
}

fn round_of(fx: &Fixture) -> DesignRound {
    let design = fx.run().orch.design.as_ref().unwrap();
    design.round.clone().expect("a later round")
}

/// Round 2 dropped: nothing due, the design state back to round 1's approval, the
/// round record marked, the exact log line.
fn assert_dropped(fx: &Fixture, round: &DesignRound) {
    let run = fx.run();
    let design = run.orch.design.as_ref().unwrap();
    assert!(!design.commit_due, "{:#?}", run.log);
    assert!(!crate::run::engine::design_commit::due(run));
    assert!(logged(fx, DROPPED), "{:#?}", run.log);
    assert!(run.rounds[1].dropped);
    assert_eq!(run.rounds[1].approved_at, None);
    assert_eq!(design.approved_spec, round.spec_before);
    assert_eq!(design.requirements, round.base);
    assert_eq!(design.committed_round, 1);
}

/// The next round is clean: round 3's documents commit carries only its own amendment.
fn assert_next_round_clean(fx: &mut Fixture) {
    round_three_at_plan_gate(fx, 3, "plan-r3");
    let mut effects = approve(fx);
    for _ in 0..4 {
        if !commits(&effects).is_empty() {
            break;
        }
        for op in pending(fx, "CreateStageBranch") {
            effects.extend(fx.done(op, OpResult::StageCreated));
        }
        effects.extend(fx.tick());
    }
    let asked = commits(&effects);
    assert_eq!(asked.len(), 1, "{effects:?}");
    let amended: Vec<(&str, Option<&str>)> = (asked[0].1.files.iter())
        .filter(|f| f.folder == "specs")
        .map(|f| (f.what.as_str(), f.append.as_deref()))
        .collect();
    assert_eq!(amended, [("spec v3", Some("## Round 3 amendment"))]);
}

#[test]
fn a_finish_before_the_round_commit_is_sent_drops_the_round() {
    let mut fx = round_plan_gate(json!([round_task("t2", &["R2", "R3"])]));
    let round = round_of(&fx);
    assert!(commits(&approve(&mut fx)).is_empty(), "stage 1 comes first");
    let (op, _) = fx.op("CreateStageBranch");
    assert!(reply(&edit(&mut fx, vec![PlanEdit::Finish])).is_ok());
    let effects = fx.done(op, OpResult::StageCreated);
    assert!(commits(&effects).is_empty(), "no commit is sent");
    settle(&mut fx);
    assert_eq!(fx.run().state, RunState::Complete, "{:#?}", fx.run().log);
    assert_dropped(&fx, &round);
    assert_next_round_clean(&mut fx);
}

#[test]
fn every_task_cancelled_during_a_commit_failure_halt_drops_the_round() {
    let (mut fx, op, _) = approved_round(design_complete());
    let round = round_of(&fx);
    fx.done(
        op,
        OpResult::Failed {
            message: "index.lock exists".into(),
        },
    );
    assert_eq!(fx.run().state, RunState::Halted);
    assert!(fx.run().halt_retryable);
    let cancel = PlanEdit::CancelTask {
        task_id: "t2".into(),
    };
    assert!(reply(&edit(&mut fx, vec![cancel])).is_ok());
    fx.tick();
    assert_dropped(&fx, &round);
    let sent = fx.ops("CommitDesignDocs").len();
    resume(&mut fx);
    settle(&mut fx);
    assert_eq!(fx.run().state, RunState::Complete, "{:#?}", fx.run().log);
    assert_eq!(
        fx.ops("CommitDesignDocs").len(),
        sent,
        "the commit was sent again"
    );
    assert_next_round_clean(&mut fx);
}

/// The same in a `pr` run: the round's commit failed, every task cancelled.
#[test]
fn a_pr_rounds_commit_failure_with_no_task_left_drops_the_round() {
    let (mut fx, op, _) = approved_round(pr_complete());
    let round = round_of(&fx);
    let failed = OpResult::Failed {
        message: "index.lock exists".into(),
    };
    fx.done(op, failed);
    let cancel = PlanEdit::CancelTask {
        task_id: "t2".into(),
    };
    assert!(reply(&edit(&mut fx, vec![cancel])).is_ok());
    fx.tick();
    assert_dropped(&fx, &round);
}
