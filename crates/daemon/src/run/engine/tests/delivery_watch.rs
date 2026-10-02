//! Milestone 9.2 task M9.2.8: watching an open stage PR (decision 23). A view is due on
//! the interval, which backs off while nothing changes and resets on a change; a
//! merged or closed PR is polled at the cap; a rate limit doubles the interval base
//! until a success (decision 11). What a view brings is processed against the PR's
//! watermark once, and a restored run given the same view adds nothing. `run watch
//! --off` stops the views, not the pushes. A lost login and a failing op are attention
//! lines; polling goes on. Host answers are scripted `OpResult::Host` values.

use proto::{PrState, RunState};

use super::control::resume;
use super::control_restore::restart;
use super::delivery_open::{answer, green, host_op, host_ops, host_ops_in, open_stage, pr_on};
use super::dispatch::replies;
use super::fixture::*;
use super::full::attention;
use super::merge::{commit, doc_task, merge, to_queue, window_of};
use crate::host::{
    Author, CheckRun, CheckStatus, Conclusion, HostError, IssueComment, Mergeable, PrView, Review,
    ReviewState, ReviewThread, ThreadComment,
};
use crate::run::delivery::ThreadState;
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::delivery::DeliveryRequest;
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{Effect, EventKind, OpResult};
use crate::run::model::Run;

/// The stage PR every test here watches.
pub(super) const PR: u64 = 7;

/// Poll every second, back off to at most 8 (set before the PR opens).
pub(super) fn fast(run: &mut Run) {
    run.delivery.limits.poll_secs = 1;
    run.delivery.limits.poll_max_secs = 8;
    run.delivery.poll_base_secs = 1;
}

/// A `pr`-mode `Single` run of `profile` whose `t1` merged at `commit(1)`, with PR #7
/// open on it and polled every second.
pub(super) fn watched_on(profile: &str) -> (Fixture, Vec<(String, u32)>) {
    let (mut fx, windows) = pr_on(profile, &[doc_task("t1", "")]);
    fast(fx.run_mut());
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    green(&mut fx, 1);
    open_stage(&mut fx, 1, PR);
    (fx, windows)
}

pub(super) fn watched() -> Fixture {
    watched_on(PROFILE).0
}

/// A view of PR #7, open, mergeable, at `head`, with nothing on it.
pub(super) fn view(head: &str) -> PrView {
    PrView {
        number: PR,
        state: PrState::Open,
        merged_at: None,
        merge_commit: None,
        base_ref: "main".into(),
        head_oid: head.into(),
        mergeable: Mergeable::Mergeable,
        review_decision: None,
        checks: Vec::new(),
        reviews: Vec::new(),
        comments: Vec::new(),
        threads: Vec::new(),
    }
}

pub(super) fn author(login: &str) -> Author {
    Author {
        login: login.into(),
        bot: false,
    }
}

pub(super) fn comment(id: u64, body: &str) -> IssueComment {
    IssueComment {
        id,
        author: author("alice"),
        body: body.into(),
    }
}

pub(super) fn thread_comment(id: u64, body: &str) -> ThreadComment {
    ThreadComment {
        id,
        author: author("bob"),
        body: body.into(),
        diff_hunk: "@@ -1 +1 @@".into(),
    }
}

/// A check of the view's head; `None` is still running.
pub(super) fn check(name: &str, conclusion: Option<Conclusion>, run: u64) -> CheckRun {
    CheckRun {
        name: name.into(),
        status: match conclusion {
            Some(_) => CheckStatus::Completed,
            None => CheckStatus::Pending,
        },
        conclusion,
        ci_run: Some(run),
        url: format!("https://github.com/fake/app/actions/runs/{run}/job/1"),
    }
}

/// When stage 1's PR is next due a view.
pub(super) fn next_poll(fx: &Fixture) -> u64 {
    fx.run().delivery.pr(1).unwrap().next_poll_at
}

/// A tick at `at`: whether it emitted a view of PR #7.
pub(super) fn polls(fx: &mut Fixture, at: u64) -> bool {
    let effects = fx.send(at, EventKind::Tick);
    let views = host_ops_in(&effects);
    views.contains(&HostOp::ViewPr {
        stage: 1,
        number: PR,
    })
}

/// Answers the pending view of PR #7 at `at` with `result`.
pub(super) fn view_answer(fx: &mut Fixture, at: u64, result: HostResult) -> Vec<Effect> {
    let want = HostOp::ViewPr {
        stage: 1,
        number: PR,
    };
    // Task M9.2.10: a run-level `Permission` op may be out beside it.
    let views: Vec<_> = (host_ops(fx).into_iter())
        .filter(|(_, o)| *o == want)
        .collect();
    assert_eq!(views.len(), 1, "one pending view: {:#?}", host_ops(fx));
    let op = views[0].0;
    fx.send(
        at,
        EventKind::OpDone {
            run_id: RUN_ID.into(),
            op,
            result: OpResult::Host(result),
        },
    )
}

