//! M8b.12 review fixes: a classification keyed by its decider (I1), a failed `Decide`
//! (m1), a paused run (m2), a cancelled task's queued decider (m3), an answer of the
//! wrong kind (m4), and the two guards on a deferred rung (m6). Waiting for a decider
//! is alive (m5): each wait here ends with the liveness check.

use proto::{BlockReason, DeciderSource, PlanEdit, TaskState};
use serde_json::json;

use super::control::{override_task, retry};
use super::deciders::{
    check_fails, check_task, decide_op, deciding_config, running, slot_taken, summary,
};
use super::deciders_block::{REASON, classified, untyped_block, working};
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::holds::delivers;
use super::liveness::assert_alive;
use super::merge::window_of;
use super::turns::killed_exit;
use crate::decider::BlockKind;
use crate::run::contract::{blocked_recorded, check_failed_message};
use crate::run::engine::deciders::on_decided;
use crate::run::engine::{Effect, EventKind, OpResult};

/// The reviewer's scenario: an untyped block's decider is still in flight when the user
/// retries the task, and the fresh worker blocks with a typed `question`. The late
/// answer must not reclassify the typed block.
#[test]
fn a_late_classification_never_touches_a_typed_block() {
    let (mut fx, window) = working();
    let op = untyped_block(&mut fx, window);
    assert_alive(&fx);
    retry(&mut fx, "t1");
    assert_eq!(
        fx.task("t1").pending_classification,
        None,
        "retry clears it"
    );
    let effects = killed_exit(&mut fx, window);
    let (diff, _) = super::gates::only_op(&effects, "DiffSoFar");
    fx.done(
        diff,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let fresh = fx.complete_windows()[0].1;
    let args = json!({"kind": "question", "reason": REASON});
    let effects = fx.tool(fresh, "task_blocked", args);
    assert_eq!(replies(&effects), vec![Ok(blocked_recorded("question"))]);

    let effects = fx.decided(op, classified(BlockKind::MisSized));
    assert!(!effects.contains(&Effect::KillWindow { window_id: fresh }));
    let t1 = fx.task("t1");
    assert_eq!(
        t1.block.as_ref().map(|b| b.reason),
        Some(BlockReason::Question)
    );
    assert_eq!((t1.rung, t1.block_source), (2, None));
    assert_alive(&fx);
}

/// I1: after an answer, the worker blocks again with the same untyped reason. The first
/// block's late answer is not the second block's classification; the second's is.
#[test]
fn an_earlier_classification_does_not_apply_to_a_later_block() {
    let (mut fx, window) = working();
    let first = untyped_block(&mut fx, window);
    let answer = PlanEdit::Answer {
        task_id: "t1".into(),
        text: "try again".into(),
    };
    edit(&mut fx, vec![answer]);
    fx.turn_completed(window);
    let second = untyped_block(&mut fx, window);
    let effects = fx.decided(first, classified(BlockKind::MisSized));
    assert!(!effects.contains(&Effect::KillWindow { window_id: window }));
    let t1 = fx.task("t1");
    assert_eq!(
        t1.block.as_ref().map(|b| b.reason),
        Some(BlockReason::Question)
    );
    assert!(t1.pending_classification.is_some());
    fx.decided(second, classified(BlockKind::Environment));
    let t1 = fx.task("t1");
    assert_eq!(
        t1.block.as_ref().map(|b| b.reason),
        Some(BlockReason::Environment)
    );
    assert_eq!(t1.block_source, Some(DeciderSource::Decider));
    assert_alive(&fx);
}

/// An override sends the blocked task on: its classification no longer applies.
#[test]
fn an_override_ends_the_classification() {
    let (mut fx, window) = working();
    let op = untyped_block(&mut fx, window);
    let effects = override_task(&mut fx, "t1", "the user knows");
    let (count, _) = super::gates::only_op(&effects, "CountCommits");
    fx.done(
        count,
        OpResult::Commits {
            count: 1,
            head: HEAD.into(),
        },
    );
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.pending_classification),
        (TaskState::MergeQueue, None)
    );
    fx.decided(op, classified(BlockKind::Environment));
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.block.clone(), t1.block_source),
        (TaskState::MergeQueue, None, None)
    );
}

