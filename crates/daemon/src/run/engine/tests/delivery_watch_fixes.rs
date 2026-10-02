//! Milestone 9.2 task M9.2.8, fix round 1. A review's ids are given when it is started,
//! not when it is submitted, so newness is decided by key: a review not yet processed,
//! a thread comment not yet in its thread (I1, amending decision 23); every comment of
//! a thread is kept (m5). A full page is an attention line (I2). An adopt re-issued
//! after a restart finds the local ref already moved and counts as adopted; a ref moved
//! by anything else halts (I3). A held stage holds the pushes above it (m1); only `gh`
//! answers clear a lost login (m2); a push no longer due drops its failures (m3); host
//! text is one-lined and cut (m4); an answer for another PR is logged (m7); a failed
//! fetch waits for its retry and the failing line is never repeated (m9).

use proto::RunState;

use super::control::resume;
use super::control_restore::restart;
use super::delivery_open::{answer, green, host_op, host_ops, pr_on, remote_branch};
use super::delivery_watch::{
    PR, comment, count, keys, next_poll, poll_with, view, view_answer, watched,
};
use super::delivery_watch_adopt::two_stages;
use super::dispatch::replies;
use super::fixture::*;
use super::full::attention;
use super::merge::{commit, doc_task, merge, to_queue, window_of};
use crate::host::gh::THREADS_QUERY;
use crate::host::{
    Author, FetchOutcome, HostError, PushOutcome, Review, ReviewState, ReviewThread, THREAD_PAGE,
    ThreadComment, VIEW_PAGE,
};
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::EventKind;
use crate::run::engine::stages::set_stage_head;

fn author(login: &str) -> Author {
    Author {
        login: login.into(),
        bot: false,
    }
}

fn review(id: u64, login: &str, state: ReviewState, body: &str) -> Review {
    Review {
        id,
        author: author(login),
        state,
        body: body.into(),
    }
}

fn tc(id: u64, login: &str) -> ThreadComment {
    ThreadComment {
        id,
        author: author(login),
        body: format!("comment {id}"),
        diff_hunk: "@@ -1 +1 @@".into(),
    }
}

fn thread(comments: Vec<ThreadComment>) -> ReviewThread {
    ReviewThread {
        resolved: false,
        path: Some("docs/t1/a.md".into()),
        line: Some(1),
        comments,
    }
}

/// The pending adopt fetches.
fn fetches(fx: &Fixture) -> usize {
    (host_ops(fx).into_iter())
        .filter(|(_, op)| matches!(op, HostOp::Fetch { .. }))
        .count()
}

fn logged(fx: &Fixture, text: &str) -> bool {
    fx.run().log.iter().any(|l| l.text == text)
}

/// The run through `run.json`, a daemon restart and `run resume`.
fn restarted(fx: &mut Fixture) {
    let json = serde_json::to_string(fx.run()).unwrap();
    *fx.run_mut() = serde_json::from_str(&json).unwrap();
    restart(fx, Vec::new());
    assert!(replies(&resume(fx))[0].is_ok());
}

/// The reviewer's scenario (I1): Alice starts a review (R1 = 10, its inline comment
/// C1 = 20); Bob comments (C2 = 21) and approves (R2 = 11); a view; Alice submits with
/// changes requested. Her review and thread are recorded once, never again.
#[test]
fn a_review_submitted_after_a_later_item_is_recorded_once() {
    let mut fx = watched();
    let mut v = view(&commit(1));
    v.reviews = vec![
        review(10, "alice", ReviewState::Pending, ""),
        review(11, "bob", ReviewState::Approved, ""),
    ];
    v.threads = vec![thread(vec![tc(21, "bob")])];
    poll_with(&mut fx, v.clone());
    assert_eq!(keys(&fx), ["t21"]);
    let wm = &fx.run().delivery.pr(1).unwrap().watermark;
    assert_eq!((wm.review, wm.review_comment), (11, 21));
    assert!(wm.reviews_seen.contains(&11), "processed");
    assert!(
        !wm.reviews_seen.contains(&10),
        "a pending review is not yet processed"
    );

    v.reviews[0] = review(10, "alice", ReviewState::ChangesRequested, "split it");
    v.threads.insert(0, thread(vec![tc(20, "alice")]));
    poll_with(&mut fx, v.clone());
    assert_eq!(keys(&fx), ["r10", "t20", "t21"]);
    let new = "stage 1 (PR #7): new threads r10, t20";
    assert_eq!(count(&fx, new), 1, "{:#?}", fx.run().log);

    poll_with(&mut fx, v.clone());
    restarted(&mut fx);
    poll_with(&mut fx, v);
    assert_eq!(keys(&fx), ["r10", "t20", "t21"]);
    assert_eq!(count(&fx, new), 1, "never twice, across a restart");
    let wm = &fx.run().delivery.pr(1).unwrap().watermark;
    assert!(wm.reviews_seen.contains(&10));
}