/// Polls PR #7 when it is due (or takes the view already out) and answers with `v` in
/// the same second.
pub(super) fn poll_with(fx: &mut Fixture, v: PrView) -> (u64, Vec<Effect>) {
    // Task M9.2.9: a red view starts a CI log fetch, which takes the stage's one host
    // op; it is answered first, with an empty log.
    for (op, host) in host_ops(fx) {
        if let HostOp::FailedLogs { .. } = host {
            let path = format!("/tmp/data/delivery/ci-{op}.log");
            let file = crate::host::LogFile {
                path: path.into(),
                bytes: 0,
                truncated: false,
                tail: String::new(),
            };
            answer(fx, op, HostResult::Logs(file));
        }
    }
    let out = (host_ops(fx).iter()).any(|(_, op)| matches!(op, HostOp::ViewPr { .. }));
    let at = if out {
        fx.now
    } else {
        next_poll(fx).max(fx.now)
    };
    assert!(out || polls(fx, at), "a view is due at {at}");
    let effects = view_answer(fx, at, HostResult::PrViewed(Box::new(v)));
    (at, effects)
}

fn logged(fx: &Fixture, text: &str) -> bool {
    fx.run().log.iter().any(|l| l.text == text)
}

#[test]
fn poll_is_due_on_the_interval_and_backs_off_when_unchanged() {
    let mut fx = watched();
    let opened = fx.now;
    assert_eq!(
        next_poll(&fx),
        opened + 1,
        "a new PR is due one interval later"
    );
    assert!(!polls(&mut fx, opened), "not before");
    // The first view changes everything the watermark knew: the interval stays 1.
    let head = commit(1);
    let (at, _) = poll_with(&mut fx, view(&head));
    assert_eq!(next_poll(&fx), at + 1);
    // Unchanged views back off: 2, 4, 8, then the cap (`poll_max_secs` = 8).
    for gap in [2, 4, 8, 8] {
        let due = next_poll(&fx);
        assert!(!polls(&mut fx, due - 1), "not due before {due}");
        let (at, _) = poll_with(&mut fx, view(&head));
        assert_eq!(next_poll(&fx), at + gap, "after {gap}");
        assert_eq!(fx.run().delivery.pr(1).unwrap().last_view_at, Some(at));
    }
    // The cap is `max(poll_max_secs, poll_base_secs)`: a cap below the base is the base.
    fx.run_mut().delivery.limits.poll_max_secs = 0;
    fx.run_mut().delivery.limits.poll_secs = 3;
    fx.run_mut().delivery.poll_base_secs = 3;
    let (at, _) = poll_with(&mut fx, view(&head));
    assert_eq!(next_poll(&fx), at + 3);
    assert!(
        host_ops(&fx).is_empty(),
        "nothing else is due: {:?}",
        host_ops(&fx)
    );
}

#[test]
fn a_change_resets_the_backoff() {
    let mut fx = watched();
    let head = commit(1);
    let mut changes: Vec<PrView> = Vec::new();
    let mut v = view(&head);
    v.comments = vec![comment(5_000_000_001, "please rename")];
    changes.push(v.clone());
    v.mergeable = Mergeable::Conflicting;
    changes.push(v.clone());
    v.checks = vec![check("test", None, 28_000_000_001)];
    changes.push(v.clone());
    v.checks = vec![check("test", Some(Conclusion::Success), 28_000_000_001)];
    changes.push(v.clone());
    let mut current = view(&head);
    poll_with(&mut fx, current.clone());
    for change in changes {
        // Two unchanged views first: the interval is 4.
        poll_with(&mut fx, current.clone());
        let (at, _) = poll_with(&mut fx, current.clone());
        assert_eq!(next_poll(&fx), at + 4, "backed off");
        let (at, _) = poll_with(&mut fx, change.clone());
        assert_eq!(next_poll(&fx), at + 1, "a change resets it: {change:?}");
        assert_eq!(fx.run().delivery.pr(1).unwrap().unchanged_views, 0);
        current = change;
    }
}

