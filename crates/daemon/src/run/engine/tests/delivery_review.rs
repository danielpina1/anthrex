//! Milestone 9.2 task M9.2.10: which review comments count (decision 29), and how
//! they are batched (decision 31). Only a writer's or a listed reviewer's comment
//! counts, asked of GitHub once per login (`Permission`) and cached; bots, anthrex's
//! own replies (known by id, never by text) and resolved threads never count; a batch
//! closes after `review_batch_secs` of quiet and wakes a live orchestrator once. Host
//! answers are scripted `OpResult::Host` values.

use proto::{PlanEdit, RunState};

use super::delivery_open::{answer, host_ops, host_ops_in};
use super::delivery_watch::{PR, author, poll_with, view, watched};
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::full::attention;
use super::merge::commit;
use crate::host::{
    Author, IssueComment, RepoPermission, Review, ReviewState, ReviewThread, ThreadComment,
};
use crate::run::delivery::ThreadState;
use crate::run::delivery::ops::{HostOp, HostResult};

/// A watched PR #7 whose review batches close after one quiet second.
pub(super) fn reviewed() -> Fixture {
    let mut fx = watched();
    fx.run_mut().delivery.limits.review_batch_secs = 1;
    fx
}

/// A conversation comment `id` by `login`.
pub(super) fn said(id: u64, login: &str, body: &str) -> IssueComment {
    IssueComment {
        id,
        author: author(login),
        body: body.into(),
    }
}

/// A review-thread comment `id` by `login` on `docs/t1/a.md`'s hunk.
pub(super) fn noted(id: u64, login: &str, body: &str) -> ThreadComment {
    ThreadComment {
        id,
        author: author(login),
        body: body.into(),
        diff_hunk: "@@ -1 +1 @@\n-old\n+new".into(),
    }
}

/// An unresolved review thread on `path` line 3.
pub(super) fn on(path: &str, comments: Vec<ThreadComment>) -> ReviewThread {
    ReviewThread {
        resolved: false,
        path: Some(path.into()),
        line: Some(3),
        comments,
    }
}

/// The users of the pending `Permission` ops.
pub(super) fn asked(fx: &Fixture) -> Vec<String> {
    (host_ops(fx).into_iter())
        .filter_map(|(_, op)| match op {
            HostOp::Permission { user } => Some(user),
            _ => None,
        })
        .collect()
}

/// Answers the one pending `Permission` op, which must ask about `login`.
pub(super) fn grant(fx: &mut Fixture, login: &str, permission: RepoPermission) {
    let ops: Vec<_> = (host_ops(fx).into_iter())
        .filter(|(_, op)| matches!(op, HostOp::Permission { .. }))
        .collect();
    assert_eq!(ops.len(), 1, "one Permission op: {ops:?}");
    assert_eq!(ops[0].1, HostOp::Permission { user: login.into() });
    let user = login.to_string();
    answer(fx, ops[0].0, HostResult::Permission { user, permission });
}

/// Stage 1's thread `key`'s state.
pub(super) fn state(fx: &Fixture, key: &str) -> ThreadState {
    let stage = fx.run().delivery.stage(1).unwrap();
    let t = stage.threads.iter().find(|t| t.key == key);
    t.unwrap_or_else(|| panic!("no thread {key}: {:#?}", stage.threads))
        .state
        .clone()
}

pub(super) fn counted(fx: &Fixture, key: &str) -> bool {
    let stage = fx.run().delivery.stage(1).unwrap();
    stage.threads.iter().any(|t| t.key == key && t.counted)
}

pub(super) fn ignored(reason: &str) -> ThreadState {
    ThreadState::Ignored {
        reason: reason.into(),
    }
}

pub(super) fn logged(fx: &Fixture, text: &str) -> bool {
    fx.run().log.iter().any(|l| l.text == text)
}

/// Every `Permission` op the run ever emitted, by user.
fn every_permission(fx: &Fixture) -> Vec<String> {
    (host_ops_in(&fx.log).into_iter())
        .filter_map(|op| match op {
            HostOp::Permission { user } => Some(user),
            _ => None,
        })
        .collect()
}

