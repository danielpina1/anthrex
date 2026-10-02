//! The final fix wave's I-2 (review B): a red with key K on a stage whose CI fix for K
//! is still unfinished adds no second fix task. A propagate, a review fix or a base sync
//! pushes a new head while the fix works, and CI is red there with the same failure:
//! the red joins the fix (its record keeps the newest head), and once the fix finishes
//! CI is judged again on the stage's newest head. `ci_fix_max` counts fixes, not reds.

use proto::PlanEdit;

use super::delivery_ci::{RUN_A, RUN_B, answer_logs, ci_fixes, red_view, test_red, tier2};
use super::delivery_land::logged;
use super::delivery_open::answer;
use super::delivery_review::{noted, on};
use super::delivery_review_reply::merge_fix;
use super::delivery_sync::{base_sync, conflicting, fetched};
use super::delivery_watch::{PR, poll_with, view, watched};
use super::delivery_watch_adopt::{poll_stage, stage_op, two_stages};
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::full::{outcome, tier};
use super::merge::commit;
use super::propagate::{land_propagates, merged_at};
use crate::host::PushOutcome;
use crate::run::delivery::ops::{HostOp, HostResult};
use crate::run::engine::stages::set_stage_head;
use crate::run::engine::{EventKind, OrchEvent};
use crate::run::model::FixOf;

/// A red of `test` (Actions run `run`) on stage `n`'s pushed head, PR `number`, taken
/// through its log, the fallback summary (key `unknown`) and a green tier-2
/// reproduction: the CI fix task it added, if any.
pub(super) fn red(fx: &mut Fixture, n: u16, number: u64, run: u64) -> Option<String> {
    let now = fx.now;
    let pr = fx.run_mut().delivery.stages[usize::from(n) - 1].pr.as_mut();
    let pr = pr.unwrap();
    pr.next_poll_at = now;
    let head = pr.pushed_head.clone();
    let before = ci_fixes(fx);
    // A reply the last view made due goes first.
    for (op, out) in super::delivery_open::host_ops(fx) {
        if let HostOp::Reply { .. } = out {
            answer(
                fx,
                op,
                HostResult::Replied {
                    comment_id: 900 + op,
                },
            );
        }
    }
    let v = crate::host::PrView {
        number,
        ..red_view(&head, test_red(run))
    };
    poll_stage(fx, n, v);
    answer_logs(fx, "--- FAIL: a::works");
    if let Some((op, _)) = tier2(fx) {
        fx.done(op, tier(outcome(2, &[])));
    }
    ci_fixes(fx).into_iter().find(|f| !before.contains(f))
}

/// Stage `n`'s pending push lands (a view that went out first is answered first, with
/// nothing new on it).
fn push_lands(fx: &mut Fixture, n: u16, sha: &str) {
    let (op, out) = stage_op(fx, n);
    if let HostOp::ViewPr { number, .. } = out {
        let head = fx.run().delivery.pr(n).unwrap().pushed_head.clone();
        let v = crate::host::PrView {
            number,
            ..view(&head)
        };
        answer(fx, op, HostResult::PrViewed(Box::new(v)));
    }
    let (op, push) = stage_op(fx, n);
    assert_eq!(
        push,
        HostOp::Push {
            stage: n,
            sha: sha.into()
        }
    );
    answer(fx, op, HostResult::Pushed(PushOutcome::Pushed));
}

/// The stage's CI fix for `unknown` is `id`, and its record keeps `newest`.
fn joined(fx: &Fixture, n: u16, id: &str, newest: &str) {
    let stage = fx.run().delivery.stage(n).unwrap();
    let rec = (stage.ci.iter()).find(|r| r.fix_task.as_deref() == Some(id));
    assert_eq!(
        rec.and_then(|r| r.newest.as_deref()),
        Some(newest),
        "{:#?}",
        stage.ci
    );
    let line = format!(
        "stage {n}: CI red at {} is the failure fix task {id} is still fixing; CI is judged again when it finishes",
        &newest[..7]
    );
    assert!(logged(fx, &line), "{:#?}", fx.run().log);
}

