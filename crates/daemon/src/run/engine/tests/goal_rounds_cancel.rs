//! Milestone 9.3's final fix wave (W1): a round's cancel. A `pr` round cancelled
//! before its first stage exists, above landed PRs, still ends and leaves the run free
//! to iterate, cancel or end (review A, C1); a round's cancel spares the earlier
//! rounds' engine-made work (A-I1); and a halted round can be cancelled (A-M7).

use proto::{RoundOutcome, RunState, TaskState};
use serde_json::json;

use super::delivery_sync::{fetched, fetching};
use super::fixture::*;
use super::goal_rounds_end::{create_stages, settle_ops};
use super::goal_rounds_pr::{landed, sessions_end};
use super::goal_rounds_stages::{add_in, plan_round};
use super::goal_rounds_start::{iterate, reply, started};
use super::kinds_cancel::cancel;
use super::merge::{commit, pending};
use super::orch::{answer, edit_plan};
use super::orch_restore::restart;
use crate::run::engine::OpResult;

/// The run settles: stages created, ops answered, sessions ended and the ref guard
/// passed, until it is `complete` or six passes went by.
fn settle(fx: &mut Fixture) {
    for _ in 0..6 {
        create_stages(fx);
        settle_ops(fx);
        sessions_end(fx);
        if let Some((op, _)) = pending(fx, "VerifyRefs", None).first().cloned() {
            fx.done(op, OpResult::RefsOk);
        }
        if fx.run().state == RunState::Complete {
            return;
        }
        fx.tick();
    }
}

/// After C1's cancel and the fetch's answer: round 2 ended `cancelled`, nothing is due
/// into a stage that was never created, and the run is `complete` and can be iterated.
fn assert_the_round_ended(mut fx: Fixture) {
    settle(&mut fx);
    let run = fx.run();
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Cancelled));
    assert!(run.rounds[1].ended_at.is_some(), "{:#?}", run.log);
    assert!(
        run.delivery.base_sync_due.is_empty(),
        "{:?}",
        run.delivery.base_sync_due
    );
    assert!(!run.finish_edit);
    assert_eq!(run.state, RunState::Complete, "{:#?}", run.log);
    assert_eq!(reply(&iterate(&mut fx, "again")), started(3));
}

/// C1: every PR landed, round 2 iterated and approved, its stage 2 waiting for the
/// base fetch; the user cancels the round before the fetch answers. The fetched base
/// is not made due into stage 2, which no live task needs, and the round ends.
#[test]
fn a_landed_runs_round_cancelled_before_its_first_stage_exists_ends() {
    let mut fx = landed();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    create_stages(&mut fx);
    assert_eq!(fx.run().stages.len(), 1, "stage 2 waits for the fetch");
    assert!(fetching(&fx));
    assert_eq!(
        reply(&cancel(&mut fx)),
        Ok("run 3f9a round 2 cancelled; it ends once its sessions have ended".into())
    );
    fetched(&mut fx, &commit(71), None);
    assert_the_round_ended(fx);
}

/// C1's restart variant: round 2 is being planned when the daemon restarts (the run
/// paused from `planning`, its base fetch still due), then the user cancels the round
/// and the fetch answers.
#[test]
fn a_landed_runs_round_cancelled_after_a_restart_in_planning_ends() {
    let mut fx = landed();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    let effects = edit_plan(&mut fx, json!({"edits": [add_in("t2", "mail", 2, &[])]}));
    assert!(answer(&effects).0, "{effects:#?}");
    restart(&mut fx);
    assert_eq!(fx.run().state, RunState::Paused);
    assert!(reply(&cancel(&mut fx)).is_ok());
    create_stages(&mut fx);
    if fetching(&fx) {
        fetched(&mut fx, &commit(71), None);
    }
    assert_the_round_ended(fx);
}

/// C1, `sync::queue`'s side: a stage that was never created, and none of whose tasks is
/// left to run, is never the target of a base sync, however its tasks ended.
#[test]
fn a_fetched_base_is_never_due_into_a_stage_never_created() {
    let mut fx = landed();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    create_stages(&mut fx);
    fx.force("t2", TaskState::Cancelled);
    assert!(fx.run().stage_head(2).is_none());
    fetched(&mut fx, &commit(71), None);
    let due = &fx.run().delivery.base_sync_due;
    assert!(due.is_empty(), "{due:?}");
}
