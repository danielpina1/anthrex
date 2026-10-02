//! Milestone 9.2 task M9.2.8: what a view's head and the stage's own head make due.
//! A user's commit on the remote stage branch is adopted (decision 24): it becomes the
//! stage head through 9.1's `set_stage_head` and flows up by propagate, and its line
//! starts at it (ruling R-9); a remote that does not descend from the local head halts
//! with TT's text. An adopt waits for the stage's merge queue and the queue for the
//! adopt. A fix merged into an open PR's stage is pushed once the queue is quiet
//! (decision 28). A push the remote refuses holds only its stage until `run resume`
//! (ruling R-11). A stage paused by a closed PR below starts no task (decision 37).

use proto::{PlanEdit, PrState, RunState, TaskState};

use super::control::resume;
use super::delivery_open::{
    answer, green, host_op, host_ops, host_ops_in, open_stage, opened, pr_mode, pr_on,
    remote_branch,
};
use super::delivery_watch::{PR, fast, poll_with, view, watched, watched_on};
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::full::{attention, delivery_alerts};
use super::merge::{commit, doc_task, merge, pending, to_queue, window_of};
use super::propagate::{land_propagates, propagates, stages_on};
use crate::host::{Adopt, FetchOutcome, PrView, PushOutcome};
use crate::run::delivery::StageDelivery;
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{Effect, EventKind, OpResult};

/// `refs/anthrex/<run>/remote/stage-<n>`, where an adopt fetches (decision 13).
pub(super) fn into(n: u16) -> String {
    format!("refs/anthrex/{RUN_ID}/remote/stage-{n}")
}

pub(super) fn fetch(n: u16, local_ref: String, expected: &str, also_integration: bool) -> HostOp {
    HostOp::Fetch {
        stage: Some(n),
        branch: remote_branch(n),
        into: into(n),
        adopt: Some(Adopt {
            local_ref,
            expected_local: expected.into(),
            also_integration,
        }),
        parents_of: None,
    }
}

fn logged(fx: &Fixture, text: &str) -> bool {
    fx.run().log.iter().any(|l| l.text == text)
}

/// A view of stage PR `number` at `head`.
pub(super) fn view_of(number: u64, head: &str) -> PrView {
    PrView {
        number,
        ..view(head)
    }
}

/// Polls stage `n`'s PR when it is due and answers with `v` in the same second.
pub(super) fn poll_stage(fx: &mut Fixture, n: u16, v: PrView) -> Vec<Effect> {
    let at = fx.run().delivery.pr(n).unwrap().next_poll_at.max(fx.now);
    fx.send(at, EventKind::Tick);
    let want = HostOp::ViewPr {
        stage: n,
        number: v.number,
    };
    let (op, _) = host_ops(fx)
        .into_iter()
        .find(|(_, op)| *op == want)
        .unwrap_or_else(|| panic!("a view of stage {n}: {:?}", host_ops(fx)));
    fx.send(
        at,
        EventKind::OpDone {
            run_id: RUN_ID.into(),
            op,
            result: OpResult::Host(HostResult::PrViewed(Box::new(v))),
        },
    )
}

/// The pending host op of stage `n` (asserting there is one).
pub(super) fn stage_op(fx: &Fixture, n: u16) -> (crate::run::model::OpId, HostOp) {
    let of = |op: &HostOp| crate::run::engine::delivery::stage_of(op) == Some(n);
    let ops: Vec<_> = host_ops(fx).into_iter().filter(|(_, op)| of(op)).collect();
    assert_eq!(ops.len(), 1, "one host op of stage {n}: {ops:#?}");
    ops[0].clone()
}

