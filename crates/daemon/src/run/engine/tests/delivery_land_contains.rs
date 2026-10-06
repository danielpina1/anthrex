//! Milestone 9.7 task M9.7.5 (DH §1.2, decisions 4, 5, 7, 8 and 9; ruling R1): a stage
//! PR merged at a head that is neither its local head nor a confirmed one is undecided.
//! Its verdict, its replies and its "not delivered" line wait for the base fetch to say
//! whether the merge contains the local head; so do the run's completion and the
//! merged branch's delete. Only `Some(true)` delivers; `Some(false)`, no answer and a
//! missing base read "not delivered"; a failed check is asked again after the wait,
//! and the third failure in a row decides as not delivered (ruling R1).

use proto::{MergeMethod, PrState};

use super::bisect::with_orchestrator;
use super::control::resume;
use super::control_restore::restart;
use super::delivery_land::{logged, merged_view};
use super::delivery_land_fixes::completes;
use super::delivery_open::{answer, host_ops, remote_branch};
use super::delivery_review::said;
use super::delivery_review_reply::{by_alice, merge_fix, one_reply, push_lands, reply_ops};
use super::delivery_sync::{base_fetch, base_ref, fetching};
use super::delivery_watch::{PR, poll_with, view};
use super::delivery_watch_adopt::{poll_stage, two_stages};
use super::fixture::*;
use super::full::{attention, later};
use super::merge::{commit, pending};
use super::propagate::land_propagates;
use super::wake_notes::notes;
use crate::host::{Contains, FetchOutcome, HostError, PushOutcome};
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{EventKind, OpResult};

/// The local stage head `H2`, pushed and never shown by a view.
fn h2() -> String {
    commit(2)
}

/// The head the host merged at, `U2`.
fn u2() -> String {
    commit(80)
}

const UNLANDED: &str = "stage 1 PR #7 was merged at 80eeeee, without 2eeeeee; that work is not delivered (anthrex run cancel gives up)";
const UNCHECKED: &str =
    "stage 1: could not check whether 2eeeeee is in the merge; treating it as not delivered";

/// The question the base fetch carries for stage 1.
fn question() -> Contains {
    Contains {
        stage: 1,
        branch: remote_branch(1),
        into: format!("refs/anthrex/{RUN_ID}/remote/stage-1"),
        head: h2(),
        merged: u2(),
        pr: Some(PR),
    }
}

/// `by_alice` (PR #7 on stage 1, opened at `commit(1)`): `c5` by alice gets fix task
/// `fix1`, merged at `H2` and pushed; no view shows `H2`. The user merges PR #7 at
/// `U2` (merge commit `commit(90)`).
fn merged_unverified() -> Fixture {
    let mut fx = by_alice();
    let mut v = view(&commit(1));
    v.comments = vec![said(5, "alice", "Please update the README.")];
    let (at, _) = poll_with(&mut fx, v);
    fx.run_mut().delivery.watching = false;
    fx.send(at + 1, EventKind::Tick);
    merge_fix(&mut fx, "fix1", &h2());
    push_lands(&mut fx, &h2());
    fx.tick();
    with_orchestrator(&mut fx);
    fx.run_mut().delivery.watching = true;
    poll_with(&mut fx, merged_view(PR, &u2(), &commit(90)));
    fx
}

fn undecided(fx: &Fixture) -> Option<(String, String)> {
    fx.run().delivery.stage(1).unwrap().undecided.clone()
}

/// Answers the pending base fetch: the base at `commit(71)`, a squash, and `contains`.
fn fetched_with(fx: &mut Fixture, contains: Option<bool>) {
    let (op, _) = base_fetch(fx);
    let outcome = FetchOutcome::Fetched {
        sha: commit(71),
        parents: Some(1),
        contains,
    };
    answer(fx, op, HostResult::Fetched(outcome));
}

/// Fails the pending base fetch.
fn fetch_fails(fx: &mut Fixture) {
    let (op, _) = base_fetch(fx);
    let error = HostError::Failed("could not reach the remote".into());
    answer(fx, op, HostResult::Error(error));
}

fn unlanded(fx: &Fixture) -> bool {
    attention(fx).contains(&UNLANDED.to_string())
}

#[test]
fn an_unverified_merge_holds_its_verdict_and_asks_the_base_fetch() {
    let fx = merged_unverified();
    assert!(!unlanded(&fx), "{:#?}", attention(&fx));
    assert!(
        fx.run().delivery.alerts.is_empty(),
        "{:#?}",
        fx.run().delivery.alerts
    );
    // fix1's reply is neither due nor dropped.
    let replies = &fx.run().delivery.stage(1).unwrap().replies;
    assert!(!replies.is_empty(), "no reply dropped");
    assert!(
        replies.iter().all(|r| !r.ready),
        "no reply due: {replies:#?}"
    );
    assert!(reply_ops(&fx).is_empty());
    assert_eq!(undecided(&fx), Some((h2(), u2())));
    let (_, op) = base_fetch(&fx);
    assert_eq!(
        op,
        HostOp::Fetch {
            stage: None,
            branch: "main".into(),
            into: base_ref(),
            adopt: None,
            parents_of: Some(commit(90)),
            contains: Some(question()),
        }
    );
}

