//! Milestone 9.18 decision 13: `apply` closes a call on `PostToolUseFailure` as
//! `PostToolUse` would close it with a `tool_response` of `{"error": <error>}`, and the
//! transcript's later result replaces it the same way.

use crate::conversation::{Caps, Draft, build, enrich};
use crate::hooks::{HookKind, ParsedHook};
use crate::transcript::Record;
use proto::{Block, HookSource, Role, ToolResult, ToolState, TurnState};
use serde_json::json;
use std::time::{Duration, Instant};

fn hook(kind: HookKind) -> ParsedHook {
    ParsedHook {
        source: HookSource::Claude,
        kind,
        session_id: None,
        agent_id: None,
        agent_type: None,
        tool_name: None,
        tool_input: None,
        notification_type: None,
        transcript_path: None,
        tool_use_id: None,
        tool_response: None,
        tool_result_truncated: None,
        tool_result_stringified: None,
        prompt: None,
        session_source: None,
        error: None,
        is_interrupt: None,
    }
}

fn prompt(text: &str) -> ParsedHook {
    ParsedHook {
        prompt: Some(text.to_owned()),
        ..hook(HookKind::UserPromptSubmit)
    }
}

fn pre(id: &str) -> ParsedHook {
    ParsedHook {
        tool_use_id: Some(id.to_owned()),
        tool_name: Some("Bash".into()),
        tool_input: Some(json!({"command": "cargo test"})),
        ..hook(HookKind::PreToolUse)
    }
}

fn failure(id: &str, error: Option<&str>, is_interrupt: Option<bool>) -> ParsedHook {
    ParsedHook {
        tool_use_id: Some(id.to_owned()),
        tool_name: Some("Bash".into()),
        tool_input: Some(json!({"command": "cargo test"})),
        error: error.map(str::to_owned),
        is_interrupt,
        ..hook(HookKind::PostToolUseFailure)
    }
}

fn run(draft: &mut Draft, hook: &ParsedHook, ts: u64, now: Instant) -> bool {
    build::apply(
        draft,
        proto::Runtime::Claude,
        hook,
        None,
        ts,
        now,
        Caps::default(),
    )
}

/// A prompt and an open `Bash` call `id`, started at `t0`.
fn open_call(id: &str, t0: Instant) -> Draft {
    let mut draft = Draft::new(4, None, proto::Runtime::Claude);
    run(&mut draft, &prompt("run the tests"), 1, t0);
    run(&mut draft, &pre(id), 2, t0);
    draft
}

fn only_call(draft: &Draft) -> &Block {
    let open = draft.open_turn_index().expect("the turn is still open");
    assert_eq!(draft.turns[open].role, Role::Assistant);
    assert_eq!(draft.turns[open].state, TurnState::Running);
    let [block] = draft.turns[open].blocks.as_slice() else {
        panic!("one block expected: {:?}", draft.turns[open].blocks);
    };
    block
}

#[test]
fn a_failure_closes_its_open_call_as_failed() {
    let t0 = Instant::now();
    let mut draft = open_call("toolu_1", t0);
    let error = "exit status 101\nerror: test failed";
    let changed = run(
        &mut draft,
        &failure("toolu_1", Some(error), Some(false)),
        3,
        t0 + Duration::from_millis(1500),
    );
    assert!(changed);
    let Block::ToolCall {
        id,
        state,
        result,
        duration_ms,
        ..
    } = only_call(&draft)
    else {
        panic!("a ToolCall expected");
    };
    assert_eq!(id.as_deref(), Some("toolu_1"));
    assert_eq!(*state, ToolState::Failed);
    assert_eq!(
        result.as_ref(),
        Some(&ToolResult {
            ok: false,
            summary: "exit status 101".into(),
            detail: None,
            truncated: false,
        })
    );
    assert_eq!(*duration_ms, Some(1500));
    assert!(draft.tool_started.is_empty(), "the start time is consumed");
}

#[test]
fn an_interrupt_with_no_error_says_interrupted() {
    let t0 = Instant::now();
    for error in [Some(""), None] {
        let mut draft = open_call("toolu_2", t0);
        assert!(run(
            &mut draft,
            &failure("toolu_2", error, Some(true)),
            3,
            t0
        ));
        let Block::ToolCall { state, result, .. } = only_call(&draft) else {
            panic!("a ToolCall expected");
        };
        assert_eq!(*state, ToolState::Failed, "{error:?}");
        let result = result.as_ref().expect("a result");
        assert!(!result.ok, "{error:?}");
        assert_eq!(result.summary, "interrupted", "{error:?}");
    }
}

#[test]
fn a_failure_for_no_open_call_changes_nothing() {
    let t0 = Instant::now();
    // Another id than the open call's.
    let mut draft = open_call("toolu_3", t0);
    let (turns, started) = (draft.turns.clone(), draft.tool_started.clone());
    let stray = failure("toolu_other", Some("Exit code 1"), Some(false));
    assert!(!run(&mut draft, &stray, 3, t0));
    assert_eq!(draft.turns, turns);
    assert_eq!(draft.tool_started, started);

    // No open turn at all.
    let mut draft = open_call("toolu_3", t0);
    run(&mut draft, &hook(HookKind::Stop), 3, t0);
    let (turns, started) = (draft.turns.clone(), draft.tool_started.clone());
    let late = failure("toolu_3", Some("Exit code 1"), Some(false));
    assert!(!run(&mut draft, &late, 4, t0));
    assert_eq!(draft.turns, turns);
    assert_eq!(draft.tool_started, started);
}

