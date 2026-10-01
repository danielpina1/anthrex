//! M8b.12: deciders inside a run (M8b decisions 18 and 20). A failed check's rung waits
//! for its check summary, a `Decide` op that holds a reader slot; queued deciders start
//! before queued reviewers, fall back after `slot_wait_secs`, and are answered inline
//! when deciders are off. Block classification, the restart and the accounting are in
//! `deciders_block.rs`.

use proto::{DeciderSource, TaskState, TokenUsage};

use super::fixture::*;
use super::gates::{check_result, only_op};
use super::holds::delivers;
use super::liveness::assert_alive;
use super::merge::{claim, commit, head_of, pending_one, window_of};
use crate::decider::fallback::fallback_decision;
use crate::decider::{DeciderAnswer, DeciderKind, DeciderRequest, Decision};
use crate::run::contract::{candidate_red_message, check_failed_message};
use crate::run::engine::schedule::readers_busy;
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::model::{CheckRecord, OpId};

/// The failed check's output every test gives.
pub(super) const TAIL: &str = "compiling\ntest a::works ... FAILED";

/// `review.small = "off"` (a check-mode S task goes from its check to the merge queue)
/// and the deciders on, as the config's default leaves them.
pub(super) fn deciding_config() -> config::Orchestrator {
    config::Orchestrator {
        review_small: false,
        ..config::Orchestrator::default()
    }
}

/// A check-mode S task owning `docs/<id>/**`, outside `source`, so with
/// `review.small = "off"` its check is its only gate before the merge queue.
pub(super) fn check_task(id: &str) -> String {
    super::merge::doc_task(id, "")
}

/// A running run of `tasks` on `profile` under `config`, every worker launched.
pub(super) fn running(
    profile: &str,
    tasks: &[String],
    config: config::Orchestrator,
) -> (Fixture, Vec<(String, u32)>) {
    let mut fx = Fixture::deciding(&plan_with(profile, tasks), config);
    fx.ready(true);
    let windows = fx.launch_all();
    (fx, windows)
}

/// `id` claims done and its check fails; the effects of the failing step.
pub(super) fn check_fails(fx: &mut Fixture, id: &str, window: u32) -> Vec<Effect> {
    claim(fx, id, window, &head_of(id));
    let (op, _) = pending_one(fx, "Check", Some(id));
    fx.done(op, check_result(false))
}

/// The usage the decider tests report.
pub(super) fn usage(n: u64) -> TokenUsage {
    TokenUsage {
        input: 100 * n,
        output: 10 * n,
        cache_read: n,
        cache_write: 2 * n,
    }
}

/// A decider's summary with `lines`.
pub(super) fn summary(lines: &[&str]) -> Decision {
    Decision {
        kind: DeciderKind::CheckSummary,
        answer: DeciderAnswer::CheckSummary {
            lines: lines.iter().map(|l| l.to_string()).collect(),
        },
        source: DeciderSource::Decider,
        fallback_reason: None,
        usage: Some(usage(1)),
        secs: 3,
    }
}

/// The request and op of the only `Decide` among `effects`.
pub(super) fn decide_op(effects: &[Effect]) -> (OpId, u64, DeciderRequest) {
    match only_op(effects, "Decide") {
        (
            op,
            OpKind::Decide {
                decider_id,
                request,
                ..
            },
        ) => (op, decider_id, request),
        _ => unreachable!(),
    }
}

/// The M8a record of the failed check `check_result(false)` gives, at `at`.
fn record(at: u64, on_candidate: bool, source: Option<DeciderSource>) -> CheckRecord {
    CheckRecord {
        at,
        ok: false,
        code: Some(101),
        timed_out: false,
        tail: TAIL.into(),
        secs: 42,
        on_candidate,
        summary: None,
        summary_source: source,
        tier: None,
    }
}

#[test]
fn failed_check_defers_the_bounce_until_the_summary() {
    let (mut fx, windows) = running(PROFILE, &[check_task("t1")], deciding_config());
    let window = window_of(&windows, "t1");
    let effects = check_fails(&mut fx, "t1", window);
    let t1 = fx.task("t1");
    // Counted on the ladder at once; the rung's action waits for the summary.
    assert_eq!((t1.failures, t1.bounces.check, t1.rung), (1, 1, 0));
    assert_eq!(t1.state, TaskState::Check);
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    assert!(fx.run().outbox.is_empty());
    let (op, decider_id, request) = decide_op(&effects);
    assert_eq!(
        request,
        DeciderRequest::CheckSummary(crate::decider::CheckSummaryInput {
            task_id: "t1".into(),
            command: "cargo test".into(),
            code: Some(101),
            timed_out: false,
            tail: TAIL.into(),
        })
    );
    let pending = t1.pending_failure.clone().expect("a deferred failure");
    assert_eq!((pending.rung, pending.check_index), (1, 0));
    assert_eq!(pending.decider_id, decider_id);
    // No second check while the first one's failure waits.
    let effects = fx.tick();
    assert!(ops_in(&effects, "Check").is_empty(), "{effects:#?}");

    let effects = fx.decided(op, summary(&["error: a::works failed", "at src/a.rs:3"]));
    let text = delivers(&effects).pop().expect("the bounce is delivered");
    assert!(
        text.contains("Summary of its output:\nerror: a::works failed\nat src/a.rs:3\n"),
        "{text}"
    );
    assert!(!text.contains("Last 40 lines"), "{text}");
    assert!(!text.contains("compiling"), "{text}");
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.rung, t1.failures), (TaskState::Working, 1, 1));
    assert_eq!(t1.pending_failure, None);
    let check = &t1.checks[0];
    assert_eq!(check.summary_source, Some(DeciderSource::Decider));
    assert_eq!(
        check.summary.as_deref(),
        Some("error: a::works failed\nat src/a.rs:3")
    );
    assert_eq!(t1.failure_log.last(), Some(&text));
    assert_alive(&fx);
}

