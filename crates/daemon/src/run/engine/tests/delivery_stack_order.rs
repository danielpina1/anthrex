//! The final fix wave's B m-3 to m-6 and task 8's deferred opening-push hold: the order
//! in which a stack's stages reach the host, and one history line per landing outcome.
//! An open in flight at `run cancel` is recorded once it is answered; a reopened stage's
//! next landing gets its own record; a stage above a merged one opens only on the new
//! base; an upper stage pushes only once every lower stage's head is pushed, and opens
//! only once no lower stage is held.

use proto::{PrState, StageOutcome};

use super::delivery_land::{closed_view, head, merged_view, park, stage_lines};
use super::delivery_open::{answer, green, host_ops, opened, remote_branch};
use super::delivery_sync::{base_sync, fetched, fetching};
use super::delivery_watch::{PR, poll_with, view, watched_on};
use super::delivery_watch_adopt::{poll_stage, two_stages};
use super::fixture::*;
use super::merge::{commit, doc_task, merge, to_queue, window_of};
use super::propagate::{land_propagates, merged_at};
use crate::host::PushOutcome;
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{EventKind, OpId};

/// The pending host ops of stage `n`.
fn of_stage(fx: &Fixture, n: u16) -> Vec<(OpId, HostOp)> {
    (host_ops(fx).into_iter())
        .filter(|(_, op)| crate::run::engine::delivery::stage_of(op) == Some(n))
        .collect()
}

fn pushes(fx: &Fixture, n: u16) -> Vec<(OpId, String)> {
    (of_stage(fx, n).into_iter())
        .filter_map(|(op, o)| match o {
            HostOp::Push { sha, .. } => Some((op, sha)),
            _ => None,
        })
        .collect()
}

/// `t2` (stage 2 of `two_stages(false)`) merges at `commit(2)`.
fn t2_merges(fx: &mut Fixture, windows: &[(String, u32)]) {
    to_queue(fx, "t2", window_of(windows, "t2"));
    merge(fx, "t2", &commit(2));
    land_propagates(fx);
}

fn tier3(fx: &Fixture) -> bool {
    (super::merge::pending(fx, "Tier", None).into_iter())
        .any(|(_, k)| matches!(k, crate::run::engine::OpKind::Tier(s) if s.tier == 3))
}

#[test]
fn an_open_in_flight_at_cancel_is_recorded_once_answered() {
    let (mut fx, windows) = super::delivery_open::pr_on(PROFILE, &[doc_task("t1", "")]);
    fx.run_mut().repo_dir = "/tmp/repo".into();
    to_queue(&mut fx, "t1", window_of(&windows, "t1"));
    merge(&mut fx, "t1", &commit(1));
    green(&mut fx, 1);
    let (op, _) = super::delivery_open::host_op(&fx);
    answer(&mut fx, op, HostResult::Pushed(PushOutcome::Pushed));
    let (op, open) = super::delivery_open::host_op(&fx);
    assert!(matches!(open, HostOp::OpenPr { stage: 1, .. }), "{open:?}");
    let reply = fx.reply();
    let effects = fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    assert!(stage_lines(&effects).is_empty(), "no PR yet");
    let effects = answer(&mut fx, op, opened(PR, false));
    let lines = stage_lines(&effects);
    assert_eq!(lines.len(), 1, "{effects:#?}");
    assert_eq!(lines[0].outcome, StageOutcome::OpenAtCancel);
    assert_eq!(lines[0].pr, PR);
}

#[test]
fn a_close_then_a_reopen_then_a_merge_writes_two_records() {
    let (mut fx, _) = watched_on(PROFILE);
    fx.run_mut().repo_dir = "/tmp/repo".into();
    let at = head(&fx, 1);
    poll_with(&mut fx, closed_view(PR, &at));
    let closed = stage_lines(&fx.log);
    assert_eq!(closed.len(), 1, "{closed:#?}");
    assert_eq!(closed[0].outcome, StageOutcome::Closed);
    assert_eq!(closed[0].record_id, format!("{RUN_ID}/stage/1"));
    // Reopened, then merged: the merge is its own record.
    poll_with(&mut fx, view(&at));
    assert_eq!(fx.run().delivery.pr(1).unwrap().state, PrState::Open);
    poll_with(&mut fx, merged_view(PR, &at, &commit(70)));
    fetched(&mut fx, &commit(71), Some(2));
    let lines = stage_lines(&fx.log);
    assert_eq!(lines.len(), 2, "{lines:#?}");
    assert_eq!(lines[1].outcome, StageOutcome::Merged);
    assert_eq!(lines[1].record_id, format!("{RUN_ID}/stage/1-r1"));
}

