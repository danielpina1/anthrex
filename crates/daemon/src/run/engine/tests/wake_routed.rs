//! Milestone 9.9 task M9.9.4 (OFA section 4.1): the alerts routed to the orchestrator
//! each wake it. Four sources had no note before this task (a held tier 3, a re-red
//! with a fix task open, a review round over the cap, a host op that keeps failing);
//! the halt and the block already woke it and are pinned here.

use proto::RunState;

use super::bisect::{TEST, answer, merge_next, with_orchestrator as bisect_orchestrator};
use super::deciders_block::{REASON, classified, untyped_block, working};
use super::delivery_open::{answer as host_answer, host_op};
use super::delivery_review::{reviewed, said};
use super::delivery_watch::{poll_with, view, watched};
use super::fixture::*;
use super::full::{full_job, merge_tiered, outcome, profile, tier};
use super::full_fixes::with_orchestrator;
use super::merge::{commit, doc_task, pending, start_on, to_queue, window_of};
use super::wake_notes::{clear, notes};
use crate::decider::BlockKind;
use crate::host::HostError;
use crate::run::contract::sha7;
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{EventKind, OpResult};

fn failed() -> OpResult {
    OpResult::Failed {
        message: "git: could not lock the index".into(),
    }
}

#[test]
fn a_held_tier3_wakes_the_orchestrator() {
    let (mut fx, windows) = start_on(&profile(), &[doc_task("t1", ""), doc_task("t2", "")]);
    super::full::block(&mut fx, "t2", window_of(&windows, "t2"));
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    let since = fx.run().queue_idle_since.unwrap();
    let (op, _) = super::full::full_jobs(&fx.send(since + 120, EventKind::Tick))[0].clone();
    with_orchestrator(&mut fx);
    fx.done(op, failed());
    assert_eq!(notes(&fx), Vec::<String>::new());
    fx.send(fx.now + 30, EventKind::Tick);
    let (op, _) = full_job(&fx);
    fx.done(op, failed());
    assert_eq!(notes(&fx), Vec::<String>::new());
    fx.send(fx.now + 120, EventKind::Tick);
    let (op, _) = full_job(&fx);
    fx.done(op, failed());
    assert_eq!(
        notes(&fx),
        ["stage 1 tier 3 held after 3 executor failures; resume_run with stage 1 retries it"]
    );
}

#[test]
fn a_red_again_with_an_open_fix_task_wakes_it() {
    let ids = ["t1", "t2", "t3", "t4"];
    let tasks: Vec<String> = ids.iter().map(|id| doc_task(id, "")).collect();
    let (mut fx, mut windows) = start_on(&profile(), &tasks);
    bisect_orchestrator(&mut fx);
    for (k, id) in ids[..3].iter().enumerate() {
        merge_next(&mut fx, &mut windows, id, &commit(k as u32 + 1));
    }
    let since = fx.run().queue_idle_since.expect("idle");
    fx.send(since + 120, EventKind::Tick);
    let (op, _) = full_job(&fx);
    fx.done(op, tier(outcome(3, &[TEST])));
    answer(&mut fx, 2);
    merge_next(&mut fx, &mut windows, "t4", &commit(7));
    clear(&mut fx);
    let now = fx.now;
    let mut effects = Vec::new();
    assert!(crate::run::engine::full::request(
        fx.run_mut(),
        1,
        crate::run::engine::full::FullWhy::Deliver,
        now,
        &mut effects
    ));
    let (op, _) = full_job(&fx);
    fx.done(op, tier(outcome(3, &[TEST])));
    let line = format!(
        "stage 1 tier 3 red again on {}; fix task fix1 is still open",
        sha7(&commit(7))
    );
    assert_eq!(notes(&fx), [line]);
}

#[test]
fn a_review_round_over_the_cap_wakes_it() {
    let mut fx = reviewed();
    fx.run_mut().delivery.limits.reviewers = vec!["alice".into()];
    bisect_orchestrator(&mut fx);
    fx.run_mut().delivery.limits.review_fix_max = 0;
    let mut v = view(&commit(1));
    v.comments = vec![said(5, "alice", "one")];
    let (at, _) = poll_with(&mut fx, v);
    fx.send(at + 1, EventKind::Tick);
    let line = "PR #7: review round 1 is over the cap; thread 7:c5 by @alice is yours";
    assert_eq!(
        notes(&fx),
        [format!(
            "{line}; decide: a fix task, a reply_comment, or ask_user"
        )]
    );
}

#[test]
fn a_host_op_that_keeps_failing_wakes_it_once() {
    let mut fx = watched();
    bisect_orchestrator(&mut fx);
    fx.run_mut().delivery.watching = false;
    set_stage_head(fx.run_mut(), 1, &commit(5));
    for _ in 1..=7u32 {
        fx.tick();
        let (op, push) = host_op(&fx);
        assert!(matches!(push, HostOp::Push { .. }), "{push:?}");
        host_answer(
            &mut fx,
            op,
            HostResult::Error(HostError::Failed("boom".into())),
        );
    }
    assert_eq!(notes(&fx), ["PR #7: push keeps failing: \"boom\""]);
}

#[test]
fn every_halt_already_wakes_it() {
    let profile = profile_with("max_writers = 2");
    let (mut fx, windows) = start_on(&profile, &[doc_task("t1", ""), doc_task("t2", "")]);
    bisect_orchestrator(&mut fx);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    let (op, _) = pending(&fx, "MergeCandidate", Some("t1"))[0].clone();
    let reason = format!("refs/heads/anthrex/{RUN_ID}/integration moved from 1eeeeee to 9999999");
    fx.done(op, OpResult::RefMoved { reason });
    assert_eq!(fx.run().state, RunState::Halted);
    let halted: Vec<String> = notes(&fx)
        .into_iter()
        .filter(|n| n.starts_with("the run halted: "))
        .collect();
    assert_eq!(halted.len(), 1, "{:#?}", notes(&fx));
}

#[test]
fn every_block_already_wakes_it() {
    let (mut fx, window) = working();
    bisect_orchestrator(&mut fx);
    let op = untyped_block(&mut fx, window);
    fx.decided(op, classified(BlockKind::Environment));
    let want = format!("t1 blocked (environment): {REASON}");
    assert!(notes(&fx).contains(&want), "{:#?}", notes(&fx));
}