#[test]
fn only_writers_and_listed_reviewers_count() {
    let mut fx = reviewed();
    fx.run_mut().delivery.limits.reviewers = vec!["Dave".into()];
    let mut v = view(&commit(1));
    v.comments = vec![
        said(1, "alice", "a"),
        said(2, "carol", "c"),
        said(3, "dave", "d"),
        said(4, "erin", "e"),
        said(5, "alice", "a again"),
        // A login the allow-list would refuse: never asked about, never a halt.
        said(6, "-x", "x"),
    ];
    poll_with(&mut fx, v);
    // A listed reviewer counts at once (any case); one op, for the first unknown login.
    assert!(counted(&fx, "c3"));
    assert_eq!(asked(&fx), vec!["alice"]);
    assert_eq!(state(&fx, "c6"), ignored("no write access"));
    assert_eq!(fx.run().state, RunState::Running);
    grant(&mut fx, "alice", RepoPermission::Write);
    assert!(counted(&fx, "c1") && counted(&fx, "c5"), "cached for both");
    grant(&mut fx, "carol", RepoPermission::Read);
    // GitHub's 404 (not a collaborator) is `RepoPermission::None`.
    grant(&mut fx, "erin", RepoPermission::None);
    assert!(asked(&fx).is_empty());
    assert_eq!(state(&fx, "c2"), ignored("no write access"));
    assert_eq!(state(&fx, "c4"), ignored("no write access"));
    let line = "stage 1 (PR #7): ignored a comment by @carol: no write access";
    assert!(logged(&fx, line), "{:#?}", fx.run().log);
    // A later comment by a login already asked about is decided from the cache.
    let mut v = view(&commit(1));
    v.comments = vec![said(7, "alice", "b"), said(8, "carol", "c2")];
    poll_with(&mut fx, v);
    assert!(counted(&fx, "c7"));
    assert_eq!(state(&fx, "c8"), ignored("no write access"));
    assert_eq!(every_permission(&fx), vec!["alice", "carol", "erin"]);
    let cache = &fx.run().delivery.permissions;
    assert_eq!(cache.get("alice"), Some(&RepoPermission::Write));
    assert!(!cache.contains_key("dave"));
}

#[test]
fn bots_and_the_anthrex_marker_are_ignored_but_the_users_own_comments_count() {
    let mut fx = reviewed();
    fx.run_mut().delivery.limits.reviewers = vec!["tester".into(), "mallory".into()];
    let pasted = format!("<!-- anthrex:reply {RUN_ID} {PR}:c1 1a2b3c4 -->");
    let mut v = view(&commit(1));
    let bot = Author {
        login: "renovate".into(),
        bot: true,
    };
    v.comments = vec![
        said(1, "dependabot[bot]", "bump"),
        IssueComment {
            id: 2,
            author: bot,
            body: "deps".into(),
        },
        // A reviewer pasting anthrex's marker is still a reviewer.
        said(3, "mallory", &format!("look\n\n{pasted}")),
        // The user reviewing their own stack (anthrex posts as the same login).
        said(4, "tester", "please rename"),
    ];
    poll_with(&mut fx, v);
    assert_eq!(state(&fx, "c1"), ignored("a bot"));
    assert_eq!(state(&fx, "c2"), ignored("a bot"));
    assert!(counted(&fx, "c3"), "a pasted marker proves nothing");
    assert!(counted(&fx, "c4"), "the user's own comment counts");
    let line = "stage 1 (PR #7): ignored a comment by @dependabot[bot]: a bot";
    assert!(logged(&fx, line));
    // anthrex's own reply: known by the id its post was answered with.
    let reply = PlanEdit::ReplyComment {
        pr: PR,
        thread: "c4".into(),
        body: "Done.".into(),
    };
    assert!(replies(&edit(&mut fx, vec![reply]))[0].is_ok());
    let (op, marker) = match &host_ops(&fx)[..] {
        [(op, HostOp::Reply { marker, .. })] => (*op, marker.clone()),
        other => panic!("one reply: {other:?}"),
    };
    answer(&mut fx, op, HostResult::Replied { comment_id: 9 });
    let mut v = view(&commit(1));
    v.comments = vec![said(9, "tester", &format!("Done.\n\n{marker}"))];
    poll_with(&mut fx, v);
    assert_eq!(state(&fx, "c9"), ignored("anthrex's own"));
    let line = "stage 1 (PR #7): ignored a comment by @tester: anthrex's own";
    assert!(logged(&fx, line));
    assert!(
        asked(&fx).is_empty(),
        "nothing asked about a bot or anthrex"
    );
    let text = &fx.run().delivery.stage(1).unwrap().threads;
    let own = text.iter().find(|t| t.key == "c9").unwrap();
    assert!(own.text.is_empty(), "what does not count keeps no text");
}

