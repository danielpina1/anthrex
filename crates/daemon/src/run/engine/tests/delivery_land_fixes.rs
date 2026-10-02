//! Task M9.2.11's fix round 1: every merged stage gets its merge method and its one
//! `stage` line however its merge and the base fetches interleave (I1); a PR reopened
//! and merged between two polls lands as merged (I2); a merge commit oid that is not
//! one asks git nothing (m2); a base the user set by hand is left alone until a stage
//! below changes (ruling 97, m4).

use proto::{MergeMethod, RunState, StageOutcome};

use super::delivery_land::{
    closed_view, head, logged, merged_view, park, stage_1_squashed, stage_lines,
};
use super::delivery_open::{answer, host_ops, host_ops_in};
use super::delivery_sync::{base_fetch, base_fetch_op, fetched, fetching};
use super::delivery_watch::{poll_with, watched};
use super::delivery_watch_adopt::{poll_stage, stage_op, two_stages, view_of};
use super::fixture::*;
use super::full::{attention, later, verify_ok};
use super::merge::{commit, pending};
use crate::host::{HostError, MergeCommit, PrView, PushOutcome};
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::{OpKind, OpResult};

/// The run settles and completes: its history lines are appended and the polls of its
/// merged PRs answered with `views` (stage 1's first), then its refs check.
fn completes(fx: &mut Fixture, views: &[PrView]) {
    for _ in 0..10 {
        let appends: Vec<_> = (fx.run().pending_ops.values())
            .filter(|p| matches!(p.kind, OpKind::AppendHistory { .. }))
            .map(|p| p.op)
            .collect();
        for op in appends {
            fx.done(op, OpResult::HistoryAppended);
        }
        for (op, kind) in host_ops(fx) {
            if let HostOp::ViewPr { stage, .. } = kind {
                let view = views[usize::from(stage) - 1].clone();
                answer(fx, op, HostResult::PrViewed(Box::new(view)));
            }
        }
        if !pending(fx, "VerifyRefs", None).is_empty() {
            break;
        }
        fx.tick();
    }
    verify_ok(fx);
    assert_eq!(fx.run().state, RunState::Complete);
}

/// Each stage's `stage` line: (stage, outcome, method).
fn lines(fx: &Fixture) -> Vec<(u16, StageOutcome, MergeMethod)> {
    (stage_lines(&fx.log).into_iter())
        .map(|l| (l.stage, l.outcome, l.merge_method))
        .collect()
}

fn two_merged() -> Vec<(u16, StageOutcome, MergeMethod)> {
    vec![
        (1, StageOutcome::Merged, MergeMethod::Merge),
        (2, StageOutcome::Merged, MergeMethod::SquashOrRebase),
    ]
}

/// I1 (a): both stage PRs are merged before one base fetch (the first one failed): the
/// fetch counts stage 1's merge commit, and another is due for stage 2's.
#[test]
fn two_merges_seen_before_one_base_fetch_each_get_a_method() {
    let (mut fx, _) = two_stages(true);
    fx.run_mut().repo_dir = "/tmp/repo".into();
    let (h1, h2) = (head(&fx, 1), head(&fx, 2));
    let views = [
        merged_view(11, &h1, &commit(70)),
        merged_view(12, &h2, &commit(80)),
    ];
    poll_stage(&mut fx, 1, views[0].clone());
    let (op, _) = base_fetch(&fx);
    let lost = HostError::Failed("could not reach the remote".into());
    answer(&mut fx, op, HostResult::Error(lost));
    assert!(!fetching(&fx));
    poll_stage(&mut fx, 2, views[1].clone());
    later(&mut fx, 30);
    let (_, op) = base_fetch(&fx);
    assert_eq!(op, base_fetch_op(Some(commit(70))));
    fetched(&mut fx, &commit(71), Some(2));
    let (_, op) = base_fetch(&fx);
    assert_eq!(op, base_fetch_op(Some(commit(80))), "stage 2's method");
    fetched(&mut fx, &commit(71), Some(1));
    assert_eq!(lines(&fx), two_merged());
    completes(&mut fx, &views);
}

