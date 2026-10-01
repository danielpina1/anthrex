//! M8b.12: classifying a free-text `task_blocked` (M8b decision 21), a decider op a
//! restart dropped (decision 18, through M8a's reconcile), the accounting, and the
//! reviewer prompt's summary (decision 20).

use std::time::Duration;

use proto::{BlockReason, DeciderSource, PlanEdit, Size, TaskState};
use serde_json::json;

use super::control::resume;
use super::deciders::{
    TAIL, check_fails, check_task, decide_op, deciding_config, running, summary, usage,
};
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::liveness::assert_alive;
use super::merge::window_of;
use crate::decider::fallback::fallback_decision;
use crate::decider::{
    BlockKind, BlockedReasonInput, DeciderAnswer, DeciderKind, DeciderRequest, Decision,
};
use crate::run::contract::{blocked_recorded, reviewer_prompt};
use crate::run::engine::{Effect, EngineState, EventKind};
use crate::run::journal::JournalLine;
use crate::run::model::CheckRecord;
use crate::run::reconcile::{Reconciled, reconcile};

const CLASSIFYING: &str = "Blocked recorded (classifying). Stop and wait for an answer.";
pub(super) const REASON: &str = "the fixture server will not start";

/// A working check-mode `t1` with the deciders on; its window.
pub(super) fn working() -> (Fixture, u32) {
    let (fx, windows) = running(PROFILE, &[check_task("t1")], deciding_config());
    let window = window_of(&windows, "t1");
    (fx, window)
}

pub(super) fn classified(kind: BlockKind) -> Decision {
    Decision {
        kind: DeciderKind::BlockedReason,
        answer: DeciderAnswer::BlockedReason {
            kind,
            reason: "judged".into(),
        },
        source: DeciderSource::Decider,
        fallback_reason: None,
        usage: Some(usage(2)),
        secs: 2,
    }
}

/// `task_blocked` with no kind: blocked as a question at once, its classification
/// asked. Returns the `Decide` op.
pub(super) fn untyped_block(fx: &mut Fixture, window: u32) -> u64 {
    let effects = fx.tool(window, "task_blocked", json!({"reason": REASON}));
    assert_eq!(replies(&effects), vec![Ok(CLASSIFYING.to_string())]);
    let t1 = fx.task("t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert_eq!(
        t1.block.as_ref().map(|b| b.reason),
        Some(BlockReason::Question)
    );
    assert!(t1.pending_classification.is_some());
    assert_eq!(t1.block_source, None);
    let (op, _, request) = decide_op(&effects);
    assert_eq!(
        request,
        DeciderRequest::BlockedReason(BlockedReasonInput {
            task_id: "t1".into(),
            title: "Title t1".into(),
            reason: REASON.into(),
        })
    );
    op
}

#[test]
fn an_unclassified_block_is_classified() {
    // A question stays a question.
    let (mut fx, window) = working();
    let op = untyped_block(&mut fx, window);
    fx.decided(op, classified(BlockKind::Question));
    let t1 = fx.task("t1");
    assert_eq!(
        t1.block.as_ref().map(|b| b.reason),
        Some(BlockReason::Question)
    );
    assert_eq!(
        (t1.block_source, t1.pending_classification),
        (Some(DeciderSource::Decider), None)
    );
    assert_alive(&fx);

    // An environment problem changes the reason, and keeps the worker's text.
    let (mut fx, window) = working();
    let op = untyped_block(&mut fx, window);
    fx.decided(op, classified(BlockKind::Environment));
    let t1 = fx.task("t1");
    let block = t1.block.clone().unwrap();
    assert_eq!(
        (block.reason, block.text.as_str()),
        (BlockReason::Environment, REASON)
    );
    assert_eq!(t1.block_source, Some(DeciderSource::Decider));
    assert_alive(&fx);

    // Mis-sized is rung 3, exactly as a typed `mis_sized`.
    let (mut fx, window) = working();
    let op = untyped_block(&mut fx, window);
    let effects = fx.decided(op, classified(BlockKind::MisSized));
    let t1 = fx.task("t1");
    let block = t1.block.clone().unwrap();
    assert_eq!(block.reason, BlockReason::MisSized);
    assert_eq!(
        block.text,
        format!("the worker reported the task mis-sized: {REASON}")
    );
    assert_eq!(
        (t1.rung, t1.size, t1.raised_size),
        (3, Size::M, Some(Size::M))
    );
    assert_eq!(t1.block_source, Some(DeciderSource::Decider));
    assert!(effects.contains(&Effect::KillWindow { window_id: window }));
    assert_alive(&fx);
}

#[test]
fn a_typed_kind_is_never_reclassified() {
    for (kind, reason) in [
        ("question", BlockReason::Question),
        ("environment", BlockReason::Environment),
    ] {
        let (mut fx, window) = working();
        let effects = fx.tool(
            window,
            "task_blocked",
            json!({"kind": kind, "reason": REASON}),
        );
        assert_eq!(replies(&effects), vec![Ok(blocked_recorded(kind))]);
        assert!(ops_in(&effects, "Decide").is_empty(), "{effects:#?}");
        assert!(fx.queued_deciders().is_empty());
        let t1 = fx.task("t1");
        assert_eq!(t1.block.as_ref().map(|b| b.reason), Some(reason));
        assert_eq!((t1.block_source, t1.pending_classification), (None, None));
        assert_alive(&fx);
    }
}

