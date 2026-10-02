//! The final fix wave's I-3 (review B): closing a PR, then reopening it, and cancelling
//! a review fix lose no work. Whenever a task with `addresses` ends cancelled (a
//! `cancel_task`, a rejected hold, a split, a close), the threads it had tasked are
//! `new` again and are handed out through the normal path; a reopen re-arms the stage's
//! CI judgement on its head. Cancelled fix tasks are never resurrected.

use proto::{PlanEdit, PrState, TaskState};

use super::delivery_ci::RUN_A;
use super::delivery_ci_dupes::red;
use super::delivery_land::{closed_view, logged};
use super::delivery_review::{noted, on, said, state};
use super::delivery_review_reply::by_alice;
use super::delivery_watch::{PR, poll_with, view};
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::full::attention;
use super::merge::commit;
use crate::run::delivery::ThreadState;
use crate::run::engine::{EventKind, OrchEvent};

fn tasked(id: &str) -> ThreadState {
    ThreadState::Tasked { task: id.into() }
}

/// A view at `v`, then a tick past the review batch's quiet second.
fn batch(fx: &mut Fixture, v: crate::host::PrView) {
    let (at, _) = poll_with(fx, v);
    fx.send(at + 1, EventKind::Tick);
}

/// `c5` by alice, handed to the fast path's fix task `fix1`.
fn c5_tasked() -> Fixture {
    let mut fx = by_alice();
    let mut v = view(&commit(1));
    v.comments = vec![said(5, "alice", "Please update the README.")];
    batch(&mut fx, v);
    assert_eq!(state(&fx, "c5"), tasked("fix1"));
    fx
}

fn cancelled(fx: &Fixture, id: &str) -> bool {
    fx.task(id).state == TaskState::Cancelled
}

#[test]
fn a_red_seen_before_a_close_gets_a_fix_again_after_the_reopen() {
    let mut fx = super::delivery_watch::watched();
    let fix1 = red(&mut fx, 1, PR, RUN_A).expect("a CI fix");
    poll_with(&mut fx, closed_view(PR, &commit(1)));
    fx.tick();
    assert!(cancelled(&fx, &fix1), "the close cancels the stage's fixes");
    // Reopened: the red on the same head is judged again, and gets a new fix.
    let reopened = super::delivery_ci::red_view(&commit(1), super::delivery_ci::test_red(RUN_A));
    poll_with(&mut fx, reopened);
    fx.tick();
    assert_eq!(fx.run().delivery.pr(1).unwrap().state, PrState::Open);
    assert!(logged(
        &fx,
        "stage 1 (PR #7): CI at 1eeeeee is judged again"
    ));
    let fix2 = red(&mut fx, 1, PR, RUN_A).expect("a fix again after the reopen");
    assert_ne!(fix2, fix1);
    assert!(!cancelled(&fx, &fix2));
    let closed = "stage 1 PR closed without merging; resume, re-plan, or cancel the rest";
    assert!(!attention(&fx).contains(&closed.to_string()));
}

#[test]
fn a_thread_tasked_before_a_close_is_handed_out_again_after_the_reopen() {
    let mut fx = c5_tasked();
    poll_with(&mut fx, closed_view(PR, &commit(1)));
    fx.tick();
    assert!(cancelled(&fx, "fix1"));
    assert_eq!(state(&fx, "c5"), ThreadState::New);
    assert!(logged(
        &fx,
        "stage 1 (PR #7): thread c5 is new again: its fix task fix1 was cancelled"
    ));
    // While the PR is closed its batch waits.
    let later = fx.now + 30;
    fx.send(later, EventKind::Tick);
    assert_eq!(state(&fx, "c5"), ThreadState::New);
    batch(&mut fx, view(&commit(1)));
    fx.tick();
    assert_eq!(
        state(&fx, "c5"),
        tasked("fix2"),
        "a new fix, not fix1 again"
    );
    assert!(cancelled(&fx, "fix1"));
}

#[test]
fn a_cancelled_review_fix_hands_its_thread_out_again() {
    let mut fx = c5_tasked();
    let cancel = PlanEdit::CancelTask {
        task_id: "fix1".into(),
    };
    assert!(replies(&edit(&mut fx, vec![cancel]))[0].is_ok());
    fx.tick();
    assert!(logged(
        &fx,
        "stage 1 (PR #7): thread c5 is new again: its fix task fix1 was cancelled"
    ));
    let later = fx.now + 2;
    fx.send(later, EventKind::Tick);
    assert_eq!(state(&fx, "c5"), tasked("fix2"));
}

#[test]
fn a_rejected_hold_hands_its_thread_out_again() {
    let mut fx = by_alice();
    let mut v = view(&commit(1));
    v.threads = vec![on(
        "Cargo.toml",
        vec![noted(30, "alice", "bump the version")],
    )];
    batch(&mut fx, v);
    assert_eq!(state(&fx, "t30"), tasked("fix1"));
    let event = OrchEvent::RejectHold {
        reply: fx.reply(),
        run_id: RUN_ID.into(),
        hold: "hold-fix1".into(),
    };
    let effects = fx.next(EventKind::Orch(event));
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    fx.tick();
    assert!(cancelled(&fx, "fix1"));
    assert!(logged(
        &fx,
        "stage 1 (PR #7): thread t30 is new again: its fix task fix1 was cancelled"
    ));
    let later = fx.now + 2;
    fx.send(later, EventKind::Tick);
    assert_eq!(state(&fx, "t30"), tasked("fix2"));
    let task = fx.task("fix2");
    assert_eq!(
        task.orch.gate_hold.as_deref(),
        Some("hold-fix2"),
        "held again"
    );
}

#[test]
fn a_red_seen_while_paused_gets_a_fix_once_the_stage_below_reopens() {
    let (mut fx, _) = super::delivery_watch_adopt::two_stages(true);
    let h1 = fx.run().stage_head(1).unwrap().to_string();
    super::delivery_watch_adopt::poll_stage(&mut fx, 1, closed_view(11, &h1));
    fx.tick();
    assert!(fx.run().delivery.stage(2).unwrap().paused_by.is_some());
    // Stage 2's red while it is paused adds nothing.
    assert_eq!(red(&mut fx, 2, 12, RUN_A), None);
    let open = super::delivery_watch_adopt::view_of(11, &h1);
    super::delivery_watch_adopt::poll_stage(&mut fx, 1, open);
    fx.tick();
    assert!(fx.run().delivery.stage(2).unwrap().paused_by.is_none());
    assert!(
        red(&mut fx, 2, 12, RUN_A).is_some(),
        "judged again once unpaused"
    );
}