/// The user cancels `id`; CI is judged again on stage `n`'s newest head, whose red
/// gets a new fix task (the cancelled one made no fix: `ci_fix_max` is not spent).
fn finishes_then_judged_again(fx: &mut Fixture, n: u16, number: u64, id: &str) {
    // One fix per key: the cancelled one must not count.
    fx.run_mut().delivery.limits.ci_fix_max = 1;
    let cancel = PlanEdit::CancelTask { task_id: id.into() };
    assert!(replies(&edit(fx, vec![cancel]))[0].is_ok());
    fx.tick();
    let head = fx.run().delivery.pr(n).unwrap().pushed_head.clone();
    let wm = &fx.run().delivery.pr(n).unwrap().watermark;
    assert!(!wm.ci.contains_key(&head), "the red there is judged again");
    let fix = red(fx, n, number, RUN_B + 100).expect("a new fix once the first ended");
    let task = fx.task(&fix);
    assert!(
        matches!(&task.fixes, Some(FixOf::Ci { head: h, .. }) if *h == head),
        "{:?}",
        task.fixes
    );
}

/// The user approves hold `hold`.
fn approve(fx: &mut Fixture, hold: &str) {
    let event = OrchEvent::ApproveHold {
        reply: fx.reply(),
        run_id: RUN_ID.into(),
        hold: hold.into(),
    };
    let effects = fx.next(EventKind::Orch(event));
    assert!(replies(&effects)[0].is_ok(), "{effects:#?}");
}

#[test]
fn a_propagate_push_during_a_ci_fix_adds_no_second_fix() {
    let (mut fx, _) = two_stages(true);
    let fix1 = red(&mut fx, 2, 12, RUN_A).expect("a CI fix of stage 2");
    // Stage 1 moves; 9.1's propagate carries it into stage 2, which is pushed.
    set_stage_head(fx.run_mut(), 1, &commit(50));
    fx.tick();
    land_propagates(&mut fx);
    fx.tick();
    push_lands(&mut fx, 1, &commit(50));
    let h2 = fx.run().stage_head(2).unwrap().to_string();
    push_lands(&mut fx, 2, &h2);
    assert_eq!(red(&mut fx, 2, 12, RUN_B), None, "no second fix");
    assert_eq!(ci_fixes(&fx), std::slice::from_ref(&fix1));
    joined(&fx, 2, &fix1, &h2);
    finishes_then_judged_again(&mut fx, 2, 12, &fix1);
}

#[test]
fn a_review_fix_push_during_a_ci_fix_adds_no_second_fix() {
    let mut fx = watched();
    fx.run_mut().delivery.limits.reviewers = vec!["alice".into()];
    fx.run_mut().delivery.limits.review_batch_secs = 1;
    let fix1 = red(&mut fx, 1, PR, RUN_A).expect("a CI fix");
    // A reviewer's comment on a file the CI fix does not own becomes a review fix (held
    // for approval: no task of the stage owns it), which the user approves; it merges
    // beside the CI fix and is pushed.
    let mut v = view(&commit(1));
    v.threads = vec![on(
        "Cargo.toml",
        vec![noted(30, "alice", "bump the version")],
    )];
    let (at, _) = poll_with(&mut fx, v);
    fx.send(at + 1, EventKind::Tick);
    let review: Vec<String> = (fx.run().tasks.iter())
        .filter(|t| t.origin == proto::TaskOrigin::Review)
        .map(|t| t.id().to_string())
        .collect();
    assert_eq!(review.len(), 1, "{review:?}");
    approve(&mut fx, &format!("hold-{}", review[0]));
    merge_fix(&mut fx, &review[0], &commit(5));
    push_lands(&mut fx, 1, &commit(5));
    assert_eq!(red(&mut fx, 1, PR, RUN_B), None, "no second fix");
    assert_eq!(ci_fixes(&fx), std::slice::from_ref(&fix1));
    joined(&fx, 1, &fix1, &commit(5));
    finishes_then_judged_again(&mut fx, 1, PR, &fix1);
}

#[test]
fn a_base_sync_push_during_a_ci_fix_adds_no_second_fix() {
    let mut fx = watched();
    let fix1 = red(&mut fx, 1, PR, RUN_A).expect("a CI fix");
    // The base moved and conflicts: it is merged into stage 1, which is pushed.
    poll_with(&mut fx, conflicting(PR, &commit(1)));
    fetched(&mut fx, &commit(50), None);
    let (op, _) = base_sync(&fx);
    fx.done(op, merged_at(&commit(60)));
    fx.tick();
    push_lands(&mut fx, 1, &commit(60));
    assert_eq!(red(&mut fx, 1, PR, RUN_B), None, "no second fix");
    assert_eq!(ci_fixes(&fx), std::slice::from_ref(&fix1));
    joined(&fx, 1, &fix1, &commit(60));
    finishes_then_judged_again(&mut fx, 1, PR, &fix1);
}