#[test]
fn a_merge_that_contains_the_local_head_is_delivered_without_a_line() {
    let mut fx = merged_unverified();
    fetched_with(&mut fx, Some(true));
    assert_eq!(undecided(&fx), None);
    assert!(!(fx.run().log.iter()).any(|l| l.text.contains("not delivered")));
    assert!(!(fx.run().log.iter()).any(|l| l.text.contains("missed the merge")));
    assert!(attention(&fx).is_empty(), "{:#?}", attention(&fx));
    // The held replies are judged delivered: fix1's reply goes out.
    let (_, reply) = one_reply(&fx);
    assert!(matches!(reply, HostOp::Reply { ref thread, .. } if thread == "7:c5"));
}

#[test]
fn a_merge_without_the_local_head_reads_not_delivered() {
    let mut fx = merged_unverified();
    fetched_with(&mut fx, Some(false));
    assert_eq!(undecided(&fx), None);
    assert!(unlanded(&fx), "{:#?}", attention(&fx));
    assert!(
        notes(&fx).contains(&UNLANDED.to_string()),
        "{:#?}",
        notes(&fx)
    );
    assert!(!logged(&fx, UNCHECKED));
    assert!(
        reply_ops(&fx).is_empty(),
        "the fix missed the merge: no reply"
    );
    assert!((attention(&fx).iter()).any(|l| l.contains("missed the merge")));
}

#[test]
fn an_unanswered_check_reads_not_delivered() {
    let mut fx = merged_unverified();
    fetched_with(&mut fx, None);
    assert_eq!(undecided(&fx), None);
    assert!(unlanded(&fx), "{:#?}", attention(&fx));
    // A base that is gone from the remote decides too (BR-4).
    let mut fx = merged_unverified();
    let (op, _) = base_fetch(&fx);
    answer(&mut fx, op, HostResult::Fetched(FetchOutcome::Missing));
    assert_eq!(undecided(&fx), None);
    assert!(unlanded(&fx), "{:#?}", attention(&fx));
}

#[test]
fn a_failed_check_is_asked_again_after_the_wait() {
    let mut fx = merged_unverified();
    fetch_fails(&mut fx);
    assert_eq!(undecided(&fx), Some((h2(), u2())));
    assert_eq!(fx.run().delivery.stage(1).unwrap().undecided_fails, 1);
    assert!(!unlanded(&fx));
    let retry = fx.run().delivery.base_fetch_retry_at.expect("a wait");
    assert!(retry > fx.now);
    while fx.now + 1 < retry {
        later(&mut fx, 1);
        assert!(!fetching(&fx), "no fetch before the wait ends");
    }
    fx.send(retry, EventKind::Tick);
    let (_, op) = base_fetch(&fx);
    assert!(
        matches!(&op, HostOp::Fetch { contains: Some(c), .. } if *c == question()),
        "{op:?}"
    );
}