#[test]
fn closed_and_merged_prs_are_polled_at_the_cap() {
    let mut fx = watched();
    let mut v = view(&commit(1));
    poll_with(&mut fx, v.clone());
    v.state = PrState::Closed;
    let (at, _) = poll_with(&mut fx, v.clone());
    assert_eq!(fx.run().delivery.pr(1).unwrap().state, PrState::Closed);
    assert_eq!(
        next_poll(&fx),
        at + 8,
        "closed: at the cap, to see a reopen"
    );
    let (at, _) = poll_with(&mut fx, v.clone());
    assert_eq!(next_poll(&fx), at + 8);
    // Reopened on GitHub: a change, so the interval starts again.
    v.state = PrState::Open;
    // Review m3 (task M9.2.7): the base is the host's.
    v.base_ref = "release".into();
    let (at, _) = poll_with(&mut fx, v.clone());
    assert_eq!(fx.run().delivery.pr(1).unwrap().state, PrState::Open);
    assert_eq!(fx.run().delivery.pr(1).unwrap().base, "release");
    assert_eq!(next_poll(&fx), at + 1);
    v.state = PrState::Merged;
    v.merged_at = Some(9_000);
    let (at, _) = poll_with(&mut fx, v);
    assert_eq!(fx.run().delivery.pr(1).unwrap().state, PrState::Merged);
    assert_eq!(next_poll(&fx), at + 8, "merged: at the cap");
}

#[test]
fn rate_limit_doubles_the_interval_until_a_success() {
    let mut fx = watched();
    poll_with(&mut fx, view(&commit(1)));
    let limited = || HostResult::Error(HostError::RateLimited("API rate limit exceeded".into()));
    for base in [2, 4] {
        let at = next_poll(&fx);
        assert!(polls(&mut fx, at));
        view_answer(&mut fx, at, limited());
        assert_eq!(fx.run().delivery.poll_base_secs, base);
        assert_eq!(next_poll(&fx), at + base, "the interval doubles");
        assert_eq!(fx.run().state, RunState::Running);
    }
    let line = "stage 1: view_pr failed: API rate limit exceeded";
    assert!(logged(&fx, line), "{:#?}", fx.run().log);
    // The next success restores the base; this view is unchanged, so it backs off once.
    let (at, _) = poll_with(&mut fx, view(&commit(1)));
    assert_eq!(fx.run().delivery.poll_base_secs, 1);
    assert_eq!(next_poll(&fx), at + 2);
    assert!(attention(&fx).is_empty(), "{:?}", attention(&fx));
    // The base never passes an hour.
    fx.run_mut().delivery.poll_base_secs = 3_000;
    let at = next_poll(&fx);
    assert!(polls(&mut fx, at));
    view_answer(&mut fx, at, limited());
    assert_eq!(fx.run().delivery.poll_base_secs, 3_600);
}

/// A view of PR #7 at `commit(1)` with a conversation comment, a changes-requested
/// review, an approval, a two-comment review thread, and `test` red (`build` green).
fn busy_view() -> PrView {
    let mut v = view(&commit(1));
    v.comments = vec![comment(5_000_000_001, "why this name?")];
    v.reviews = vec![
        Review {
            id: 5_000_000_010,
            author: author("alice"),
            state: ReviewState::ChangesRequested,
            body: "split this function".into(),
        },
        Review {
            id: 5_000_000_011,
            author: author("carol"),
            state: ReviewState::Approved,
            body: String::new(),
        },
    ];
    v.threads = vec![ReviewThread {
        resolved: false,
        path: Some("docs/t1/a.md".into()),
        line: Some(3),
        comments: vec![
            thread_comment(5_000_000_020, "typo here"),
            thread_comment(5_000_000_021, "and here"),
        ],
    }];
    v.checks = vec![
        check("build", Some(Conclusion::Success), 28_000_000_001),
        check("test", Some(Conclusion::Failure), 28_000_000_002),
    ];
    v
}

pub(super) fn keys(fx: &Fixture) -> Vec<String> {
    let mut keys: Vec<String> = (fx.run().delivery.stage(1).unwrap().threads.iter())
        .map(|t| t.key.clone())
        .collect();
    keys.sort();
    keys
}

pub(super) fn count(fx: &Fixture, text: &str) -> usize {
    fx.run().log.iter().filter(|l| l.text == text).count()
}