/// I1 (b): stage 2 merges while stage 1's base fetch is in flight.
#[test]
fn a_merge_seen_while_the_base_fetch_is_in_flight_gets_its_method() {
    let (mut fx, _) = two_stages(true);
    fx.run_mut().repo_dir = "/tmp/repo".into();
    let (h1, h2) = (head(&fx, 1), head(&fx, 2));
    let views = [
        merged_view(11, &h1, &commit(70)),
        merged_view(12, &h2, &commit(80)),
    ];
    poll_stage(&mut fx, 1, views[0].clone());
    assert!(fetching(&fx));
    poll_stage(&mut fx, 2, views[1].clone());
    let (_, op) = base_fetch(&fx);
    assert_eq!(op, base_fetch_op(Some(commit(70))), "one fetch at a time");
    fetched(&mut fx, &commit(71), Some(2));
    let (_, op) = base_fetch(&fx);
    assert_eq!(op, base_fetch_op(Some(commit(80))));
    fetched(&mut fx, &commit(71), Some(1));
    assert_eq!(lines(&fx), two_merged());
    completes(&mut fx, &views);
}

/// I2: closed, then reopened and merged before the next poll: the view goes straight
/// from closed to merged. The closed line goes, and the history's last word on the
/// stage is `merged`.
#[test]
fn a_pr_reopened_and_merged_between_polls_lands_as_merged() {
    let (mut fx, _) = two_stages(true);
    fx.run_mut().repo_dir = "/tmp/repo".into();
    park(&mut fx, 2);
    let h1 = head(&fx, 1);
    let closed = "stage 1 PR closed without merging; resume, re-plan, or cancel the rest";
    poll_stage(&mut fx, 1, closed_view(11, &h1));
    assert!(attention(&fx).contains(&closed.to_string()));
    assert_eq!(
        lines(&fx),
        [(1, StageOutcome::Closed, MergeMethod::None)],
        "the closed line"
    );
    poll_stage(&mut fx, 1, merged_view(11, &h1, &commit(70)));
    assert!(
        !attention(&fx).contains(&closed.to_string()),
        "{:?}",
        attention(&fx)
    );
    fetched(&mut fx, &commit(71), Some(2));
    let last = lines(&fx).into_iter().rfind(|l| l.0 == 1);
    assert_eq!(last, Some((1, StageOutcome::Merged, MergeMethod::Merge)));
}

/// m2: a merge commit oid that is not an object id is never handed to git: the
/// method is unknown, the line is written, and the run completes.
#[test]
fn a_merge_commit_that_is_not_an_object_id_asks_git_nothing() {
    let mut fx = watched();
    fx.run_mut().repo_dir = "/tmp/repo".into();
    let at = head(&fx, 1);
    let view = PrView {
        merge_commit: Some(MergeCommit {
            oid: "not-an-object-id".into(),
        }),
        ..merged_view(super::delivery_watch::PR, &at, &commit(70))
    };
    poll_with(&mut fx, view.clone());
    let (_, op) = base_fetch(&fx);
    assert_eq!(op, base_fetch_op(None));
    fetched(&mut fx, &commit(71), None);
    let pr = fx.run().delivery.pr(1).unwrap();
    assert_eq!(pr.merge_method, Some(MergeMethod::None));
    assert_eq!(lines(&fx), [(1, StageOutcome::Merged, MergeMethod::None)]);
    completes(&mut fx, &[view]);
}

/// m4, ruling 97: a retarget is judged against the base anthrex set, never the host's.
/// A base the user changed by hand, with no change below, is left alone; a later merge
/// below puts the PR back on the stack's base.
#[test]
fn a_base_the_user_set_is_left_alone_until_a_stage_below_changes() {
    let (mut fx, _) = two_stages(true);
    let h2 = head(&fx, 2);
    let changed = PrView {
        base_ref: "user-picked".into(),
        ..view_of(12, &h2)
    };
    poll_stage(&mut fx, 2, changed);
    assert_eq!(fx.run().delivery.pr(2).unwrap().base, "user-picked");
    park(&mut fx, 2);
    for _ in 0..5 {
        fx.tick();
    }
    let retargets = |fx: &Fixture| {
        (host_ops_in(&fx.log).into_iter())
            .filter(|o| matches!(o, HostOp::Retarget { .. }))
            .collect::<Vec<_>>()
    };
    assert!(retargets(&fx).is_empty(), "nothing below changed");
    stage_1_squashed(&mut fx);
    let (op, push) = stage_op(&fx, 2);
    assert!(matches!(push, HostOp::Push { stage: 2, .. }), "{push:?}");
    answer(&mut fx, op, HostResult::Pushed(PushOutcome::Pushed));
    let (op, retarget) = stage_op(&fx, 2);
    let back = HostOp::Retarget {
        stage: 2,
        number: 12,
        base: "main".into(),
    };
    assert_eq!(retarget, back);
    answer(&mut fx, op, HostResult::Retargeted);
    assert_eq!(retargets(&fx), [back]);
    assert!(logged(&fx, "stage 2 (PR #12): retargeted onto main"));
}