#[test]
fn resolved_threads_and_empty_reviews_do_not_count() {
    let mut fx = reviewed();
    fx.run_mut().delivery.limits.reviewers = vec!["alice".into()];
    let review = |id, state, body: &str| Review {
        id,
        author: author("alice"),
        state,
        body: body.into(),
    };
    let mut v = view(&commit(1));
    v.reviews = vec![
        review(10, ReviewState::Approved, ""),
        review(11, ReviewState::Commented, ""),
        review(12, ReviewState::ChangesRequested, "  \n"),
        review(13, ReviewState::ChangesRequested, "Please split this."),
    ];
    let mut resolved = on("docs/t1/a.md", vec![noted(20, "alice", "nit")]);
    resolved.resolved = true;
    v.threads = vec![
        resolved,
        on("docs/t1/b.md", vec![noted(30, "alice", "why?")]),
    ];
    poll_with(&mut fx, v);
    let keys: Vec<String> = (fx.run().delivery.stage(1).unwrap().threads.iter())
        .map(|t| t.key.clone())
        .collect();
    assert_eq!(
        keys,
        vec!["r13", "t20", "t30"],
        "an empty review is no thread"
    );
    assert!(counted(&fx, "r13"));
    assert_eq!(state(&fx, "t20"), ignored("a resolved thread"));
    assert!(counted(&fx, "t30"));
    let line = "stage 1 (PR #7): ignored a comment by @alice: a resolved thread";
    assert!(logged(&fx, line));
}

#[test]
fn a_new_comment_in_an_unresolved_thread_counts() {
    let mut fx = reviewed();
    let mut v = view(&commit(1));
    v.threads = vec![on("docs/t1/a.md", vec![noted(30, "alice", "rename")])];
    poll_with(&mut fx, v.clone());
    grant(&mut fx, "alice", RepoPermission::Admin);
    fx.tick();
    assert_eq!(
        state(&fx, "t30"),
        ThreadState::Tasked {
            task: "fix1".into()
        }
    );
    // A writer's new comment in the same unresolved thread makes it count again, once
    // the writer is known (task M9.2.10's fix round, I1: until then nothing changes).
    v.threads[0].comments.push(noted(31, "bob", "and the docs"));
    poll_with(&mut fx, v.clone());
    assert_eq!(
        state(&fx, "t30"),
        ThreadState::Tasked {
            task: "fix1".into()
        }
    );
    assert_eq!(asked(&fx), vec!["bob"]);
    grant(&mut fx, "bob", RepoPermission::Maintain);
    assert_eq!(state(&fx, "t30"), ThreadState::New);
    assert!(counted(&fx, "t30"));
    fx.tick();
    // Its fix task exists: the fast path never makes a second one; it is the user's.
    let fixes = (fx.run().tasks.iter()).filter(|t| t.id().starts_with("fix"));
    assert_eq!(fixes.count(), 1);
    assert!(attention(&fx).contains(&"PR #7: 1 thread not addressed".to_string()));
    // A comment in a resolved thread does not, and the resolved thread is no longer
    // the run's to address (the fix round's m2).
    v.threads[0].comments.push(noted(32, "bob", "one more"));
    v.threads[0].resolved = true;
    poll_with(&mut fx, v);
    assert_eq!(state(&fx, "t30"), ignored("a resolved thread"));
    assert!(!attention(&fx).iter().any(|l| l.contains("not addressed")));
    assert!(asked(&fx).is_empty());
}

#[test]
fn batch_closes_after_review_batch_secs_of_quiet() {
    let mut fx = reviewed();
    let limits = &mut fx.run_mut().delivery.limits;
    limits.review_batch_secs = 10;
    limits.reviewers = vec!["alice".into()];
    let mut v = view(&commit(1));
    v.comments = vec![said(1, "alice", "one")];
    poll_with(&mut fx, v.clone());
    // A second thread joins the open batch and moves its quiet time.
    v.comments.push(said(2, "alice", "two"));
    let (at, _) = poll_with(&mut fx, v);
    let batch = fx.run().delivery.stage(1).unwrap().batch.clone().unwrap();
    assert_eq!(batch.threads, vec!["c1", "c2"]);
    assert_eq!(batch.last_at, at);
    let fixes = |fx: &Fixture| {
        (fx.run().tasks.iter())
            .filter(|t| t.id().starts_with("fix"))
            .count()
    };
    fx.send(at + 9, crate::run::engine::EventKind::Tick);
    assert_eq!(fixes(&fx), 0, "not before ten quiet seconds");
    assert!(fx.run().delivery.stage(1).unwrap().batch.is_some());
    fx.send(at + 10, crate::run::engine::EventKind::Tick);
    assert_eq!(fixes(&fx), 2, "one batch, one fix task per thread");
    let stage = fx.run().delivery.stage(1).unwrap();
    assert!(stage.batch.is_none());
    assert_eq!((stage.batches, stage.review_rounds), (1, 1));
    // `review_batch_secs = 0` closes a batch at the view that found it.
    fx.run_mut().delivery.limits.review_batch_secs = 0;
    let mut v = view(&commit(1));
    v.comments = vec![said(3, "alice", "three")];
    poll_with(&mut fx, v);
    assert_eq!(fixes(&fx), 3);
    assert_eq!(fx.run().delivery.stage(1).unwrap().review_rounds, 2);
}