#[test]
fn watermark_processes_each_comment_and_check_once() {
    let mut fx = watched();
    let c1 = &commit(1)[..7];
    // `test` is red but `lint` still runs: the red set is not complete, nothing is
    // recorded for CI yet (ruling R-5).
    let mut running = view(&commit(1));
    running.checks = vec![
        check("test", Some(Conclusion::Failure), 28_000_000_002),
        check("lint", None, 28_000_000_003),
    ];
    poll_with(&mut fx, running);
    assert!(fx.run().delivery.pr(1).unwrap().watermark.ci.is_empty());
    let (at, _) = poll_with(&mut fx, busy_view());
    // Each new item is a thread (decision 4's keys): the comment, the changes-requested
    // review's body and the review thread (keyed by its first comment). An approval
    // makes none.
    assert_eq!(keys(&fx), ["c5000000001", "r5000000010", "t5000000020"]);
    let stage = fx.run().delivery.stage(1).unwrap();
    let t = (stage.threads.iter())
        .find(|t| t.key == "t5000000020")
        .unwrap();
    assert_eq!(
        (t.author.as_str(), t.path.as_deref(), t.line),
        ("bob", Some("docs/t1/a.md"), Some(3))
    );
    assert_eq!((t.last_comment_id, t.seen_at), (5_000_000_021, at));
    assert_eq!(t.state, ThreadState::New);
    assert_eq!(t.diff_hunk, "@@ -1 +1 @@");
    let wm = &fx.run().delivery.pr(1).unwrap().watermark;
    assert_eq!(
        (wm.issue_comment, wm.review, wm.review_comment),
        (5_000_000_001, 5_000_000_011, 5_000_000_021)
    );
    assert_eq!(wm.ci[&commit(1)].failing, ["test"]);
    assert_eq!(wm.ci[&commit(1)].at, at);
    let red = format!("stage 1 (PR #7): CI red at {c1}: test");
    assert_eq!(count(&fx, &red), 1, "{:#?}", fx.run().log);
    let new = "stage 1 (PR #7): new threads c5000000001, r5000000010, t5000000020";
    assert_eq!(count(&fx, new), 1, "{:#?}", fx.run().log);
    // The PR's checks are the view's (decision 41).
    let checks = &fx.run().delivery.pr(1).unwrap().checks;
    let names: Vec<(&str, proto::CiState)> =
        checks.iter().map(|c| (c.name.as_str(), c.state)).collect();
    assert_eq!(
        names,
        [
            ("build", proto::CiState::Green),
            ("test", proto::CiState::Red)
        ]
    );

    // The same view again: nothing new.
    let before = fx.run().delivery.clone();
    poll_with(&mut fx, busy_view());
    let after = &fx.run().delivery;
    assert_eq!(
        after.stage(1).unwrap().threads,
        before.stage(1).unwrap().threads
    );
    assert_eq!(
        after.pr(1).unwrap().watermark,
        before.pr(1).unwrap().watermark
    );
    assert_eq!(count(&fx, &red), 1);
    assert_eq!(count(&fx, new), 1);

    // A second comment and a reply in the thread; `lint` still running, so the red set
    // is not complete and CI is not processed again.
    let mut v = busy_view();
    v.comments.push(comment(5_000_000_002, "and this?"));
    (v.threads[0].comments).push(thread_comment(5_000_000_030, "still a typo"));
    v.checks.push(check("lint", None, 28_000_000_003));
    poll_with(&mut fx, v.clone());
    assert_eq!(
        keys(&fx),
        ["c5000000001", "c5000000002", "r5000000010", "t5000000020"]
    );
    let stage = fx.run().delivery.stage(1).unwrap();
    let t = (stage.threads.iter())
        .find(|t| t.key == "t5000000020")
        .unwrap();
    assert_eq!((t.last_comment_id, t.seen_at), (5_000_000_030, at));
    assert_eq!(count(&fx, "stage 1 (PR #7): new threads c5000000002"), 1);
    assert_eq!(
        count(&fx, &red),
        1,
        "a pending check: the set is not complete"
    );

    // `lint` fails too: a new set of failing checks on the head is processed once.
    v.checks[2] = check("lint", Some(Conclusion::Failure), 28_000_000_003);
    poll_with(&mut fx, v.clone());
    poll_with(&mut fx, v);
    let both = format!("stage 1 (PR #7): CI red at {c1}: lint, test");
    assert_eq!(count(&fx, &both), 1);
    let wm = &fx.run().delivery.pr(1).unwrap().watermark;
    assert_eq!(wm.ci[&commit(1)].failing, ["lint", "test"]);
}