/// A `pr`-mode `Multi` run: `t1` (stage 1) merged with PR #11 open; `t2` (stage 2)
/// still working, or, with `both`, merged with PR #12 open. Both PRs are polled every
/// second from now on.
pub(super) fn two_stages(both: bool) -> (Fixture, Vec<(String, u32)>) {
    let tasks = [doc_task("t1", ""), doc_task("t2", "stage = 2")];
    let (mut fx, windows) = stages_on(PROFILE, &tasks);
    pr_mode(fx.run_mut());
    fx.tick();
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    land_propagates(&mut fx);
    green(&mut fx, 1);
    open_stage(&mut fx, 1, 11);
    if both {
        to_queue(&mut fx, "t2", window_of(&windows, "t2"));
        merge(&mut fx, "t2", &commit(2));
        land_propagates(&mut fx);
        green(&mut fx, 2);
        open_stage(&mut fx, 2, 12);
    }
    fast(fx.run_mut());
    let now = fx.now;
    for s in fx.run_mut().delivery.stages.iter_mut() {
        if let Some(pr) = s.pr.as_mut() {
            pr.next_poll_at = now + 1;
        }
    }
    (fx, windows)
}

/// Adds S doc tasks `ids` to stage 1 and launches them.
pub(super) fn add(fx: &mut Fixture, ids: &[&str]) -> Vec<(String, u32)> {
    let edits = ids
        .iter()
        .map(|id| {
            let text = plan_with(PROFILE, &[doc_task(id, "")]);
            let task = crate::run::plan::parse_plan(&text).unwrap().tasks.remove(0);
            PlanEdit::AddTask { task }
        })
        .collect();
    let effects = edit(fx, edits);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    fx.launch_all()
}

#[test]
fn user_commit_is_adopted_and_propagated() {
    let (mut fx, _) = two_stages(false);
    let head = fx.run().stage_head(1).unwrap().to_string();
    let user = commit(41);
    let effects = poll_stage(&mut fx, 1, view_of(11, &user));
    // Decision 24: the remote head is not the pushed one, so it is fetched to adopt.
    let local = format!("anthrex/{RUN_ID}/stage-1");
    assert_eq!(
        host_ops_in(&effects),
        vec![fetch(1, local, &head, false)],
        "stage 1 is not the highest: integration stays"
    );
    let stage = fx.run().delivery.stage(1).unwrap();
    assert_eq!(stage.remote_head.as_deref(), Some(user.as_str()));
    let (op, _) = stage_op(&fx, 1);
    let adopted = FetchOutcome::Adopted { sha: user.clone() };
    let effects = answer(&mut fx, op, HostResult::Fetched(adopted));
    let run = fx.run();
    assert_eq!(run.stage_head(1), Some(user.as_str()), "set_stage_head");
    assert_eq!(run.delivery.pr(1).unwrap().pushed_head, user);
    assert_eq!(run.delivery.stage(1).unwrap().remote_head, None);
    let up = run.propagate_due.contains(&2) || propagates(&fx).iter().any(|(_, s)| s.to == 2);
    assert!(up, "stage 2 is due its propagate: {:?}", run.propagate_due);
    let pushes = |ops: Vec<HostOp>| {
        ops.into_iter()
            .filter(|op| matches!(op, HostOp::Push { .. }))
            .count()
    };
    assert_eq!(
        pushes(host_ops_in(&effects)),
        0,
        "no push: it is the remote's"
    );
    // Ruling R-9: the user's commit is the line's floor; nothing below it is blamed.
    let record = run.stage(1).unwrap();
    assert_eq!(record.floor.as_deref(), Some(user.as_str()));
    assert!(record.merges.is_empty(), "{:?}", record.merges);
    let line = format!(
        "stage 1 (PR #11): adopted {} from {}",
        &user[..7],
        remote_branch(1)
    );
    assert!(logged(&fx, &line), "{:#?}", fx.run().log);
    let later = fx.tick();
    assert_eq!(pushes(host_ops_in(&later)), 0);
}

