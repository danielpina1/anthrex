//! Milestone 9.3's final fix wave (W1): a round's cancel. A `pr` round cancelled
//! before its first stage exists, above landed PRs, still ends and leaves the run free
//! to iterate, cancel or end (review A, C1); a round's cancel spares the earlier
//! rounds' engine-made work (A-I1); and a halted round can be cancelled (A-M7).

use proto::{RoundOutcome, RunState, TaskOrigin, TaskState};
use serde_json::json;

use super::delivery_sync::{fetched, fetching};
use super::fixes::spec as fixes_spec;
use super::fixture::*;
use super::goal_rounds_end::{create_stages, halted_in_round_two, settle_ops};
use super::goal_rounds_pr::{delivering, landed, sessions_end};
use super::goal_rounds_stages::{add_in, plan_round};
use super::goal_rounds_start::{iterate, reply, started};
use super::kinds_cancel::cancel;
use super::kinds_integration::{C1, C2};
use super::merge::{commit, pending};
use super::orch::{answer, edit_plan};
use super::orch_restore::restart;
use super::scenarios::halt_and_rebaseline;
use crate::run::engine::OpResult;
use crate::run::engine::fixes::add_fix;

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

/// An engine-made bisect fix task on stage 1 (round 1's), queued; its id.
fn stage_one_fix(fx: &mut Fixture) -> String {
    let spec = fixes_spec(TaskOrigin::Bisect, "crates/fix/**", Default::default());
    let now = fx.now;
    let id = add_fix(fx.run_mut(), spec, now, &mut Vec::new()).expect("added");
    assert_eq!(fx.task(&id).round, 1);
    id
}

/// A-I1 (decision 16, "the earlier rounds intact"): round 2's cancel cancels round 2's
/// tasks only. An engine-made fix on stage 1, queued before the cancel or added after
/// it while the round waits to end, keeps going.
#[test]
fn a_round_cancel_spares_an_earlier_rounds_fix() {
    let mut fx = delivering();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    create_stages(&mut fx);
    let before = stage_one_fix(&mut fx);
    assert!(reply(&cancel(&mut fx)).is_ok());
    assert_eq!(fx.task("t2").state, TaskState::Cancelled);
    fx.tick();
    let after = stage_one_fix(&mut fx);
    fx.tick();
    for id in [&before, &after] {
        let state = fx.task(id).state;
        assert!(
            !state.is_finished(),
            "{id} is {state:?}: {:#?}",
            fx.run().log
        );
    }
    assert_eq!(
        fx.run().rounds[1].ended_at,
        None,
        "the round waits for them"
    );
}

/// A-M7 (KG §2.4, "the user resumes it or cancels the round"): round 2 halted (its
/// merge found the stage ref moved). The round's cancel answers with the round's text
/// and leaves the run halted, the round `cancelled` and nothing new starting; the
/// resume that rebaselines it then lets the queued merge land, the run completes and
/// the round ends `cancelled`.
#[test]
fn a_halted_round_can_be_cancelled() {
    let mut fx = halted_in_round_two();
    assert_eq!(fx.run().state, RunState::Halted);
    assert_eq!(
        reply(&cancel(&mut fx)),
        Ok("run 3f9a round 2 cancelled; it ends once its sessions have ended".into())
    );
    let run = fx.run();
    assert_eq!(run.state, RunState::Halted);
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Cancelled));
    assert_eq!(run.rounds[1].ended_at, None);
    assert!(run.finish_edit && !run.cancelled);
    halt_and_rebaseline(&mut fx, vec![(1, C1.into()), (2, C1.into())]);
    assert_eq!(fx.run().state, RunState::Running, "{:#?}", fx.run().log);
    if let Some((op, _)) = pending(&fx, "MergeCandidate", Some("t2")).first().cloned() {
        let merged = OpResult::Merged {
            commit: C2.into(),
            tier: None,
        };
        fx.done(op, merged);
    }
    settle(&mut fx);
    let run = fx.run();
    assert_eq!(run.state, RunState::Complete, "{:#?}", run.log);
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Cancelled));
    assert!(run.rounds[1].ended_at.is_some());
    assert!(!run.finish_edit);
}

/// W1 fix round 2 (item 4): while round 2 is being cancelled, the orchestrator can still
/// answer a live round-1 fix's question (A-I2's "and by the orchestrator"); the round's
/// cancel is not the run's finish.
#[test]
fn the_orchestrator_answers_an_earlier_rounds_fix_while_a_round_is_cancelled() {
    let mut fx = super::actions_rounds::round_two_fixes();
    assert!(reply(&cancel(&mut fx)).is_ok());
    assert_eq!(fx.task("fix1").state, TaskState::Blocked);
    let edit = json!({"edits": [{"op": "answer", "task_id": "fix1", "text": "the users table"}]});
    let effects = super::orch::edit_plan(&mut fx, edit);
    assert!(answer(&effects).0, "{effects:#?}");
    assert_ne!(
        fx.task("fix1").state,
        TaskState::Blocked,
        "{:#?}",
        fx.run().log
    );
    // Work for the round itself is still refused: the round is ending.
    let add = json!({"edits": [add_in("t3", "mail", 2, &[])]});
    assert!(!answer(&super::orch::edit_plan(&mut fx, add)).0);
}

/// W1 fix round 2 (item 6): the user's run-wide `finish` while a cancelled round winds
/// down (an earlier round's fix still going) is the run's: the round's end does not clear
/// it.
#[test]
fn a_finish_during_a_cancelled_rounds_wind_down_stays_the_runs() {
    let mut fx = delivering();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    create_stages(&mut fx);
    let fix = stage_one_fix(&mut fx);
    assert!(reply(&cancel(&mut fx)).is_ok());
    fx.tick();
    assert_eq!(
        fx.run().rounds[1].ended_at,
        None,
        "the round waits for {fix}"
    );
    let finished = super::dispatch::edit(&mut fx, vec![proto::PlanEdit::Finish]);
    assert!(
        super::dispatch::replies(&finished)[0].is_ok(),
        "{finished:#?}"
    );
    fx.force(&fix, TaskState::Cancelled);
    for _ in 0..6 {
        create_stages(&mut fx);
        settle_ops(&mut fx);
        sessions_end(&mut fx);
        fx.tick();
    }
    let run = fx.run();
    assert!(run.rounds[1].ended_at.is_some(), "{:#?}", run.log);
    assert!(run.finish_edit, "the user's finish stays: {:#?}", run.log);
}