/// The block a failure builds is the block a `PostToolUse` of `{"error": <error>}`
/// builds, and the transcript's later result for the call (as `enrich` applies it)
/// leaves the two equal.
#[test]
fn the_transcript_result_replaces_the_hooks() {
    let t0 = Instant::now();
    let later = t0 + Duration::from_millis(40);
    let error = "exit status 101\nerror: test failed";

    let mut failed = open_call("toolu_5", t0);
    run(
        &mut failed,
        &failure("toolu_5", Some(error), Some(false)),
        3,
        later,
    );
    let mut posted = open_call("toolu_5", t0);
    let post = ParsedHook {
        tool_use_id: Some("toolu_5".into()),
        tool_name: Some("Bash".into()),
        tool_response: Some(json!({ "error": error })),
        ..hook(HookKind::PostToolUse)
    };
    run(&mut posted, &post, 3, later);
    assert_eq!(
        only_call(&failed),
        only_call(&posted),
        "before the transcript"
    );

    let records = [Record::ToolDetail {
        tool_use_id: "toolu_5".into(),
        input: None,
        detail: Some("running 12 tests\ntest parse ... FAILED".into()),
        ok: Some(false),
    }];
    assert!(enrich::apply(&mut failed, &records, Caps::default()));
    assert!(enrich::apply(&mut posted, &records, Caps::default()));
    assert_eq!(
        only_call(&failed),
        only_call(&posted),
        "after the transcript"
    );
    let Block::ToolCall { result, .. } = only_call(&failed) else {
        panic!("a ToolCall expected");
    };
    let result = result.as_ref().expect("a result");
    assert_eq!(
        result.detail.as_deref(),
        Some("running 12 tests\ntest parse ... FAILED")
    );
    assert_eq!(result.summary, "exit status 101");
}

/// Task 11's review, M1: a failure with no `error` (absent or empty) that is not an
/// interrupt says `failed`, not the compact JSON `{"error":""}`.
#[test]
fn a_failure_with_no_error_says_failed() {
    let t0 = Instant::now();
    for (error, is_interrupt) in [
        (Some(""), Some(false)),
        (None, Some(false)),
        (Some(""), None),
        (None, None),
        (Some(" \n"), Some(false)),
    ] {
        let mut draft = open_call("toolu_6", t0);
        assert!(run(
            &mut draft,
            &failure("toolu_6", error, is_interrupt),
            3,
            t0
        ));
        let Block::ToolCall { state, result, .. } = only_call(&draft) else {
            panic!("a ToolCall expected");
        };
        assert_eq!(*state, ToolState::Failed, "{error:?} {is_interrupt:?}");
        let result = result.as_ref().expect("a result");
        assert!(!result.ok, "{error:?} {is_interrupt:?}");
        assert_eq!(result.summary, "failed", "{error:?} {is_interrupt:?}");
    }
}

/// Task 11's review, M3: the ordering cases the failure path inherits from
/// `post_tool_use` (its `Pending` filter and strict id tier), pinned so a failure path
/// that stopped going through it could not regress them silently.
#[test]
fn a_redelivered_failure_or_a_later_post_leaves_the_failed_call() {
    let t0 = Instant::now();
    let mut draft = open_call("toolu_7", t0);
    let first = failure("toolu_7", Some("Exit code 1"), Some(false));
    assert!(run(&mut draft, &first, 3, t0));
    let (turns, started) = (draft.turns.clone(), draft.tool_started.clone());

    // The same failure delivered twice.
    let again = failure("toolu_7", Some("Exit code 2"), Some(false));
    assert!(!run(&mut draft, &again, 4, t0));
    assert_eq!(draft.turns, turns, "a second failure");
    assert_eq!(draft.tool_started, started, "a second failure");

    // A `PostToolUse` for the same id after the failure.
    let post = ParsedHook {
        tool_use_id: Some("toolu_7".into()),
        tool_name: Some("Bash".into()),
        tool_response: Some(json!({"stdout": "ok"})),
        ..hook(HookKind::PostToolUse)
    };
    assert!(!run(&mut draft, &post, 5, t0));
    assert_eq!(draft.turns, turns, "a later PostToolUse");
    let Block::ToolCall { state, result, .. } = only_call(&draft) else {
        panic!("a ToolCall expected");
    };
    assert_eq!(*state, ToolState::Failed);
    assert_eq!(
        result.as_ref().map(|r| r.summary.as_str()),
        Some("Exit code 1")
    );
}

/// Task 11's review, M3: a failure that arrives before its `PreToolUse` is a no-op, and
/// the later `PreToolUse` opens a `Pending` call, as with `PostToolUse` today.
#[test]
fn a_failure_before_its_pre_tool_use_changes_nothing() {
    let t0 = Instant::now();
    let mut draft = Draft::new(4, None, proto::Runtime::Claude);
    run(&mut draft, &prompt("run the tests"), 1, t0);
    let (turns, started) = (draft.turns.clone(), draft.tool_started.clone());
    let early = failure("toolu_8", Some("Exit code 1"), Some(false));
    assert!(!run(&mut draft, &early, 2, t0));
    assert_eq!(draft.turns, turns);
    assert_eq!(draft.tool_started, started);

    assert!(run(&mut draft, &pre("toolu_8"), 3, t0));
    let Block::ToolCall {
        id, state, result, ..
    } = only_call(&draft)
    else {
        panic!("a ToolCall expected");
    };
    assert_eq!(id.as_deref(), Some("toolu_8"));
    assert_eq!(*state, ToolState::Pending);
    assert_eq!(*result, None);
}
