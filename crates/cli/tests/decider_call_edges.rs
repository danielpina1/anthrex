//! M8b.7's review round: a dropped call, the rulings the call carries (R-T1-4, R-T5-1)
//! and the answer's first source (R-T5-2), against real `fake-agent` processes. The
//! fixtures are `support/decider.rs`.

mod support;

use std::time::Duration;

use daemon::decider::call::decide;
use daemon::decider::{
    BlockKind, DeciderAnswer, DeciderRequest, SizeCheckInput, SizeCheckTask, SizeVerdict,
};
use proto::{DeciderMode, DeciderSource, Size};
use serde_json::json;
use support::decider::*;
use support::runtime;

/// Shorter than the context's own timeout, so the caller gives up first.
const CALLER_GIVES_UP: Duration = Duration::from_secs(TIMEOUT_SECS - 1);

/// A caller that stops waiting (a `timeout`, a `select!`, an aborted task or a runtime
/// shutting down) drops the call's future: the deciders it started must still go.
#[test]
fn a_dropped_call_leaves_no_process() {
    let fx = Fixture::new();
    fx.script("blocked_reason", 1, json!({"hang": true}));
    fx.script("blocked_reason", 2, json!({"hang": true}));
    let marker = fx.marker();
    let request = blocked(&format!("hang {marker}"));
    let (claude, codex) = (
        fx.context(DeciderMode::Claude),
        fx.context(DeciderMode::Codex),
    );
    let look = marker.clone();
    let (claude, codex, seen) = runtime().block_on(async {
        let within = CALLER_GIVES_UP - Duration::from_secs(1);
        let seen = tokio::task::spawn_blocking(move || saw_running(&look, 2, within));
        let (claude, codex, seen) = tokio::join!(
            tokio::time::timeout(CALLER_GIVES_UP, decide(&claude, &request)),
            tokio::time::timeout(CALLER_GIVES_UP, decide(&codex, &request)),
            seen
        );
        (claude, codex, seen.unwrap())
    });
    assert!(claude.is_err() && codex.is_err(), "{claude:?} {codex:?}");
    assert!(seen, "the marker never matched both running deciders");
    // Only look: the test signals nothing.
    assert_gone(&marker);
}

/// Ruling R-T1-4: a turn that failed after the answer was given (here, Codex's failure
/// text is the JSON answer) still answers.
#[test]
fn a_failed_turn_that_holds_the_answer_still_answers() {
    let fx = Fixture::new();
    let text = json!({"kind": "environment", "reason": "the linker is missing"}).to_string();
    fx.script("blocked_reason", 1, json!({"fail_turn": text}));
    let decision = runtime().block_on(decide(&fx.context(DeciderMode::Codex), &blocked("x")));
    assert_eq!(decision.source, DeciderSource::Decider, "{decision:?}");
    assert_eq!(
        decision.answer,
        DeciderAnswer::BlockedReason {
            kind: BlockKind::Environment,
            reason: "the linker is missing".into()
        }
    );
}

/// Ruling R-T5-1: a size check keeps only the verdicts of the ids it asked about, the
/// first one per id.
#[test]
fn size_check_answers_are_filtered_to_the_asked_ids() {
    let fx = Fixture::new();
    fx.script(
        "size_check",
        1,
        json!({"answer": {"tasks": [
            {"id": "t1", "size": "M", "reason": "first"},
            {"id": "t9", "size": "L", "reason": "not asked"},
            {"id": "t1", "size": "L", "reason": "second"},
        ]}}),
    );
    let request = DeciderRequest::SizeCheck(SizeCheckInput {
        tasks: vec![SizeCheckTask {
            id: "t1".into(),
            title: "Add a retry".into(),
            brief: "Retry the fetch helper".into(),
            acceptance: vec![],
            owns: vec!["src/fetch.rs".into()],
            deps: vec![],
            size: Size::S,
            interface_change: false,
            hub: false,
        }],
        evidence_refs: vec![],
        evidence: vec![],
        modules: vec![],
        hub: vec![],
    });
    let decision = runtime().block_on(decide(&fx.context(DeciderMode::Claude), &request));
    assert_eq!(decision.source, DeciderSource::Decider, "{decision:?}");
    assert_eq!(
        decision.answer,
        DeciderAnswer::SizeCheck(vec![SizeVerdict {
            id: "t1".into(),
            size: Size::M,
            reason: "first".into()
        }])
    );
}

/// Decision 16's first source: the result's `structured_output` wins over the
/// `StructuredOutput` tool use (the script makes them differ; the real CLI sends the
/// same object in both).
#[test]
fn the_results_structured_output_wins() {
    let fx = Fixture::new();
    fx.script(
        "blocked_reason",
        1,
        json!({
            "answer": {"kind": "question", "reason": "from the tool use"},
            "result_output": {"kind": "environment", "reason": "from the result"},
        }),
    );
    let decision = runtime().block_on(decide(&fx.context(DeciderMode::Claude), &blocked("x")));
    assert_eq!(decision.source, DeciderSource::Decider, "{decision:?}");
    assert_eq!(
        decision.answer,
        DeciderAnswer::BlockedReason {
            kind: BlockKind::Environment,
            reason: "from the result".into()
        }
    );
}