/// m1: the driver answers `Failed` when it cannot run the op (its intent line could
/// not be written): the fallback applies, so the task is never stuck.
#[test]
fn a_failed_decide_applies_the_fallback() {
    let (mut fx, windows) = running(PROFILE, &[check_task("t1")], deciding_config());
    let effects = check_fails(&mut fx, "t1", window_of(&windows, "t1"));
    assert_alive(&fx);
    let (op, _, _) = decide_op(&effects);
    let message = "its intent line could not be appended".to_string();
    let effects = fx.done(op, OpResult::Failed { message });
    let t1 = fx.task("t1");
    let check = t1.checks[0].clone();
    assert_eq!(check.summary_source, Some(DeciderSource::Fallback));
    assert_eq!(
        delivers(&effects),
        vec![check_failed_message("cargo test", &check)]
    );
    assert_eq!(
        (t1.state, t1.rung, t1.pending_failure.clone()),
        (TaskState::Working, 1, None)
    );
    assert!(
        t1.history.iter().any(|e| e.text
            == "check summary: fallback (the decider could not start: its intent line could not be appended)"),
        "{:#?}",
        t1.history
    );
    assert_alive(&fx);
}

/// m2: no decider starts while the run is paused (ruling T14-I2); the resume starts it.
#[test]
fn no_decider_starts_while_paused() {
    let (mut fx, windows) = running(PROFILE, &[check_task("t1")], deciding_config());
    let window = window_of(&windows, "t1");
    super::merge::claim(&mut fx, "t1", window, &super::merge::head_of("t1"));
    let (check, _) = super::merge::pending_one(&fx, "Check", Some("t1"));
    edit(&mut fx, vec![PlanEdit::Pause]);
    assert_eq!(fx.run().state, proto::RunState::Paused);
    let effects = fx.done(check, super::gates::check_result(false));
    assert!(ops_in(&effects, "Decide").is_empty(), "{effects:#?}");
    assert_eq!(fx.queued_deciders().len(), 1);
    let effects = fx.tick();
    assert!(ops_in(&effects, "Decide").is_empty(), "{effects:#?}");
    assert_alive(&fx);
    let effects = edit(&mut fx, vec![PlanEdit::Resume]);
    decide_op(&effects);
    assert!(fx.queued_deciders().is_empty());
    assert_alive(&fx);
}

/// m3: a cancelled task's queued decider is dropped, by the `cancel_task` edit and by
/// `run cancel`; the slot it would have taken goes to no decider.
#[test]
fn a_cancelled_task_drops_its_queued_decider() {
    let (mut fx, _, review) = slot_taken();
    assert_eq!(fx.queued_deciders().len(), 1);
    edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "t1".into(),
        }],
    );
    assert!(fx.queued_deciders().is_empty());
    let message = "no space".to_string();
    let effects = fx.done(review, OpResult::Failed { message });
    assert!(ops_in(&effects, "Decide").is_empty(), "{effects:#?}");

    let (mut fx, _, _) = slot_taken();
    let reply = fx.reply();
    let effects = fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    });
    // t2's reviewer slot frees in the same step: the dropped decider must not take it.
    assert!(ops_in(&effects, "Decide").is_empty(), "{effects:#?}");
    assert!(fx.queued_deciders().is_empty());
}

