//! M8a.20, decision 51: every event type `fake-agent`'s headless modes emit has the
//! keys of a recorded event of that type, over real pipes, a real git repository and
//! the real `anthrex mcp` against a stub daemon.

mod headless_support;

use headless_support::*;
use serde_json::{Value, json};

const CLAUDE_ID: &str = "00000000-0000-4000-8000-000000000003";

#[test]
fn claude_mode_output_conforms_to_the_fixture() {
    let dir = tempdir();
    let repo = repo(dir.path());
    let stub = StubDaemon::start(true, "recorded");
    role_script(
        &repo,
        "worker-t1-1",
        &[
            json!({"print": "hello"}),
            json!({"sh": {"cmd": "echo out"}}),
            json!({"mcp_call": {"tool": "task_done", "args": {"summary": "s"}}}),
            json!({"deny": {"tool": "Write", "reason": "outside the worktree"}}),
            json!({"usage": {"input": 7, "output": 3, "cache_read": 100, "cache_write": 5}}),
            json!({"api_retry": {"error": "overloaded", "delay_ms": 1, "times": 1}}),
            json!({"read_message": {"expect": "two"}}),
            json!({"fail_turn": {"error": "rate_limit"}}),
            json!({"read_message": {"expect": "three"}}),
            json!({"hang": {}}),
        ],
    );
    let mcp = stub.mcp("worker", "t1");
    let mut agent = Agent::spawn(
        &claude_argv(Session::New(CLAUDE_ID), Some(&mcp)),
        &repo,
        &[],
    );

    agent.send(&user_message("one", CLAUDE_ID));
    let first = agent.until(MCP_RUN, is_result);
    assert_eq!(first["is_error"], json!(false), "{first}");
    assert_eq!(first["usage"]["input_tokens"], json!(7), "{first}");
    assert_eq!(
        first["permission_denials"][0]["tool_name"], "Write",
        "{first}"
    );
    agent.send(&user_message("two", CLAUDE_ID));
    let failed = agent.until(RUN, is_result);
    assert_eq!(failed["is_error"], json!(true), "{failed}");
    agent.send(&user_message("three", CLAUDE_ID));
    agent.until(RUN, |v| v["type"] == "system" && v["subtype"] == "init");
    agent.send(&interrupt("1"));
    let stopped = agent.until(RUN, is_result);
    assert_eq!(stopped["subtype"], "error_during_execution", "{stopped}");
    agent.close_stdin();
    assert!(agent.wait(RUN).success());

    let types = assert_conforms("claude", &agent.seen);
    for kind in [
        "system/init",
        "assistant",
        "user",
        "system/permission_denied",
        "system/api_retry",
        "result/success",
        "result/error_during_execution",
        "control_response",
    ] {
        assert!(types.contains(kind), "{kind} not emitted: {types:?}");
    }
    let inits = agent.of_type("system");
    let init = inits.iter().find(|v| v["subtype"] == "init").unwrap();
    assert_eq!(init["session_id"], CLAUDE_ID);
    assert_eq!(
        init["mcp_servers"],
        json!([{"name": "anthrex", "status": "connected", "source": "dynamic"}])
    );
    assert_eq!(
        inits.iter().filter(|v| v["subtype"] == "init").count(),
        3,
        "system/init starts every turn"
    );
    let tool_names: Vec<Value> = agent
        .of_type("assistant")
        .iter()
        .map(|v| v["message"]["content"][0].clone())
        .filter(|b| b["type"] == "tool_use")
        .map(|b| b["name"].clone())
        .collect();
    assert_eq!(
        tool_names,
        [
            json!("Bash"),
            json!("mcp__anthrex__task_done"),
            json!("Write")
        ]
    );
}

#[test]
fn codex_mode_output_conforms_to_the_fixture() {
    let dir = tempdir();
    let repo = repo(dir.path());
    let ok = StubDaemon::start(true, "recorded");
    let refused = StubDaemon::start(false, "not a worker");
    role_script(
        &repo,
        "worker-t1-1",
        &[
            json!({"print": "hello"}),
            json!({"sh": {"cmd": "echo out"}}),
            json!({"sh": {"cmd": "exit 1"}}),
            json!({"mcp_call": {"tool": "task_done", "args": {"summary": "s"}}}),
            json!({"read_message": {"expect": "two"}}),
            json!({"mcp_call": {"tool": "task_done", "args": {"summary": "s"}, "expect_error": true}}),
            json!({"read_message": {"expect": "three"}}),
            json!({"fail_turn": {"error": "rate_limit"}}),
        ],
    );
    let mut seen = Vec::new();

    let mut first = Agent::codex(
        &codex_argv(None, Some(&ok.mcp("worker", "t1")), false, "one"),
        &repo,
        &[],
    );
    assert!(first.wait(MCP_RUN).success());
    let id = thread_id(&first);
    seen.extend(first.seen.clone());
    let mut second = Agent::codex(
        &codex_argv(Some(&id), Some(&refused.mcp("worker", "t1")), false, "two"),
        &repo,
        &[],
    );
    assert!(second.wait(MCP_RUN).success());
    seen.extend(second.seen.clone());
    let mut third = Agent::codex(&codex_argv(Some(&id), None, false, "three"), &repo, &[]);
    assert_eq!(third.wait(RUN).code(), Some(1));
    seen.extend(third.seen.clone());

    let types = assert_conforms("codex", &seen);
    for kind in [
        "thread.started",
        "turn.started",
        "item.completed/agent_message",
        "item.started/command_execution",
        "item.completed/command_execution",
        "item.started/mcp_tool_call",
        "item.completed/mcp_tool_call",
        "turn.completed",
        "error",
        "turn.failed",
    ] {
        assert!(types.contains(kind), "{kind} not emitted: {types:?}");
    }
    // Codex reports no item for a command that exits non-zero (M8a.1 item 7).
    let commands: Vec<&Value> = first
        .seen
        .iter()
        .filter(|v| v["type"] == "item.completed" && v["item"]["type"] == "command_execution")
        .collect();
    assert_eq!(commands.len(), 1, "{commands:?}");
    assert_eq!(commands[0]["item"]["aggregated_output"], "out\n");
    for process in [&second, &third] {
        assert_eq!(thread_id(process), id, "a resume repeats the thread id");
    }
    let failed_call = second
        .seen
        .iter()
        .find(|v| v["type"] == "item.completed" && v["item"]["type"] == "mcp_tool_call")
        .unwrap();
    assert_eq!(failed_call["item"]["status"], "failed", "{failed_call}");
    assert_eq!(failed_call["item"]["error"]["message"], "not a worker");
}
