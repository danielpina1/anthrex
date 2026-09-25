//! M8a.12: the MCP tools' engine-side acceptance, `task_blocked`, and decision 54's
//! unavailable sandbox.

use proto::{AgentRole, BlockReason, RunState, Size, TaskState};
use serde_json::json;

use super::done::{block_of, done_args, killed, one_reply, working};
use super::fixture::*;
use crate::headless::FailureKind;
use crate::run::contract::{blocked_recorded, sandbox_unavailable_text};
use crate::run::engine::{AgentSignal, Effect, EventKind, TurnOutcome};

#[test]
fn an_unavailable_sandbox_blocks_as_environment() {
    let error = "sandbox required but unavailable: bwrap: command not found";
    let failed = TurnOutcome::Failed {
        error: error.into(),
        kind: FailureKind::SandboxUnavailable,
    };
    // A failed turn, then the process exits.
    let (mut fx, window) = working();
    fx.signal(
        window,
        AgentSignal::Init {
            session_id: "s".into(),
        },
    );
    fx.turn_ended(window, failed.clone());
    let text = sandbox_unavailable_text(error);
    assert!(text.contains("set [orchestrator] worker_sandbox = false"));
    assert_eq!(block_of(&fx), (BlockReason::Environment, text.clone()));
    let effects = fx.signal(
        window,
        AgentSignal::ProcessExited {
            code: Some(1),
            killed_by_engine: false,
            pid: 9,
        },
    );
    assert!(ops_in(&effects, "ResumeSession").is_empty());
    assert!(fx.ops("ResumeSession").is_empty(), "no resume is attempted");

    // Before `Init`: the driver classifies the stderr text as the same failure.
    let (mut fx, window) = working();
    fx.turn_ended(window, failed);
    fx.signal(
        window,
        AgentSignal::ProcessExited {
            code: Some(1),
            killed_by_engine: false,
            pid: 9,
        },
    );
    assert_eq!(block_of(&fx), (BlockReason::Environment, text));
    assert!(fx.ops("ResumeSession").is_empty());
    assert!(fx.ops("CountCommits").is_empty());
}

fn refused(fx: &mut Fixture, effects: Vec<Effect>, text: &str) {
    assert_eq!(one_reply(&effects), Err(text.to_string()));
    assert!(ops_in(&effects, "VerifyDone").is_empty(), "{text}");
    assert_eq!(fx.task("t1").state, TaskState::Working, "{text}");
}

#[test]
fn tool_authorization() {
    let (mut fx, window) = working();

    let reply = fx.reply();
    let effects = fx.next(EventKind::Tool {
        reply,
        call: proto::ToolCall {
            run_id: "nope".into(),
            task_id: Some("t1".into()),
            role: AgentRole::Worker,
            window_id: window,
            tool: "task_done".into(),
            args: done_args(),
        },
    });
    refused(&mut fx, effects, "unknown run nope");

    let effects = fx.tool(window + 99, "task_done", done_args());
    refused(
        &mut fx,
        effects,
        "this window is not the current worker of task t1",
    );
    let effects = fx.tool_as(AgentRole::Reviewer, window, "t1", "task_done", done_args());
    refused(
        &mut fx,
        effects,
        "this window is not the current worker of task t1",
    );
    let effects = fx.tool_as(AgentRole::Worker, window, "t9", "task_done", done_args());
    assert_eq!(
        one_reply(&effects),
        Err("this window is not the current worker of task t9".into())
    );

    let bad: [(serde_json::Value, &str); 7] = [
        (json!({}), "invalid arguments: summary: required"),
        (
            json!("done"),
            "invalid arguments: arguments: must be an object",
        ),
        (
            json!({"summary": ""}),
            "invalid arguments: summary: must be 1 to 4000 characters",
        ),
        (
            json!({"summary": 7}),
            "invalid arguments: summary: must be a string",
        ),
        (
            json!({"summary": "s", "test": ""}),
            "invalid arguments: test: must be 1 to 300 characters",
        ),
        (
            json!({"summary": "s", "red": "XYZ"}),
            "invalid arguments: red: must be 7 to 40 lowercase hex digits",
        ),
        (
            json!({"summary": "s", "extra": 1}),
            "invalid arguments: extra: unknown field",
        ),
    ];
    for (args, text) in bad {
        let effects = fx.tool(window, "task_done", args);
        refused(&mut fx, effects, text);
    }
    let effects = fx.tool(
        window,
        "task_blocked",
        json!({"kind": "nope", "reason": "r"}),
    );
    refused(
        &mut fx,
        effects,
        "invalid arguments: kind: must be one of question, mis_sized, environment",
    );
    let effects = fx.tool(window, "task_blocked", json!({"kind": "question"}));
    refused(&mut fx, effects, "invalid arguments: reason: required");

    fx.task_mut("t1").state = TaskState::Proof;
    let effects = fx.tool(window, "task_done", done_args());
    assert_eq!(
        one_reply(&effects),
        Err("task_done is accepted only while the task is working (it is proof)".into())
    );
    fx.task_mut("t1").state = TaskState::Working;

    fx.run_mut().state = RunState::Paused;
    let effects = fx.tool(window, "task_done", done_args());
    let paused = format!("run {RUN_ID} is paused; the user must resume it");
    assert_eq!(one_reply(&effects), Err(paused));

    fx.run_mut().state = RunState::Discarded;
    let effects = fx.tool(window, "task_done", done_args());
    assert_eq!(
        one_reply(&effects),
        Err(format!("run {RUN_ID} is discarded"))
    );
    assert!(fx.ops("VerifyDone").is_empty(), "nothing reached the check");
}

#[test]
fn task_blocked_kinds() {
    for (args, reason, kind) in [
        (
            json!({"kind": "question", "reason": "which table?"}),
            BlockReason::Question,
            "question",
        ),
        (
            json!({"reason": "which table?"}),
            BlockReason::Question,
            "question",
        ),
        (
            json!({"kind": "environment", "reason": "cargo is missing"}),
            BlockReason::Environment,
            "environment",
        ),
    ] {
        let (mut fx, window) = working();
        let text = args["reason"].as_str().unwrap().to_string();
        let effects = fx.tool(window, "task_blocked", args);
        assert_eq!(one_reply(&effects), Ok(blocked_recorded(kind)));
        assert_eq!(block_of(&fx), (reason, text));
        assert_eq!(fx.task("t1").failures, 0);
        assert!(!killed(&effects, window), "{kind}: the session waits");
    }

    let (mut fx, window) = working();
    let args = json!({"kind": "mis_sized", "reason": "three modules"});
    let effects = fx.tool(window, "task_blocked", args);
    assert_eq!(one_reply(&effects), Ok(blocked_recorded("mis_sized")));
    let (reason, text) = block_of(&fx);
    assert_eq!(reason, BlockReason::MisSized);
    assert!(text.contains("three modules"), "{text}");
    let t1 = fx.task("t1");
    assert_eq!((t1.size, t1.rung), (Size::M, 3), "rung 3 raises the size");
    assert!(killed(&effects, window));
}