/// m5 and I1 inside a thread: a reply submitted after a later one is still read, and
/// each comment of a thread is kept, once.
#[test]
fn every_fresh_comment_of_a_thread_is_kept_once() {
    let mut fx = watched();
    let mut v = view(&commit(1));
    v.threads = vec![thread(vec![tc(21, "bob"), tc(23, "bob")])];
    poll_with(&mut fx, v.clone());
    v.threads = vec![thread(vec![tc(21, "bob"), tc(22, "alice"), tc(23, "bob")])];
    poll_with(&mut fx, v.clone());
    poll_with(&mut fx, v);
    let stage = fx.run().delivery.stage(1).unwrap();
    assert_eq!(stage.threads.len(), 1);
    let t = &stage.threads[0];
    let seen: Vec<(u64, &str)> = (t.comments.iter())
        .map(|c| (c.id, c.author.as_str()))
        .collect();
    assert_eq!(seen, [(21, "bob"), (23, "bob"), (22, "alice")]);
    assert_eq!(t.comments[2].text, "comment 22");
    assert_eq!(t.last_comment_id, 23);
}

#[test]
fn a_full_page_is_an_attention_line_until_a_view_is_not_full() {
    // The page sizes are THREADS_QUERY's.
    for part in [
        format!("reviewThreads(last: {VIEW_PAGE})"),
        format!("reviews(last: {VIEW_PAGE})"),
        format!("comments(last: {VIEW_PAGE})"),
        format!("comments(first: {THREAD_PAGE})"),
    ] {
        assert!(THREADS_QUERY.contains(&part), "{part}");
    }
    let mut fx = watched();
    let mut v = view(&commit(1));
    v.comments = (1..=100).map(|i| comment(5_000_000_000 + i, "x")).collect();
    let long: Vec<ThreadComment> = (0..50).map(|i| tc(6_000_000_000 + i, "bob")).collect();
    v.threads = vec![thread(long)];
    poll_with(&mut fx, v.clone());
    let comments =
        "PR #7: more than 100 new comments since the last view; the older ones were not read";
    let long = "PR #7: thread t6000000000 has 50 comments or more; the newer ones were not read";
    assert_eq!(attention(&fx), vec![comments.to_string(), long.to_string()]);
    assert_eq!(count(&fx, &format!("stage 1 (PR #7): {comments}")), 1);
    // The next page reaches what was read: that line clears; the thread is still full.
    v.comments = (51..=150)
        .map(|i| comment(5_000_000_000 + i, "x"))
        .collect();
    poll_with(&mut fx, v.clone());
    assert_eq!(attention(&fx), vec![long.to_string()]);
    v.reviews = (1..=100)
        .map(|i| review(7_000_000_000 + i, "carol", ReviewState::Commented, ""))
        .collect();
    poll_with(&mut fx, v.clone());
    let reviews =
        "PR #7: more than 100 new reviews since the last view; the older ones were not read";
    assert!(
        attention(&fx).contains(&reviews.to_string()),
        "{:?}",
        attention(&fx)
    );
    poll_with(&mut fx, v);
    assert_eq!(
        count(&fx, &format!("stage 1 (PR #7): {long}")),
        1,
        "logged once"
    );
}

/// A pending adopt fetch of `user`, from a watched run's view.
fn adopting(user: &str) -> Fixture {
    let mut fx = watched();
    poll_with(&mut fx, view(user));
    let (_, op) = host_op(&fx);
    assert!(matches!(op, HostOp::Fetch { .. }), "{op:?}");
    fx
}