#[test]
fn rewritten_remote_halts_with_the_exact_text() {
    let mut fx = watched();
    let user = commit(41);
    let (_, effects) = poll_with(&mut fx, view(&user));
    // A `Single` run's stage branch is `integration` (9.1 decision 46).
    let local = format!("anthrex/{RUN_ID}/integration");
    assert_eq!(
        host_ops_in(&effects),
        vec![fetch(1, local, &commit(1), false)]
    );
    for _ in 0..2 {
        let (op, _) = host_op(&fx);
        let remote = FetchOutcome::NotDescendant {
            remote: user.clone(),
        };
        let effects = answer(&mut fx, op, HostResult::Fetched(remote));
        let run = fx.run();
        assert_eq!(run.state, RunState::Halted);
        // Concern 3 of the review: the remote moved; anthrex cannot tell a push from a
        // rewrite, and never forces.
        let moved = format!(
            "remote stage branch {} moved: someone else pushed to it or rewrote it; stage 1 (PR #7) is at {} on origin, which does not contain {}, and anthrex never forces a push",
            remote_branch(1),
            &user[..7],
            &commit(1)[..7]
        );
        assert_eq!(run.halted_reason.as_deref(), Some(moved.as_str()));
        let line = format!(
            "stage 1 (PR #7): origin has {}, which does not contain {}",
            &user[..7],
            &commit(1)[..7]
        );
        assert!(logged(&fx, &line), "{:#?}", fx.run().log);
        assert!(host_ops_in(&effects).is_empty(), "nothing is forced");
        // `run resume` polls again, and halts again while the remote is rewritten.
        let effects = resume(&mut fx);
        assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
        assert_eq!(fx.run().state, RunState::Running);
        let (op, pending) = host_op(&fx);
        assert_eq!(
            pending,
            HostOp::ViewPr {
                stage: 1,
                number: PR
            }
        );
        let at = fx.now;
        fx.send(
            at,
            EventKind::OpDone {
                run_id: RUN_ID.into(),
                op,
                result: OpResult::Host(HostResult::PrViewed(Box::new(view(&user)))),
            },
        );
    }
}

#[test]
fn a_stale_view_is_not_a_rewrite() {
    let mut fx = watched();
    let at = fx.run().delivery.pr(1).unwrap().next_poll_at;
    fx.send(at, EventKind::Tick);
    // A fix merges while the view is out; GitHub's PR head lags behind the branch.
    set_stage_head(fx.run_mut(), 1, &commit(5));
    let effects = super::delivery_watch::view_answer(
        &mut fx,
        at,
        HostResult::PrViewed(Box::new(view(&"f".repeat(40)))),
    );
    let local = format!("anthrex/{RUN_ID}/integration");
    let adopt = fetch(1, local, &commit(5), false);
    assert_eq!(
        host_ops_in(&effects),
        vec![adopt],
        "the adopt goes before the push"
    );
    let (op, _) = host_op(&fx);
    // The branch itself still has what anthrex pushed: nothing was rewritten.
    let outcome = FetchOutcome::NotDescendant { remote: commit(1) };
    let effects = answer(&mut fx, op, HostResult::Fetched(outcome));
    assert_eq!(fx.run().state, RunState::Running);
    let push = HostOp::Push {
        stage: 1,
        sha: commit(5),
    };
    assert_eq!(host_ops_in(&effects), vec![push]);
}

#[test]
fn adopt_waits_for_the_stages_queue_and_the_queue_waits_for_the_adopt() {
    let (mut fx, _) = watched_on(&profile_with("max_writers = 3"));
    let windows = add(&mut fx, &["t9", "t10"]);
    to_queue(&mut fx, "t9", window_of(&windows, "t9"));
    assert_eq!(pending(&fx, "MergeCandidate", Some("t9")).len(), 1);
    // A user's commit is seen while t9's merge is in the stage's queue: the adopt waits.
    let user = commit(41);
    let (_, effects) = poll_with(&mut fx, view(&user));
    assert!(host_ops_in(&effects).is_empty(), "{effects:#?}");
    let stage = fx.run().delivery.stage(1).unwrap();
    assert_eq!(stage.remote_head.as_deref(), Some(user.as_str()));
    // t9 merges: the adopt goes from the new head, and that head's push waits for it.
    merge(&mut fx, "t9", &commit(9));
    let local = format!("anthrex/{RUN_ID}/integration");
    let (op, adopt) = host_op(&fx);
    assert_eq!(adopt, fetch(1, local, &commit(9), false));
    // While the adopt is in flight, the queue starts nothing for the stage.
    to_queue(&mut fx, "t10", window_of(&windows, "t10"));
    assert_eq!(fx.task("t10").state, TaskState::MergeQueue);
    assert!(pending(&fx, "MergeCandidate", Some("t10")).is_empty());
    let adopted = FetchOutcome::Adopted { sha: commit(42) };
    answer(&mut fx, op, HostResult::Fetched(adopted));
    assert_eq!(fx.run().stage_head(1), Some(commit(42).as_str()));
    assert_eq!(pending(&fx, "MergeCandidate", Some("t10")).len(), 1);
}