/// m4: an answer of another kind than its request (unreachable from the driver today)
/// is the request's fallback, so the deferred rung is still taken.
#[test]
fn an_answer_of_the_wrong_kind_is_the_fallback() {
    let (mut fx, windows) = running(PROFILE, &[check_task("t1")], deciding_config());
    let effects = check_fails(&mut fx, "t1", window_of(&windows, "t1"));
    let (_, id, request) = decide_op(&effects);
    let now = fx.now + 1;
    let wrong = classified(BlockKind::Environment);
    let effects = on_decided(fx.run_mut(), id, &["t1".to_string()], &request, wrong, now);
    let t1 = fx.task("t1");
    let check = t1.checks[0].clone();
    assert_eq!(check.summary_source, Some(DeciderSource::Fallback));
    assert_eq!(
        (t1.state, t1.rung, t1.pending_failure.clone()),
        (TaskState::Working, 1, None)
    );
    assert_eq!(
        fx.run().outbox.last().map(|m| m.text.clone()),
        Some(check_failed_message("cargo test", &check))
    );
    assert!(
        t1.history.iter().any(|e| e.text
            == "check summary: fallback (the decider's answer does not match its request)")
    );
    assert_eq!((fx.run().decider_calls, fx.run().decider_fallbacks), (1, 1));
    assert!(ops_in(&effects, "Decide").is_empty(), "{effects:#?}");
}

/// m6: an answer for another decider than the one the failure waits for is not
/// applied; nor is one for a red candidate that is back in the merge queue.
#[test]
fn a_deferred_rung_is_taken_only_by_its_own_answer() {
    let (mut fx, windows) = running(PROFILE, &[check_task("t1")], deciding_config());
    let effects = check_fails(&mut fx, "t1", window_of(&windows, "t1"));
    let (_, id, request) = decide_op(&effects);
    let now = fx.now + 1;
    let other = summary(&["someone else's"]);
    on_decided(
        fx.run_mut(),
        id + 1,
        &["t1".to_string()],
        &request,
        other,
        now,
    );
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.rung), (TaskState::Check, 0));
    assert_eq!(t1.pending_failure.as_ref().map(|p| p.decider_id), Some(id));
    assert_eq!(t1.checks[0].summary_source, None);

    let tasks = [check_task("t1")];
    let (mut fx, windows) = running(PROFILE, &tasks, deciding_config());
    let window = window_of(&windows, "t1");
    super::merge::claim(&mut fx, "t1", window, &super::merge::head_of("t1"));
    let (check, _) = super::merge::pending_one(&fx, "Check", Some("t1"));
    fx.done(check, super::gates::check_result(true));
    let (candidate, _) = super::merge::pending_one(&fx, "MergeCandidate", Some("t1"));
    let red = OpResult::CandidateRed {
        code: Some(1),
        timed_out: false,
        tail: "red".into(),
        secs: 1,
        tier: None,
    };
    let effects = fx.done(candidate, red);
    assert_alive(&fx);
    let (op, _, _) = decide_op(&effects);
    fx.run_mut().merge_queue.push("t1".into());
    fx.decided(op, summary(&["line"]));
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.rung), (TaskState::MergeQueue, 0));
    assert_eq!(
        t1.checks.last().and_then(|c| c.summary_source),
        Some(DeciderSource::Decider)
    );
}

/// Re-review r1: a new `task_blocked` clears the earlier block's `block_source`, so a
/// typed block never shows a decider's classification.
#[test]
fn a_new_block_clears_the_earlier_block_source() {
    let (mut fx, window) = working();
    let op = untyped_block(&mut fx, window);
    fx.decided(op, classified(BlockKind::Question));
    assert_eq!(fx.task("t1").block_source, Some(DeciderSource::Decider));
    let answer = PlanEdit::Answer {
        task_id: "t1".into(),
        text: "use the other port".into(),
    };
    edit(&mut fx, vec![answer]);
    fx.turn_completed(window);
    let args = json!({"kind": "environment", "reason": REASON});
    let effects = fx.tool(window, "task_blocked", args);
    assert_eq!(replies(&effects), vec![Ok(blocked_recorded("environment"))]);
    let snapshot = crate::run::snapshot::snapshot(&fx.state, fx.now);
    let task = &snapshot.runs[0].tasks[0];
    assert_eq!(task.block_source, None);
    assert_eq!(fx.task("t1").block_source, None);
    assert_alive(&fx);
}