#[test]
fn a_stage_above_a_merged_one_opens_only_on_the_new_base() {
    let (mut fx, windows) = two_stages(false);
    let h1 = head(&fx, 1);
    // The user merges stage 1's PR while stage 2 still works.
    poll_stage(&mut fx, 1, merged_view(11, &h1, &commit(70)));
    assert!(fetching(&fx), "the base is fetched");
    t2_merges(&mut fx, &windows);
    assert!(!tier3(&fx), "tier 3 waits for the new base");
    assert!(
        pushes(&fx, 2).is_empty(),
        "no opening push before the base is absorbed: {:#?}",
        host_ops(&fx)
    );
    fetched(&mut fx, &commit(71), Some(1));
    assert!(pushes(&fx, 2).is_empty(), "the base sync is due");
    let (op, _) = base_sync(&fx);
    fx.done(op, merged_at(&commit(72)));
    // Tier 3 runs on the synced head (decision 20), then it opens.
    fx.tick();
    assert_eq!(super::full::full_job(&fx).1.head, commit(72));
    green(&mut fx, 2);
    let pushed = pushes(&fx, 2);
    assert_eq!(
        pushed.iter().map(|(_, s)| s.as_str()).collect::<Vec<_>>(),
        [commit(72).as_str()],
        "{:#?}",
        host_ops(&fx)
    );
    answer(
        &mut fx,
        pushed[0].0,
        HostResult::Pushed(PushOutcome::Pushed),
    );
    let open = of_stage(&fx, 2);
    assert!(
        matches!(&open[..], [(_, HostOp::OpenPr { base, .. })] if base == "main"),
        "{open:#?}"
    );
}

#[test]
fn an_upper_stage_pushes_only_once_the_lower_head_is_pushed() {
    let (mut fx, _) = two_stages(true);
    // No view goes out in this test, so only the pushes are host ops.
    park(&mut fx, 1);
    park(&mut fx, 2);
    set_stage_head(fx.run_mut(), 1, &commit(50));
    fx.tick();
    land_propagates(&mut fx);
    fx.tick();
    let lower = pushes(&fx, 1);
    assert_eq!(lower.len(), 1, "{:#?}", host_ops(&fx));
    assert!(
        pushes(&fx, 2).is_empty(),
        "stage 2 waits for stage 1's push: {:#?}",
        host_ops(&fx)
    );
    answer(&mut fx, lower[0].0, HostResult::Pushed(PushOutcome::Pushed));
    fx.tick();
    let h2 = head(&fx, 2);
    let upper: Vec<String> = pushes(&fx, 2).into_iter().map(|(_, s)| s).collect();
    assert_eq!(upper, [h2]);
}

#[test]
fn an_upper_stage_opens_only_once_the_lower_head_is_pushed() {
    let (mut fx, windows) = two_stages(false);
    // Stage 1 moves; its update push is in flight.
    set_stage_head(fx.run_mut(), 1, &commit(50));
    fx.tick();
    let lower = pushes(&fx, 1);
    assert_eq!(lower.len(), 1, "{:#?}", host_ops(&fx));
    land_propagates(&mut fx);
    t2_merges(&mut fx, &windows);
    fx.tick();
    assert!(
        of_stage(&fx, 2).is_empty() && !tier3(&fx),
        "stage 2 is delivered only once stage 1's head is pushed: {:#?}",
        host_ops(&fx)
    );
    answer(&mut fx, lower[0].0, HostResult::Pushed(PushOutcome::Pushed));
    fx.tick();
    green(&mut fx, 2);
    let upper = pushes(&fx, 2);
    assert_eq!(upper.len(), 1, "{:#?}", host_ops(&fx));
}

/// Task 8's deferred item: a held lower stage (its push refused) keeps the stage above
/// from opening; `run resume` pushes it, and the stage above opens after it.
#[test]
fn an_upper_stage_opens_only_once_no_lower_stage_is_held() {
    let (mut fx, windows) = two_stages(false);
    park(&mut fx, 1);
    set_stage_head(fx.run_mut(), 1, &commit(50));
    fx.tick();
    let (op, _) = pushes(&fx, 1)[0].clone();
    let refused = PushOutcome::Refused {
        reason: format!(
            "the remote refused the push of {}: protected",
            remote_branch(1)
        ),
    };
    answer(&mut fx, op, HostResult::Pushed(refused));
    assert!(fx.run().delivery.stage(1).unwrap().held.is_some());
    land_propagates(&mut fx);
    t2_merges(&mut fx, &windows);
    let later = fx.now + 30;
    fx.send(later, EventKind::Tick);
    assert!(
        of_stage(&fx, 2).is_empty() && !tier3(&fx),
        "a held stage below: {:#?}",
        host_ops(&fx)
    );
    super::control::resume(&mut fx);
    let (op, _) = pushes(&fx, 1)[0].clone();
    answer(&mut fx, op, HostResult::Pushed(PushOutcome::Pushed));
    fx.tick();
    green(&mut fx, 2);
    assert_eq!(pushes(&fx, 2).len(), 1, "{:#?}", host_ops(&fx));
}