#[test]
fn a_propagate_waits_for_an_adopt_of_its_stage() {
    let (mut fx, _) = two_stages(true);
    let head2 = fx.run().stage_head(2).unwrap().to_string();
    let user = commit(42);
    let effects = poll_stage(&mut fx, 2, view_of(12, &user));
    let local = format!("anthrex/{RUN_ID}/stage-2");
    assert_eq!(
        host_ops_in(&effects),
        vec![fetch(2, local, &head2, true)],
        "the highest stage of a Multi run moves integration with it"
    );
    let (op, _) = stage_op(&fx, 2);
    // Stage 1 moves: stage 2 is due a propagate, which waits for the adopt.
    set_stage_head(fx.run_mut(), 1, &commit(8));
    fx.tick();
    assert!(fx.run().propagate_due.contains(&2));
    assert!(propagates(&fx).is_empty(), "{:?}", propagates(&fx));
    let adopted = FetchOutcome::Adopted { sha: user.clone() };
    answer(&mut fx, op, HostResult::Fetched(adopted));
    let started: Vec<(u16, String)> = (propagates(&fx).into_iter())
        .map(|(_, s)| (s.to, s.from_head))
        .collect();
    assert_eq!(started, vec![(2, commit(8))]);
}

#[test]
fn push_waits_for_the_stages_queue_then_pushes_the_new_head() {
    let (mut fx, _) = watched_on(&profile_with("max_writers = 3"));
    fx.run_mut().delivery.watching = false;
    let windows = add(&mut fx, &["t9", "t10"]);
    to_queue(&mut fx, "t9", window_of(&windows, "t9"));
    to_queue(&mut fx, "t10", window_of(&windows, "t10"));
    // t9 merges, but t10's merge into the same stage is next in the queue: no push yet.
    merge(&mut fx, "t9", &commit(9));
    assert_eq!(pending(&fx, "MergeCandidate", Some("t10")).len(), 1);
    assert!(host_ops(&fx).is_empty(), "{:?}", host_ops(&fx));
    assert!(host_ops_in(&fx.tick()).is_empty());
    // The queue is quiet: the stage's new head is pushed (tier 2 passed; decision 28).
    merge(&mut fx, "t10", &commit(10));
    let (op, push) = host_op(&fx);
    let want = HostOp::Push {
        stage: 1,
        sha: commit(10),
    };
    assert_eq!(push, want);
    answer(&mut fx, op, HostResult::Pushed(PushOutcome::Pushed));
    assert_eq!(fx.run().delivery.pr(1).unwrap().pushed_head, commit(10));
    let line = format!("stage 1 (PR #7): pushed {}", &commit(10)[..7]);
    assert!(logged(&fx, &line), "{:#?}", fx.run().log);
    assert!(host_ops_in(&fx.tick()).is_empty(), "pushed once");
}

