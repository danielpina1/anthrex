//! The final fix wave's I-1 (review B): what a stage PR delivered is the head the host
//! reports at the merge, never what anthrex pushed. GitHub accepts a push to a merged
//! PR's branch, so a push answered after the user's merge proves nothing: its work is
//! not delivered, and the replies of the fix it carried are dropped with an attention
//! line. A reply is due only once a view of the still-open PR shows the pushed head.

use proto::PrState;

use super::delivery_land::merged_view;
use super::delivery_open::{answer, host_ops};
use super::delivery_review::state;
use super::delivery_review::{noted, on, said};
use super::delivery_review_reply::{by_alice, merge_fix, one_reply, push_lands, reply_ops, shown};
use super::delivery_watch::{PR, poll_with, view};
use super::fixture::*;
use super::full::attention;
use super::merge::commit;
use crate::run::delivery::ThreadState;
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::EventKind;

/// `c5` by alice with its fix task `fix1`, which merged at `commit(2)`; views off, so
/// only the push goes out.
fn fix1_merged(fx: &mut Fixture) {
    let mut v = view(&commit(1));
    v.comments = vec![said(5, "alice", "Please update the README.")];
    v.threads = vec![on("docs/t1/a.md", vec![noted(30, "alice", "Rename this.")])];
    let (at, _) = poll_with(fx, v);
    fx.run_mut().delivery.watching = false;
    fx.send(at + 1, EventKind::Tick);
    merge_fix(fx, "fix1", &commit(2));
}

/// The next view of PR #7 shows it merged at `head`.
fn merged_at(fx: &mut Fixture, head: &str) {
    fx.run_mut().delivery.watching = true;
    poll_with(fx, merged_view(PR, head, &commit(90)));
    fx.tick();
}

const MISSED: &str = "PR #7 was merged at 1eeeeee before fix task fix1 reached it: the fix missed the merge, so thread 7:c5 gets no reply";
const UNLANDED: &str = "stage 1 PR #7 was merged at 1eeeeee, without 2eeeeee; that work is not delivered (anthrex run cancel gives up)";

#[test]
fn a_push_answered_after_the_users_merge_delivers_nothing_and_its_replies_drop() {
    let mut fx = by_alice();
    fix1_merged(&mut fx);
    // The user merged PR #7 at commit(1) on GitHub; anthrex has not seen it, and
    // GitHub accepts the push of commit(2) to the merged PR's branch.
    push_lands(&mut fx, &commit(2));
    fx.tick();
    assert!(
        reply_ops(&fx).is_empty(),
        "an answered push alone is not a reply"
    );
    merged_at(&mut fx, &commit(1));
    assert!(
        reply_ops(&fx).is_empty(),
        "the fix missed the merge: no reply"
    );
    assert!(fx.run().delivery.stage(1).unwrap().replies.is_empty());
    assert_eq!(
        state(&fx, "c5"),
        ThreadState::Tasked {
            task: "fix1".into()
        },
        "no `Addressed in` for a fix that is not on the merged PR"
    );
    let lines = attention(&fx);
    assert!(lines.contains(&MISSED.to_string()), "{lines:#?}");
    assert!(lines.contains(&UNLANDED.to_string()), "{lines:#?}");
}

#[test]
fn a_push_a_view_of_the_open_pr_showed_is_delivered_and_replied() {
    let mut fx = by_alice();
    fix1_merged(&mut fx);
    push_lands(&mut fx, &commit(2));
    shown(&mut fx, &commit(2));
    let (op, reply) = one_reply(&fx);
    let HostOp::Reply { thread, body, .. } = reply else {
        unreachable!()
    };
    assert_eq!(thread, "7:c5");
    assert_eq!(body, "@alice Addressed in 2eeeeee by task fix1.");
    answer(&mut fx, op, HostResult::Replied { comment_id: 901 });
    assert_eq!(state(&fx, "c5"), ThreadState::Replied { comment_id: 901 });
    // The user merges at the pushed head: delivered, no line.
    merged_at(&mut fx, &commit(2));
    let lines = attention(&fx);
    assert!(lines.is_empty(), "{lines:#?}");
    let pr = fx.run().delivery.pr(1).unwrap();
    assert_eq!(pr.state, PrState::Merged);
}

#[test]
fn a_merge_at_the_pushed_head_before_any_view_still_replies() {
    let mut fx = by_alice();
    fix1_merged(&mut fx);
    push_lands(&mut fx, &commit(2));
    // The merge happened after the push reached the PR: the host's head holds it.
    merged_at(&mut fx, &commit(2));
    let (_, reply) = one_reply(&fx);
    assert!(matches!(reply, HostOp::Reply { ref thread, .. } if thread == "7:c5"));
    assert!(attention(&fx).is_empty(), "{:#?}", attention(&fx));
    assert!(
        host_ops(&fx)
            .iter()
            .all(|(_, o)| !matches!(o, HostOp::Push { .. }))
    );
}