#[test]
fn restore_replays_no_event_twice() {
    let mut fx = watched();
    // Fix round m9: the run has an orchestrator, so a wake note would be kept.
    super::bisect::with_orchestrator(&mut fx);
    poll_with(&mut fx, busy_view());
    let before = fx.run().clone();
    // Persisted as `run.json` is, then the daemon restarts: the run comes back paused.
    let json = serde_json::to_string(fx.run()).unwrap();
    *fx.run_mut() = serde_json::from_str(&json).unwrap();
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().state, RunState::Paused);
    let effects = resume(&mut fx);
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
    let notes = |r: &Run| {
        let o = r.orch.orchestrator.as_ref().expect("an orchestrator");
        (o.notes.clone(), o.last_note_seq)
    };
    // The restart's own note is there; the replayed view must add none.
    let resumed = notes(fx.run());
    let from = fx.log.len();
    poll_with(&mut fx, busy_view());
    let run = fx.run();
    assert_eq!(run.tasks.len(), before.tasks.len(), "no task");
    assert_eq!(
        run.delivery.stage(1).unwrap().threads,
        before.delivery.stage(1).unwrap().threads
    );
    assert_eq!(
        run.delivery.pr(1).unwrap().watermark,
        before.delivery.pr(1).unwrap().watermark
    );
    assert_eq!(notes(run), resumed, "no wake note");
    let replies_sent = host_ops_in(&fx.log[from..])
        .into_iter()
        .filter(|op| matches!(op, HostOp::Reply { .. }))
        .count();
    assert_eq!(replies_sent, 0, "no reply");
    let red = format!("stage 1 (PR #7): CI red at {}: test", &commit(1)[..7]);
    assert_eq!(count(&fx, &red), 1);
}

fn watch_request(fx: &mut Fixture, on: bool) -> Vec<Effect> {
    let reply = fx.reply();
    let request = DeliveryRequest::Watch {
        reply,
        run_id: RUN_ID.into(),
        on,
    };
    fx.next(EventKind::Delivery(request))
}

#[test]
fn watch_off_emits_no_view_and_on_polls_at_once() {
    let mut fx = watched();
    let effects = watch_request(&mut fx, false);
    assert!(replies(&effects)[0].is_ok());
    let due = next_poll(&fx);
    assert!(!polls(&mut fx, due), "no view while watching is off");
    assert!(!polls(&mut fx, due + 100));
    // A fix already merged is still pushed.
    set_stage_head(fx.run_mut(), 1, &commit(5));
    let effects = fx.tick();
    let push = HostOp::Push {
        stage: 1,
        sha: commit(5),
    };
    assert_eq!(host_ops_in(&effects), vec![push]);
    let (op, _) = host_op(&fx);
    answer(
        &mut fx,
        op,
        HostResult::Pushed(crate::host::PushOutcome::Pushed),
    );
    assert_eq!(fx.run().delivery.pr(1).unwrap().pushed_head, commit(5));
    // On: every open PR is due at once.
    let effects = watch_request(&mut fx, true);
    let view = HostOp::ViewPr {
        stage: 1,
        number: PR,
    };
    assert_eq!(host_ops_in(&effects), vec![view]);
}

#[test]
fn auth_lost_is_an_attention_line_and_polling_continues() {
    let mut fx = watched();
    let line = "gh is no longer logged in to github.com; run gh auth login";
    let lost = || {
        HostResult::Error(HostError::Auth(
            "To get started with GitHub CLI, please run:  gh auth login".into(),
        ))
    };
    for gap in [2, 4] {
        let at = next_poll(&fx);
        assert!(polls(&mut fx, at));
        view_answer(&mut fx, at, lost());
        assert_eq!(attention(&fx), vec![line.to_string()]);
        assert_eq!(fx.run().state, RunState::Running, "nothing halts");
        assert_eq!(next_poll(&fx), at + gap, "polled again, backed off");
    }
    // Logged in again: the next success clears the line.
    poll_with(&mut fx, view(&commit(1)));
    assert!(attention(&fx).is_empty(), "{:?}", attention(&fx));
}

#[test]
fn five_failures_raise_an_attention_line() {
    let mut fx = watched();
    let line = "PR #7: view_pr keeps failing: HTTP 502";
    for k in 1..=5u32 {
        let at = next_poll(&fx).max(fx.now);
        assert!(polls(&mut fx, at));
        view_answer(
            &mut fx,
            at,
            HostResult::Error(HostError::Failed("HTTP 502".into())),
        );
        assert_eq!(fx.run().delivery.failures.get("1/view_pr"), Some(&k));
        let shown = attention(&fx).contains(&line.to_string());
        assert_eq!(shown, k == 5, "after {k} failures: {:?}", attention(&fx));
    }
    assert!(logged(&fx, "stage 1: view_pr failed: HTTP 502"));
    assert_eq!(fx.run().state, RunState::Running);
    poll_with(&mut fx, view(&commit(1)));
    assert!(attention(&fx).is_empty(), "{:?}", attention(&fx));
    assert_eq!(fx.run().delivery.failures.get("1/view_pr"), None);
}
