//! Milestone 9.3 task 5, fix round 1: a `pr` round's reject goes back to `complete`
//! only when every earlier stage's landing is processed (I1), a later round above
//! landed PRs still fetches the base first after a rejected one (m1), and the `pr`
//! end of a round waits for a due base sync and a held stage (m5).

use proto::{PrState, RunState};
use serde_json::json;

use super::delivery_land::{merged_view, stage_lines};
use super::delivery_open::{answer as host_answer, host_ops, opened};
use super::delivery_sync::{base_sync, fetched};
use super::fixture::*;
use super::goal_rounds_end::{create_stages, settle_ops, submit_round};
use super::goal_rounds_pr::{delivering, landed, opening, reject, sessions_end};
use super::goal_rounds_stages::{add_in, creating, plan_round};
use super::goal_rounds_start::{iterate, reply, started};
use super::kinds_integration::{C3, merge_real};
use super::merge::{commit, pending};
use super::propagate::merged_at;
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::OpResult;
use crate::run::engine::goal_rounds::landed_below;
use crate::run::engine::goal_rounds_end::delivered;

/// The `complete: …` log lines of the run: one per completion.
fn completions(fx: &Fixture) -> usize {
    let run = fx.run();
    run.log
        .iter()
        .filter(|l| l.text.starts_with("complete:"))
        .count()
}

/// I1: the only open PR is seen merged while round 2 is planned (its view was in flight
/// at the iterate). A reject sends the run back to running, so the landing is processed
/// (one `stage` line) and the run completes once.
#[test]
fn a_landing_seen_while_a_round_is_planned_is_processed_after_its_reject() {
    let mut fx = delivering();
    let now = fx.now;
    fx.run_mut().delivery.stages[0]
        .pr
        .as_mut()
        .unwrap()
        .next_poll_at = now;
    fx.tick();
    let view = (host_ops(&fx).into_iter()).find(|(_, op)| matches!(op, HostOp::ViewPr { .. }));
    let (view, _) = view.expect("a view of stage 1 in flight");
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    let head = fx.run().delivery.pr(1).unwrap().pushed_head.clone();
    let merged = merged_view(12, &head, &commit(70));
    host_answer(&mut fx, view, HostResult::PrViewed(Box::new(merged)));
    assert_eq!(fx.run().state, RunState::Planning);
    assert_eq!(fx.run().delivery.pr(1).unwrap().state, PrState::Merged);
    submit_round(&mut fx);
    let before = completions(&fx);
    assert!(reply(&reject(&mut fx)).is_ok());
    assert_eq!(fx.run().state, RunState::Running, "{:#?}", fx.run().log);
    // The landing: its base fetch counts the merge's parents, its line goes out.
    create_stages(&mut fx);
    fetched(&mut fx, &commit(71), Some(1));
    for _ in 0..6 {
        create_stages(&mut fx);
        settle_ops(&mut fx);
        sessions_end(&mut fx);
        if let Some((op, _)) = pending(&fx, "VerifyRefs", None).first().cloned() {
            fx.done(op, OpResult::RefsOk);
        }
        if fx.run().state == RunState::Complete {
            break;
        }
        fx.tick();
    }
    assert_eq!(fx.run().state, RunState::Complete, "{:#?}", fx.run().log);
    assert_eq!(stage_lines(&fx.log).len(), 1, "{:#?}", stage_lines(&fx.log));
    assert_eq!(
        completions(&fx),
        before + 1,
        "one completion after the reject"
    );
}

/// m1: a landed run's round 2 rejected (back to `complete`, its own fetch flag
/// cleared), then iterated again: round 3 still makes the fetch due first, its first
/// stage waits for it, and the fetched base goes into that stage before its task
/// merges.
#[test]
fn a_round_after_a_rejected_one_still_fetches_the_base_first() {
    let mut fx = landed();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    submit_round(&mut fx);
    assert!(reply(&reject(&mut fx)).is_ok());
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(
        !fx.run().delivery.base_fetch_due,
        "the rejected round's fetch is dropped"
    );
    assert_eq!(reply(&iterate(&mut fx, "again")), started(3));
    assert!(fx.run().delivery.base_fetch_due);
    plan_round(&mut fx, json!([add_in("t3", "web", 3, &[])]));
    // Stages 1 and 2 (the rejected round's, empty) are created; stage 3 waits.
    create_stages(&mut fx);
    assert_eq!(fx.run().stages.len(), 2, "{:?}", fx.run().stages);
    assert!(creating(&fx).is_empty());
    let base = commit(71);
    fetched(&mut fx, &base, None);
    let ops = creating(&fx);
    assert_eq!(ops.len(), 1, "{ops:?}");
    fx.done(ops[0].0, OpResult::StageCreated);
    let (op, spec) = base_sync(&fx);
    assert_eq!((spec.to, spec.from_head.as_str()), (3, base.as_str()));
    fx.done(op, merged_at(&commit(72)));
    merge_real(&mut fx, "t3", C3);
    assert!(fx.run().stages[2].tasks_in.contains("t3"));
}

/// m5: the `pr` end of a round also waits for a held stage, and is not reached while a
/// base sync is due.
#[test]
fn a_pr_round_end_waits_for_a_held_stage_and_a_due_base_sync() {
    let (mut fx, op) = opening();
    fx.run_mut().delivery.stages[0].held = Some("protected branch".into());
    host_answer(&mut fx, op, opened(13, false));
    assert_eq!(fx.run().rounds[1].ended_at, None);
    let mut due = fx.run().clone();
    due.delivery.stages[0].held = None;
    assert!(delivered(&due));
    due.delivery.base_sync_due.insert(2, commit(71));
    assert!(!delivered(&due));
    fx.run_mut().delivery.stages[0].held = None;
    fx.tick();
    assert_eq!(fx.run().rounds[1].ended_at, Some(fx.now));
}

/// m1: a stage below with no delivery record has landed when it merged nothing and
/// nothing of it is left to run (it is skipped once delivery looks at it), and has not
/// when it merged work no PR carries yet.
#[test]
fn a_stage_without_a_delivery_record_lands_only_when_it_merged_nothing() {
    let mut fx = landed();
    let mut t9 = fx.task("t1").clone();
    t9.spec.id = "t9".into();
    t9.spec.stage = 2;
    t9.state = proto::TaskState::Cancelled;
    fx.run_mut().tasks.push(t9);
    assert!(fx.run().delivery.stage(2).is_none());
    assert!(landed_below(fx.run(), 3));
    fx.task_mut("t9").state = proto::TaskState::Merged;
    assert!(!landed_below(fx.run(), 3));
    fx.task_mut("t9").state = proto::TaskState::Working;
    assert!(!landed_below(fx.run(), 3));
}