#[test]
fn a_refused_push_holds_its_stage_and_run_resume_pushes_again() {
    let (mut fx, windows) = pr_on(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    green(&mut fx, 1);
    let refuse = |fx: &mut Fixture| {
        let (op, _) = host_op(fx);
        let reason = format!(
            "the remote refused the push of {}: protected",
            remote_branch(1)
        );
        let refused = PushOutcome::Refused {
            reason: reason.clone(),
        };
        answer(fx, op, HostResult::Pushed(refused));
        format!("stage 1 is held: {reason}; anthrex run resume {RUN_ID} pushes it again")
    };
    // Ruling R-11: the opening push is refused; the stage is held, not the run.
    let line = refuse(&mut fx);
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(attention(&fx), vec![line.clone()]);
    let held = proto::DeliveryAlertKind::HostOpHeld;
    assert_eq!(delivery_alerts(&fx), vec![(held, Some(1), line.clone())]);
    assert!(logged(&fx, &line));
    assert!(
        host_ops_in(&fx.tick()).is_empty(),
        "held: nothing is retried"
    );
    let effects = resume(&mut fx);
    let text = format!("run {RUN_ID}: held pushes retry (stage 1)");
    assert_eq!(replies(&effects), vec![Ok(text)]);
    assert!(attention(&fx).is_empty());
    let (op, push) = host_op(&fx);
    assert!(matches!(push, HostOp::Push { stage: 1, .. }), "{push:?}");
    answer(&mut fx, op, HostResult::Pushed(PushOutcome::Pushed));
    let (op, open) = host_op(&fx);
    assert!(matches!(open, HostOp::OpenPr { stage: 1, .. }), "{open:?}");
    answer(&mut fx, op, opened(PR, false));

    // An update refused holds the stage too; its PR is still watched.
    set_stage_head(fx.run_mut(), 1, &commit(5));
    let effects = fx.tick();
    assert_eq!(
        host_ops_in(&effects),
        vec![HostOp::Push {
            stage: 1,
            sha: commit(5)
        }]
    );
    let line = refuse(&mut fx);
    assert_eq!(attention(&fx), vec![line]);
    assert_eq!(fx.run().state, RunState::Running);
    let at = fx.run().delivery.pr(1).unwrap().next_poll_at;
    let effects = fx.send(at, EventKind::Tick);
    let view = HostOp::ViewPr {
        stage: 1,
        number: PR,
    };
    assert_eq!(host_ops_in(&effects), vec![view], "polled, not pushed");
}

#[test]
fn a_paused_stage_starts_no_task() {
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", "stage = 2"),
        doc_task("t3", "stage = 2"),
    ];
    let (mut fx, windows) = stages_on(&profile_with("max_writers = 2"), &tasks);
    pr_mode(fx.run_mut());
    assert_eq!(fx.task("t3").state, TaskState::Queued, "no writer free");
    // Decision 37: stage 1's PR was closed without merging; stage 2 pauses (task
    // M9.2.11 records the pause from the PR's state).
    fx.run_mut().delivery.stages = vec![StageDelivery {
        pr: Some(super::delivery_land::pr_record(7, PrState::Closed)),
        ..StageDelivery::default()
    }];
    fx.tick();
    assert_eq!(fx.run().delivery.stage(2).unwrap().paused_by, Some(1));
    assert_eq!(fx.task("t3").state, TaskState::Pending);
    let t2 = fx.task("t2").state;
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    fx.tick();
    assert_eq!(fx.task("t3").state, TaskState::Pending, "a writer is free");
    assert_eq!(fx.task("t2").state, t2, "a task already running goes on");
    // Reopened: stage 2 resumes.
    let pr = fx.run_mut().delivery.stages[0].pr.as_mut().unwrap();
    pr.state = PrState::Open;
    fx.tick();
    assert_eq!(fx.run().delivery.stage(2).unwrap().paused_by, None);
    assert_ne!(fx.task("t3").state, TaskState::Pending);
}

#[test]
fn opening_waits_when_the_stage_is_no_longer_ready_after_its_push() {
    let (mut fx, windows) = pr_on(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    green(&mut fx, 1);
    let (op, _) = host_op(&fx);
    // Review m1: the stage's queue is busy again by the time the push answers.
    fx.run_mut().delivery.base_sync_due.insert(1, commit(8));
    let effects = answer(&mut fx, op, HostResult::Pushed(PushOutcome::Pushed));
    assert!(host_ops_in(&effects).is_empty(), "{effects:#?}");
    assert!(host_ops(&fx).is_empty());
    // Task M9.2.11: the due base sync runs (the stage holds that base already).
    let (sync, _) = super::delivery_sync::base_sync(&fx);
    fx.done(sync, OpResult::AlreadyHeld);
    let (op, push) = {
        fx.tick();
        host_op(&fx)
    };
    assert!(matches!(push, HostOp::Push { stage: 1, .. }), "{push:?}");
    answer(&mut fx, op, HostResult::Pushed(PushOutcome::UpToDate));
    let (_, open) = host_op(&fx);
    assert!(matches!(open, HostOp::OpenPr { stage: 1, .. }), "{open:?}");
}