#[test]
fn summary_fallback_uses_the_tail_and_is_marked() {
    let (mut fx, windows) = running(PROFILE, &[check_task("t1")], deciding_config());
    let window = window_of(&windows, "t1");
    let effects = check_fails(&mut fx, "t1", window);
    let at = fx.now;
    let (op, _, request) = decide_op(&effects);
    let fallback = fallback_decision(&request, "the decider timed out after 90 s".into());
    let effects = fx.decided(op, fallback);
    let expected = record(at, false, Some(DeciderSource::Fallback));
    assert_eq!(fx.task("t1").checks, vec![expected.clone()]);
    let text = check_failed_message("cargo test", &expected);
    assert_eq!(
        text,
        "[anthrex] The check failed (exit 101): cargo test\nLast 40 lines:\ncompiling\ntest a::works ... FAILED\nFix it, commit, then call task_done again."
    );
    assert_eq!(delivers(&effects), vec![text]);
    assert!(
        fx.task("t1")
            .history
            .iter()
            .any(|e| e.text == "check summary: fallback (the decider timed out after 90 s)"),
        "{:#?}",
        fx.task("t1").history
    );
    assert_alive(&fx);
}

#[test]
fn candidate_red_uses_the_summary_and_the_queue_moves_on() {
    let tasks = [check_task("t1"), check_task("t2")];
    let (mut fx, windows) = running(PROFILE, &tasks, deciding_config());
    for id in ["t1", "t2"] {
        claim(&mut fx, id, window_of(&windows, id), &head_of(id));
        let (op, _) = pending_one(&fx, "Check", Some(id));
        fx.done(op, check_result(true));
    }
    assert_eq!(fx.run().merge_queue, vec!["t1", "t2"]);
    let (op, _) = pending_one(&fx, "MergeCandidate", Some("t1"));
    let effects = fx.done(
        op,
        OpResult::CandidateRed {
            code: Some(101),
            timed_out: false,
            tail: TAIL.into(),
            secs: 42,
            tier: None,
        },
    );
    // The queue moves on before the summary comes; t1 waits outside it.
    let candidates = ops_in(&effects, "MergeCandidate");
    assert_eq!(candidates.len(), 1, "{effects:#?}");
    let (next, _) = pending_one(&fx, "MergeCandidate", Some("t2"));
    assert_eq!(candidates[0].0, next);
    assert!(delivers(&effects).is_empty(), "{effects:#?}");
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::MergeQueue);
    assert_eq!((t1.failures, t1.bounces.merge, t1.rung), (1, 1, 0));
    assert_eq!(fx.run().merge_queue, vec!["t2"]);
    let (op, _, _) = decide_op(&effects);

    // t2 merges meanwhile; t1's failure is unaffected.
    fx.done(
        next,
        OpResult::Merged {
            commit: commit(2),
            tier: None,
        },
    );
    let effects = fx.decided(op, summary(&["merged result: a::works FAILED"]));
    let checks = &fx.task("t1").checks;
    assert_eq!(checks.len(), 2, "the passing check, then the candidate's");
    let mut expected = record(checks[1].at, true, Some(DeciderSource::Decider));
    expected.summary = Some("merged result: a::works FAILED".into());
    assert_eq!(checks[1], expected);
    let text = candidate_red_message("cargo test", &expected);
    assert_eq!(
        text,
        "[anthrex] Your work merged cleanly into the run branch, but the check failed on the merged result (exit 101): cargo test\nSummary of its output:\nmerged result: a::works FAILED\nFix it in your worktree, commit, then call task_done again."
    );
    assert_eq!(delivers(&effects), vec![text]);
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.rung), (TaskState::Working, 1));
    assert_alive(&fx);
}