#[test]
fn an_answer_before_classification_wins() {
    let (mut fx, window) = working();
    let op = untyped_block(&mut fx, window);
    let answer = PlanEdit::Answer {
        task_id: "t1".into(),
        text: "use the other port".into(),
    };
    edit(&mut fx, vec![answer]);
    let t1 = fx.task("t1");
    assert_eq!(
        (t1.state, t1.pending_classification),
        (TaskState::Working, None)
    );
    let effects = fx.decided(op, classified(BlockKind::Environment));
    assert!(!effects.contains(&Effect::KillWindow { window_id: window }));
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.block.clone()), (TaskState::Working, None));
    assert_eq!(t1.block_source, None);
    // The call still happened, and is counted.
    assert_eq!(fx.run().decider_calls, 1);
    assert_alive(&fx);
}

#[test]
fn a_restart_requeues_a_dropped_decider_op() {
    let (mut fx, window) = working();
    let effects = check_fails(&mut fx, "t1", window);
    let (op, decider_id, request) = decide_op(&effects);
    // The daemon dies after the op's intent line: M8a's reconcile answers it.
    let run = fx.run().clone();
    // The safety rule: reconcile kills recorded session pids; this run records none.
    assert!(
        run.tasks
            .iter()
            .flat_map(|t| &t.rounds)
            .all(|r| r.pid.is_none())
    );
    let journal: Vec<JournalLine> = run
        .pending_ops
        .values()
        .map(|p| JournalLine::Intent {
            op: p.op,
            kind: p.kind.clone(),
        })
        .collect();
    let git = std::ffi::OsStr::new("/nonexistent/anthrex-test/git");
    let reconciled = reconcile(git, &run, &journal, &[], Duration::from_secs(1));
    assert!(
        reconciled.ops.contains(&(op, Reconciled::NotStarted)),
        "{reconciled:?}"
    );
    fx.state = EngineState::default();
    let effects = fx.next(EventKind::Restore {
        runs: vec![run],
        replay: reconciled.replay(RUN_ID),
        held: Vec::new(),
    });
    assert!(
        ops_in(&effects, "Decide").is_empty(),
        "paused: {effects:#?}"
    );
    let queued = fx.queued_deciders();
    assert_eq!(queued.len(), 1);
    assert_eq!(
        (queued[0].decider_id, queued[0].request.clone()),
        (decider_id, request.clone())
    );
    assert!(!fx.run().pending_ops.contains_key(&op));

    let effects = resume(&mut fx);
    let (again, id, asked) = decide_op(&effects);
    assert_ne!(again, op);
    assert_eq!((id, asked), (decider_id, request));
    // The old op's late answer is ignored; the new one's takes the rung.
    fx.decided(op, summary(&["stale"]));
    assert_eq!(fx.task("t1").state, TaskState::Check);
    fx.decided(again, summary(&["fresh line"]));
    let t1 = fx.task("t1");
    assert_eq!((t1.state, t1.rung), (TaskState::Working, 1));
    assert!(
        t1.failure_log
            .last()
            .is_some_and(|t| t.contains("Summary of its output:\nfresh line\n")),
        "{:#?}",
        t1.failure_log
    );
    assert_alive(&fx);
}

#[test]
fn decider_usage_is_counted_on_task_and_run() {
    let (mut fx, window) = working();
    let effects = check_fails(&mut fx, "t1", window);
    let (op, _, _) = decide_op(&effects);
    fx.decided(op, summary(&["one"]));
    assert_eq!(fx.task("t1").decider_usage, usage(1));
    assert_eq!(fx.run().decider_usage, usage(1));
    assert_eq!((fx.run().decider_calls, fx.run().decider_fallbacks), (1, 0));

    // A fallback still spent what its turn used.
    fx.turn_completed(window);
    let effects = check_fails(&mut fx, "t1", window);
    let (op, _, request) = decide_op(&effects);
    let mut fallback = fallback_decision(&request, "the decider's answer is not JSON: x".into());
    fallback.usage = Some(usage(3));
    fx.decided(op, fallback);
    assert_eq!(fx.task("t1").decider_usage, usage(4));
    assert_eq!(fx.run().decider_usage, usage(4));
    assert_eq!((fx.run().decider_calls, fx.run().decider_fallbacks), (2, 1));
    let snapshot = crate::run::snapshot::snapshot(&fx.state, fx.now);
    let task = &snapshot.runs[0].tasks[0];
    assert_eq!(task.decider_usage, Some(usage(4)));
    let check = task.last_check.clone().unwrap();
    assert_eq!(check.summary_source, Some(DeciderSource::Fallback));
    assert_eq!(check.decider_summary, None);
    assert_alive(&fx);
}

#[test]
fn reviewer_prompt_uses_the_latest_summary() {
    let (fx, _) = working();
    let mut task = fx.task("t1").clone();
    let check = |summary: Option<&str>, source| CheckRecord {
        at: 1,
        ok: false,
        code: Some(1),
        timed_out: false,
        tail: TAIL.into(),
        secs: 1,
        on_candidate: false,
        summary: summary.map(str::to_string),
        summary_source: source,
        tier: None,
    };
    task.checks = vec![
        check(Some("old summary"), Some(DeciderSource::Decider)),
        check(Some("error: a::works"), Some(DeciderSource::Decider)),
    ];
    let prompt = reviewer_prompt(fx.run(), &task, 1, BASE, HEAD, "diff", "");
    assert!(
        prompt.contains("Last check (40 lines):\nerror: a::works\n"),
        "{prompt}"
    );
    assert!(!prompt.contains("compiling"), "{prompt}");
    assert!(!prompt.contains("old summary"), "{prompt}");

    // A fallback's record shows the tail, as in M8a.
    task.checks.push(check(None, Some(DeciderSource::Fallback)));
    let prompt = reviewer_prompt(fx.run(), &task, 1, BASE, HEAD, "diff", "");
    assert!(
        prompt.contains(&format!("Last check (40 lines):\n{TAIL}\n")),
        "{prompt}"
    );
}