#[test]
fn planned_run_wakes_the_orchestrator_once_per_batch() {
    let mut fx = reviewed();
    super::bisect::with_orchestrator(&mut fx);
    let limits = &mut fx.run_mut().delivery.limits;
    limits.review_batch_secs = 5;
    limits.reviewers = vec!["alice".into(), "bob".into()];
    let tasks = fx.run().tasks.len();
    let mut v = view(&commit(1));
    v.comments = vec![said(1, "alice", "one")];
    poll_with(&mut fx, v.clone());
    v.threads = vec![on("docs/t1/a.md", vec![noted(30, "bob", "two")])];
    v.comments.push(said(2, "alice", "three"));
    let (at, _) = poll_with(&mut fx, v);
    fx.send(at + 5, crate::run::engine::EventKind::Tick);
    fx.send(at + 30, crate::run::engine::EventKind::Tick);
    let notes = &fx.run().orch.orchestrator.as_ref().unwrap().notes;
    let wakes: Vec<&String> = notes
        .iter()
        .filter(|n| n.contains("review thread"))
        .collect();
    assert_eq!(
        wakes,
        vec![
            "PR #7 (stage 1) has 3 new review threads from @alice, @bob; read them in run_status and add fix tasks, reply, or escalate"
        ],
        "{notes:#?}"
    );
    assert_eq!(fx.run().tasks.len(), tasks, "the engine adds no task");
    assert!(attention(&fx).contains(&"PR #7: 3 threads not addressed".to_string()));
    for key in ["c1", "c2", "t30"] {
        assert_eq!(state(&fx, key), ThreadState::New);
    }
}

#[test]
fn a_failed_permission_is_asked_again_when_due() {
    let mut fx = reviewed();
    let mut v = view(&commit(1));
    v.comments = vec![said(1, "alice", "a")];
    poll_with(&mut fx, v);
    let (op, _) = (host_ops(&fx).into_iter())
        .find(|(_, o)| matches!(o, HostOp::Permission { .. }))
        .expect("a Permission op");
    let error = HostResult::Error(crate::host::HostError::Failed("boom".into()));
    answer(&mut fx, op, error);
    let due = fx.run().delivery.permission_retry_at.expect("a retry time");
    fx.send(due - 1, crate::run::engine::EventKind::Tick);
    assert!(asked(&fx).is_empty(), "not before its retry time");
    fx.send(due, crate::run::engine::EventKind::Tick);
    assert_eq!(asked(&fx), vec!["alice"]);
    assert_eq!(fx.run().state, RunState::Running);
}

#[test]
fn a_batch_waits_for_a_pending_permission() {
    let mut fx = reviewed();
    fx.run_mut().delivery.limits.reviewers = vec!["alice".into()];
    let mut v = view(&commit(1));
    v.comments = vec![said(1, "alice", "a"), said(2, "bob", "b")];
    let (at, _) = poll_with(&mut fx, v);
    fx.run_mut().delivery.watching = false;
    fx.send(at + 5, crate::run::engine::EventKind::Tick);
    let stage = fx.run().delivery.stage(1).unwrap();
    assert!(
        stage.batch.is_some() && stage.batches == 0,
        "bob's answer may add to it"
    );
    grant(&mut fx, "bob", RepoPermission::Write);
    fx.tick();
    let stage = fx.run().delivery.stage(1).unwrap();
    assert_eq!(stage.batches, 1, "one batch");
    let fixes = (fx.run().tasks.iter()).filter(|t| t.id().starts_with("fix"));
    assert_eq!(fixes.count(), 2);
}