fn fetched(fx: &mut Fixture, outcome: FetchOutcome) {
    let (op, _) = host_op(fx);
    answer(fx, op, HostResult::Fetched(outcome));
}

#[test]
fn local_moved_is_an_adopt_when_it_is_the_remote_head_and_a_halt_otherwise() {
    // Re-issued after a restart: the local ref is already at the remote head.
    let user = commit(41);
    let mut fx = adopting(&user);
    fetched(
        &mut fx,
        FetchOutcome::LocalMoved {
            local: user.clone(),
        },
    );
    let run = fx.run();
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.stage_head(1), Some(user.as_str()));
    assert_eq!(run.delivery.pr(1).unwrap().pushed_head, user);
    assert_eq!(run.delivery.stage(1).unwrap().remote_head, None);
    let line = format!(
        "stage 1 (PR #7): adopted {} from {}",
        &user[..7],
        remote_branch(1)
    );
    assert!(logged(&fx, &line));
    fx.tick();
    let fetches = host_ops(&fx)
        .into_iter()
        .filter(|(_, op)| matches!(op, HostOp::Fetch { .. }));
    assert_eq!(fetches.count(), 0, "adopted once, not again every tick");

    // Moved by something else: a retryable halt naming the ref; a view comes first.
    for (local, at) in [
        (commit(9), format!("at {}", &commit(9)[..7])),
        (String::new(), "gone".into()),
    ] {
        let mut fx = adopting(&user);
        fetched(&mut fx, FetchOutcome::LocalMoved { local });
        let run = fx.run();
        assert_eq!(run.state, RunState::Halted);
        let reason = format!(
            "stage 1's branch anthrex/{RUN_ID}/integration moved outside anthrex: it is {at}, where anthrex had {}; check it, then anthrex run resume {RUN_ID}",
            &commit(1)[..7]
        );
        assert_eq!(run.halted_reason.as_deref(), Some(reason.as_str()));
        assert!(run.halt_retryable);
        assert_eq!(run.delivery.stage(1).unwrap().remote_head, None);
        assert!(replies(&resume(&mut fx))[0].is_ok());
        let (_, op) = host_op(&fx);
        assert_eq!(
            op,
            HostOp::ViewPr {
                stage: 1,
                number: PR
            }
        );
    }
}

#[test]
fn a_missing_remote_branch_drops_the_adopt() {
    let user = commit(41);
    let mut fx = adopting(&user);
    fetched(&mut fx, FetchOutcome::Missing);
    let run = fx.run();
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.delivery.stage(1).unwrap().remote_head, None);
    assert_eq!(run.stage_head(1), Some(commit(1).as_str()), "nothing moved");
    let line = format!(
        "stage 1 (PR #7): {} is gone from the remote",
        remote_branch(1)
    );
    assert!(logged(&fx, &line));
    fx.tick();
    assert_eq!(fetches(&fx), 0, "{:?}", host_ops(&fx));
}

#[test]
fn a_held_lower_stage_holds_the_push_above() {
    let (mut fx, _) = two_stages(true);
    fx.run_mut().delivery.watching = false;
    fx.run_mut().delivery.stages[0].held = Some("the remote refused".into());
    set_stage_head(fx.run_mut(), 2, &commit(7));
    fx.tick();
    let pushes = |fx: &Fixture| {
        host_ops(fx)
            .into_iter()
            .filter(|(_, op)| matches!(op, HostOp::Push { .. }))
            .map(|(_, op)| op)
            .collect::<Vec<_>>()
    };
    assert!(
        pushes(&fx).is_empty(),
        "stage 1's unpushed work would ride in PR #12"
    );
    fx.run_mut().delivery.stages[0].held = None;
    fx.tick();
    let push = HostOp::Push {
        stage: 2,
        sha: commit(7),
    };
    assert_eq!(pushes(&fx), vec![push]);
}