/// Ruling R1: three failed checks in a row decide as not delivered, and a run whose
/// stage above carries the work completes.
#[test]
fn three_failed_checks_decide_not_delivered() {
    let mut fx = merged_unverified();
    for k in 1..=3u8 {
        fetch_fails(&mut fx);
        if k < 3 {
            assert_eq!(fx.run().delivery.stage(1).unwrap().undecided_fails, k);
            assert!(!logged(&fx, UNCHECKED));
            later(&mut fx, 30);
        }
    }
    assert_eq!(undecided(&fx), None);
    assert_eq!(fx.run().delivery.stage(1).unwrap().undecided_fails, 0);
    assert!(logged(&fx, UNCHECKED), "{:#?}", fx.run().log);
    assert!(unlanded(&fx), "{:#?}", attention(&fx));
    let texts: Vec<&str> = fx.run().log.iter().map(|l| l.text.as_str()).collect();
    let (c, u) = (
        texts.iter().position(|t| *t == UNCHECKED),
        texts.iter().position(|t| *t == UNLANDED),
    );
    assert!(c < u, "the unchecked line first: {texts:#?}");

    // Two stages: stage 1's unverifiable work goes up with stage 2, and once stage 2
    // merges the run completes.
    let (mut fx, _) = two_stages(true);
    fx.run_mut().repo_dir = "/tmp/repo".into();
    let pushed = super::delivery_land::head(&fx, 1);
    fx.run_mut().delivery.stages[0].held = Some("protected branch".into());
    set_stage_head(fx.run_mut(), 1, &commit(60));
    fx.tick();
    land_propagates(&mut fx);
    for (op, kind) in host_ops(&fx) {
        if let HostOp::Push { stage: 2, .. } = kind {
            answer(&mut fx, op, HostResult::Pushed(PushOutcome::Pushed));
        }
    }
    let h2 = super::delivery_land::head(&fx, 2);
    let views = [
        merged_view(11, &pushed, &commit(70)),
        merged_view(12, &h2, &commit(81)),
    ];
    poll_stage(&mut fx, 1, views[0].clone());
    assert_eq!(
        fx.run().delivery.stage(1).unwrap().undecided,
        Some((commit(60), pushed.clone()))
    );
    for _ in 0..3 {
        fetch_fails(&mut fx);
        later(&mut fx, 30);
    }
    let line =
        "stage 1: could not check whether 60eeeee is in the merge; treating it as not delivered";
    assert!(logged(&fx, line), "{:#?}", fx.run().log);
    let up = format!(
        "stage 1 (PR #11): merged at {}, without 60eeeee, so its commits go up with stage 2",
        &pushed[..7]
    );
    assert!(logged(&fx, &up), "{:#?}", fx.run().log);
    poll_stage(&mut fx, 2, views[1].clone());
    for _ in 0..4 {
        if fetching(&fx) {
            fetched_with(&mut fx, None);
        }
        later(&mut fx, 1);
    }
    let methods: Vec<_> = (1..=2)
        .map(|n| fx.run().delivery.pr(n).unwrap().merge_method)
        .collect();
    assert_eq!(methods, [Some(MergeMethod::SquashOrRebase); 2]);
    completes(&mut fx, &views);
}

#[test]
fn a_restart_while_undecided_asks_again() {
    let mut fx = merged_unverified();
    assert!(fetching(&fx), "the check is in flight");
    restart(&mut fx, Vec::new());
    resume(&mut fx);
    later(&mut fx, 1);
    assert_eq!(undecided(&fx), Some((h2(), u2())));
    let (_, op) = base_fetch(&fx);
    assert!(
        matches!(&op, HostOp::Fetch { contains: Some(c), .. } if *c == question()),
        "{op:?}"
    );
    fetched_with(&mut fx, Some(true));
    assert_eq!(undecided(&fx), None);
}

#[test]
fn an_undecided_stage_holds_completion_and_the_branch_delete() {
    let mut fx = merged_unverified();
    fx.run_mut().delivery.limits.delete_merged_branches = true;
    // A stale answer (another head) is ignored: the base is fetched, the method known
    // and the history line out, but the stage is still undecided.
    let stale = (h2(), commit(81));
    fx.run_mut().delivery.stages[0].undecided = Some(stale.clone());
    fetched_with(&mut fx, Some(true));
    assert_eq!(
        undecided(&fx),
        Some(stale.clone()),
        "a stale answer is ignored"
    );
    assert!(!fx.run().delivery.base_fetch_due);
    let deletes = |fx: &Fixture| {
        (host_ops(fx).into_iter())
            .filter(|(_, o)| matches!(o, HostOp::DeleteBranch { .. }))
            .count()
    };
    for _ in 0..5 {
        for op in pending(&fx, "AppendHistory", None)
            .into_iter()
            .map(|(op, _)| op)
        {
            fx.done(op, OpResult::HistoryAppended);
        }
        if let Some((op, _)) = reply_ops(&fx).first().cloned() {
            answer(&mut fx, op, HostResult::Replied { comment_id: 901 });
        }
        later(&mut fx, 1);
        assert_eq!(deletes(&fx), 0, "{:#?}", host_ops(&fx));
        assert!(pending(&fx, "VerifyRefs", None).is_empty(), "not complete");
    }
    assert_eq!(fx.run().delivery.pr(1).unwrap().state, PrState::Merged);
    // The answer to the question asked again decides it; the delete and the
    // completion follow.
    let (_, op) = base_fetch(&fx);
    let HostOp::Fetch {
        contains: Some(c), ..
    } = op
    else {
        panic!("{op:?}")
    };
    assert_eq!((c.head.clone(), c.merged.clone()), stale);
    fetched_with(&mut fx, Some(true));
    assert_eq!(undecided(&fx), None);
    let (op, _) = one_reply(&fx);
    answer(&mut fx, op, HostResult::Replied { comment_id: 901 });
    later(&mut fx, 1);
    let found: Vec<_> = (host_ops(&fx).into_iter())
        .filter(|(_, o)| matches!(o, HostOp::DeleteBranch { .. }))
        .collect();
    assert_eq!(found.len(), 1, "{:#?}", host_ops(&fx));
    answer(&mut fx, found[0].0, HostResult::Deleted);
    completes(&mut fx, &[merged_view(PR, &u2(), &commit(90))]);
}
