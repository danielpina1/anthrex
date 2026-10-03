//! Milestone 9.5 decision 31 (task M9.5.5a): an orchestrator in PTY mode launched with
//! no prompt after `--`, as the daemon launches one since decision 38. It lists the
//! anthrex server's tools at start (so the real `anthrex mcp` sends its `McpReady`),
//! then runs the `SessionStart` hook the launch configured (Codex, with no hooks, sets
//! its ready title), then takes its first message from the terminal. The MCP server is
//! the real `anthrex mcp` against the stub daemon; no real daemon, no real agent.

mod headless_support;
mod orch_support;

use headless_support::*;
use orch_support::*;
use proto::Runtime;
use serde_json::{Value, json};

#[test]
fn fake_agent_takes_its_first_message_from_stdin() {
    // The script's `expect` reads the first message (`FAKE_AGENT_RESULT` holds it as
    // `{"first_message": <text>}` until the first `mcp_call`).
    let orch = Orch::new(
        &[
            json!({"expect": {"pointer": "/first_message", "equals": "plan this\nand that"}}),
            json!({"exit": 7}),
        ],
        &[(true, "{}")],
    );
    let (log, stdin) = (orch.path("hooks.log"), orch.path("stdin.jsonl"));
    let mut pty = orch.spawn_with(Runtime::Claude, &[("FAKE_AGENT_STDIN_FILE", &stdin)], None);
    wait_for("the SessionStart hook", MCP_RUN, || {
        hooks(&log).iter().any(|(e, _)| e == "SessionStart")
    });
    // The tools were listed first: the server's notice reached the daemon before the
    // hook ran (fake-agent waits for the listing server to exit before going on).
    assert_eq!(
        orch.stub.readies(),
        [(RUN_ID.to_string(), ORCH_WINDOW)],
        "screen {:?}",
        pty.screen()
    );
    assert!(orch.stub.calls().is_empty(), "{:?}", orch.stub.calls());
    pty.write(b"\x1b[200~plan this\nand that\x1b[201~\r");
    assert_eq!(pty.wait(RUN), 7, "screen {:?}", pty.screen());

    // No turn ended before the first message; the message was submitted as a prompt.
    let seen: Vec<(String, Value)> = hooks(&log)
        .into_iter()
        .map(|(e, p)| (e, p["prompt"].clone()))
        .collect();
    assert_eq!(
        seen,
        [
            ("SessionStart".to_string(), Value::Null),
            ("UserPromptSubmit".to_string(), json!("plan this\nand that")),
        ]
    );
    // Recorded as the first message, apart from the reads (`read_message`'s lines).
    let records: Vec<Value> = lines(&stdin)
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(records.len(), 1, "{records:?}");
    assert_eq!(records[0]["first_message"], json!("plan this\nand that"));
    assert!(records[0].get("raw").is_none(), "{records:?}");
}

/// Codex's orchestrator has no hooks here: its ready title is the signal.
#[test]
fn a_codex_fake_sets_its_ready_title_before_its_first_message() {
    let orch = Orch::new(
        &[
            json!({"expect": {"pointer": "/first_message", "equals": "go"}}),
            json!({"exit": 8}),
        ],
        &[(true, "{}")],
    );
    let mut pty = orch.spawn_with(Runtime::Codex, &[], None);
    wait_for("the ready title", MCP_RUN, || {
        pty.screen().contains("\x1b]0;Ready")
    });
    assert_eq!(orch.stub.readies(), [(RUN_ID.to_string(), ORCH_WINDOW)]);
    pty.write(b"\x1b[200~go\x1b[201~\r");
    assert_eq!(pty.wait(RUN), 8, "screen {:?}", pty.screen());
}

/// A session given a prompt after `--` (the launch before decision 38) lists nothing
/// and runs no start hook: its first message is the argument, as before.
#[test]
fn a_prompt_after_the_separator_is_still_the_first_message() {
    let orch = Orch::new(&[json!({"exit": 0})], &[(true, "{}")]);
    let log = orch.path("hooks.log");
    assert_eq!(orch.run(&[]), 0);
    assert_eq!(orch.stub.readies(), []);
    assert!(hooks(&log).is_empty(), "{:?}", hooks(&log));
}