#[test]
fn only_a_gh_answer_clears_a_lost_login() {
    let mut fx = watched();
    let at = next_poll(&fx);
    assert!(super::delivery_watch::polls(&mut fx, at));
    let lost = HostError::Auth("gh auth login".into());
    view_answer(&mut fx, at, HostResult::Error(lost));
    let line = "gh is no longer logged in to github.com; run gh auth login".to_string();
    assert_eq!(attention(&fx), vec![line.clone()]);
    // A push is git's: it says nothing of gh's login.
    set_stage_head(fx.run_mut(), 1, &commit(5));
    fx.send(at, EventKind::Tick);
    let (op, push) = host_op(&fx);
    assert!(matches!(push, HostOp::Push { .. }), "{push:?}");
    answer(&mut fx, op, HostResult::Pushed(PushOutcome::Pushed));
    assert_eq!(attention(&fx), vec![line]);
    poll_with(&mut fx, view(&commit(5)));
    assert!(attention(&fx).is_empty(), "{:?}", attention(&fx));
}

#[test]
fn a_push_no_longer_due_drops_its_failures_and_the_line_is_never_repeated() {
    let mut fx = watched();
    fx.run_mut().delivery.watching = false;
    set_stage_head(fx.run_mut(), 1, &commit(5));
    let line = "PR #7: push keeps failing: boom".to_string();
    for k in 1..=7u32 {
        fx.tick();
        let (op, push) = host_op(&fx);
        assert!(matches!(push, HostOp::Push { .. }), "{push:?}");
        answer(
            &mut fx,
            op,
            HostResult::Error(HostError::Failed("boom".into())),
        );
        assert_eq!(fx.run().delivery.failures.get("1/push"), Some(&k));
        let lines = attention(&fx);
        let shown = lines.iter().filter(|l| **l == line).count();
        assert_eq!(shown, usize::from(k >= 5), "after {k}: {lines:?}");
    }
    // The stage is back at what PR #7 has: the push is not due, so its failures go.
    set_stage_head(fx.run_mut(), 1, &commit(1));
    fx.tick();
    assert!(attention(&fx).is_empty(), "{:?}", attention(&fx));
    assert_eq!(fx.run().delivery.failures.get("1/push"), None);
}

#[test]
fn a_failed_fetch_waits_for_its_retry() {
    let mut fx = adopting(&commit(41));
    let wait = fx.run().delivery.limits.poll_secs;
    // No view gets in the way: only the fetch's retry is watched here.
    fx.run_mut().delivery.watching = false;
    let (op, _) = host_op(&fx);
    answer(
        &mut fx,
        op,
        HostResult::Error(HostError::Failed("network".into())),
    );
    let failed_at = fx.now;
    assert_eq!(fx.run().delivery.failures.get("1/fetch"), Some(&1));
    assert_eq!(
        fx.run().delivery.stage(1).unwrap().retry_at,
        Some(failed_at + wait)
    );
    fx.send(failed_at, EventKind::Tick);
    assert_eq!(fetches(&fx), 0, "not before its retry");
    fx.send(failed_at + wait, EventKind::Tick);
    assert_eq!(fetches(&fx), 1, "{:?}", host_ops(&fx));
}

#[test]
fn host_text_is_one_lined_and_cut() {
    let (mut fx, windows) = pr_on(PROFILE, &[doc_task("t1", "")]);
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    green(&mut fx, 1);
    let (op, _) = host_op(&fx);
    let reason = format!("the remote refused\nthe push: {}", "x".repeat(1_000));
    answer(
        &mut fx,
        op,
        HostResult::Pushed(PushOutcome::Refused { reason }),
    );
    let held = fx.run().delivery.stage(1).unwrap().held.clone().unwrap();
    assert!(!held.contains('\n'), "{held}");
    assert_eq!(held.chars().count(), 300);

    let mut fx = watched();
    let at = next_poll(&fx);
    assert!(super::delivery_watch::polls(&mut fx, at));
    let error = HostError::Failed("bad\ngateway".into());
    view_answer(&mut fx, at, HostResult::Error(error));
    assert!(
        logged(&fx, "stage 1: view_pr failed: bad gateway"),
        "{:#?}",
        fx.run().log
    );
}

#[test]
fn an_answer_for_another_pr_is_logged_and_polled_again() {
    let mut fx = watched();
    let mut v = view(&commit(1));
    v.number = 99;
    let (at, _) = poll_with(&mut fx, v);
    assert!(logged(
        &fx,
        "stage 1 (PR #7): the host answered for PR #99; ignored"
    ));
    let pr = fx.run().delivery.pr(1).unwrap();
    assert!(pr.next_poll_at > at, "not due again at once");
    assert_eq!(pr.watermark.head, "", "nothing of it was taken");
}