/// Three check-mode S tasks with `max_readers = 1` and review on: t2's review takes the
/// reader slot, then t1's check fails. Returns the fixture, t1's decider id, and the
/// `PrepareReview` op holding the slot.
pub(super) fn slot_taken() -> (Fixture, u64, OpId) {
    let tasks = [check_task("t1"), check_task("t2"), check_task("t3")];
    let config = config::Orchestrator::default();
    let (mut fx, windows) = running(&profile_with("max_readers = 1"), &tasks, config);
    claim(&mut fx, "t2", window_of(&windows, "t2"), &head_of("t2"));
    let (op, _) = pending_one(&fx, "Check", Some("t2"));
    fx.done(op, check_result(true));
    let (review, _) = pending_one(&fx, "PrepareReview", Some("t2"));
    assert_eq!(readers_busy(fx.run()), 1);
    let effects = check_fails(&mut fx, "t1", window_of(&windows, "t1"));
    assert!(ops_in(&effects, "Decide").is_empty(), "{effects:#?}");
    let queued = fx.queued_deciders();
    assert_eq!(queued.len(), 1);
    assert_eq!(queued[0].queued_at, fx.now);
    assert_eq!(queued[0].task_ids, vec!["t1"]);
    // t3 reaches review too, and waits for a reader slot.
    claim(&mut fx, "t3", window_of(&windows, "t3"), &head_of("t3"));
    let (op, _) = pending_one(&fx, "Check", Some("t3"));
    let effects = fx.done(op, check_result(true));
    assert!(ops_in(&effects, "PrepareReview").is_empty(), "{effects:#?}");
    // m5: a check waiting for its queued decider is alive.
    super::liveness::assert_alive(&fx);
    (fx, queued[0].decider_id, review)
}

#[test]
fn deciders_take_a_reader_slot_before_reviewers() {
    let (mut fx, decider_id, review) = slot_taken();
    // t2's review cannot be prepared: the slot frees, and the decider takes it.
    let effects = fx.done(
        review,
        OpResult::Failed {
            message: "no space".into(),
        },
    );
    let (_, id, _) = decide_op(&effects);
    assert_eq!(id, decider_id);
    assert!(ops_in(&effects, "PrepareReview").is_empty(), "{effects:#?}");
    assert!(fx.queued_deciders().is_empty());
    assert_eq!(readers_busy(fx.run()), 1);
    let snapshot = crate::run::snapshot::snapshot(&fx.state, fx.now);
    assert_eq!(snapshot.runs[0].readers_busy, 1);

    // Its answer frees the slot for t3's reviewer.
    let (op, _) = pending_one(&fx, "Decide", Some("t1"));
    let effects = fx.decided(op, summary(&["one line"]));
    assert_eq!(ops_in(&effects, "PrepareReview").len(), 1, "{effects:#?}");
    assert_alive(&fx);
}

#[test]
fn a_decider_waiting_past_slot_wait_falls_back_on_tick() {
    let (mut fx, _, _) = slot_taken();
    let queued_at = fx.queued_deciders()[0].queued_at;
    let effects = fx.send(queued_at + 29, crate::run::engine::EventKind::Tick);
    assert!(delivers(&effects).is_empty());
    assert_eq!(fx.queued_deciders().len(), 1);

    let effects = fx.send(queued_at + 30, crate::run::engine::EventKind::Tick);
    assert!(fx.queued_deciders().is_empty());
    assert!(ops_in(&effects, "Decide").is_empty(), "{effects:#?}");
    let t1 = fx.task("t1");
    let check = t1.checks[0].clone();
    assert_eq!(check.summary_source, Some(DeciderSource::Fallback));
    assert_eq!(
        delivers(&effects),
        vec![check_failed_message("cargo test", &check)]
    );
    assert_eq!((t1.state, t1.rung), (TaskState::Working, 1));
    assert!(
        t1.history
            .iter()
            .any(|e| e.text == "check summary: fallback (no reader slot was free within 30 s)"),
        "{:#?}",
        t1.history
    );
    assert_eq!((fx.run().decider_calls, fx.run().decider_fallbacks), (1, 1));
    assert_alive(&fx);
}

#[test]
fn mode_off_decides_inline_without_an_op() {
    let mut config = deciding_config();
    config.deciders.mode = proto::DeciderMode::Off;
    let (mut fx, windows) = running(PROFILE, &[check_task("t1")], config);
    let effects = check_fails(&mut fx, "t1", window_of(&windows, "t1"));
    assert!(ops_in(&effects, "Decide").is_empty(), "{effects:#?}");
    assert!(fx.queued_deciders().is_empty());
    let expected = record(fx.now, false, Some(DeciderSource::Fallback));
    let t1 = fx.task("t1");
    assert_eq!(t1.checks, vec![expected.clone()]);
    assert_eq!(
        delivers(&effects),
        vec![check_failed_message("cargo test", &expected)]
    );
    assert_eq!(
        (t1.state, t1.rung, t1.pending_failure.clone()),
        (TaskState::Working, 1, None)
    );
    // Off is no call: nothing is counted, and M8a's history is unchanged.
    assert_eq!((fx.run().decider_calls, fx.run().decider_fallbacks), (0, 0));
    assert!(
        !t1.history
            .iter()
            .any(|e| e.text.starts_with("check summary"))
    );
    assert_alive(&fx);
}
