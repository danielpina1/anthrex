//! M8a.20 fix round 1: turn endings, exits, interrupts and the `sh` child's lifetime in
//! `fake-agent`'s headless modes (rulings T20-I1, T20-I3 and the minors).

mod headless_support;

use std::fs;
use std::os::unix::fs::PermissionsExt;

use headless_support::*;
use serde_json::{Value, json};

const CLAUDE_ID: &str = "00000000-0000-4000-8000-000000000003";

/// An `sh` that leaves a background `sleep` (which ignores SIGINT, as a non-interactive
/// shell's background job does) and records its pid.
fn sleeper(dir: &std::path::Path) -> Value {
    let pid = dir.join("sleep.pid");
    json!({"sh": {"cmd": format!("sleep 37 & echo $! > {}; wait", pid.display())}})
}

#[test]
fn sh_children_die_with_the_agent() {
    // The daemon's kill: SIGTERM to the whole group, and SIGKILL, which no handler
    // sees, so only sharing the group reaches the child. Then Codex's interrupt: SIGINT
    // to the leader alone (`headless/session.rs`).
    let cases = [
        (libc::SIGTERM, true),
        (libc::SIGKILL, true),
        (libc::SIGINT, false),
    ];
    for (signal, whole_group) in cases {
        let dir = tempdir();
        let script = write_steps(&dir.path().join("s.jsonl"), &[sleeper(dir.path())]);
        let mut agent = Agent::codex(
            &codex_argv(None, None, false, "go"),
            dir.path(),
            &[("FAKE_AGENT_SCRIPT", &script)],
        );
        let sleep = wait_pid(&dir.path().join("sleep.pid"));

        if whole_group {
            agent.signal_group(signal);
        } else {
            agent.signal(signal);
        }

        let status = agent.wait(RUN);
        assert!(!status.success(), "signal {signal}: {status:?}");
        assert_gone(
            sleep,
            &format!("signal {signal}, whole group {whole_group}"),
        );
    }
}

#[test]
fn an_interrupt_ends_an_sh_and_kills_it() {
    let dir = tempdir();
    let script = write_steps(
        &dir.path().join("s.jsonl"),
        &[sleeper(dir.path()), json!({"print": "after"})],
    );
    let mut agent = Agent::spawn(
        &claude_argv(Session::New(CLAUDE_ID), None),
        dir.path(),
        &[("FAKE_AGENT_SCRIPT", &script)],
    );

    agent.send(&user_message("go", CLAUDE_ID));
    let sleep = wait_pid(&dir.path().join("sleep.pid"));
    agent.send(&interrupt("4"));
    let stopped = agent.until(RUN, is_result);

    assert_eq!(stopped["subtype"], "error_during_execution", "{stopped}");
    // M8a.1 item 2: an interrupt inside a tool is `aborted_tools`, `stop_reason:
    // "tool_use"`, after the tool's rejected result.
    assert_eq!(stopped["terminal_reason"], "aborted_tools", "{stopped}");
    assert_eq!(stopped["stop_reason"], "tool_use", "{stopped}");
    let rejected = agent
        .of_type("user")
        .into_iter()
        .map(|v| v["message"]["content"][0].clone())
        .find(|b| b["type"] == "tool_result")
        .expect("the interrupted tool's result");
    assert_eq!(rejected["is_error"], json!(true), "{rejected}");
    assert_gone(sleep, "an interrupted sh");
    assert!(
        texts(&agent).is_empty(),
        "the script stops at the interrupt"
    );
    agent.close_stdin();
    assert!(agent.wait(RUN).success());
    assert_conforms("claude", &agent.seen);
}

#[test]
fn a_claude_hang_ends_at_stdin_eof() {
    let dir = tempdir();
    let script = write_steps(&dir.path().join("s.jsonl"), &[json!({"hang": {}})]);
    let mut agent = Agent::spawn(
        &claude_argv(Session::New(CLAUDE_ID), None),
        dir.path(),
        &[("FAKE_AGENT_SCRIPT", &script)],
    );

    agent.send(&user_message("go", CLAUDE_ID));
    agent.until(RUN, |v| v["subtype"] == "init");
    agent.close_stdin();

    assert!(agent.wait(RUN).success(), "the real CLI exits at EOF");
}

#[test]
fn a_read_message_timeout_exits_4() {
    let dir = tempdir();
    let script = write_steps(
        &dir.path().join("s.jsonl"),
        &[
            json!({"print": "a"}),
            json!({"read_message": {"timeout_ms": 300}}),
        ],
    );
    let mut agent = Agent::spawn(
        &claude_argv(Session::New(CLAUDE_ID), None),
        dir.path(),
        &[("FAKE_AGENT_SCRIPT", &script)],
    );

    agent.send(&user_message("go", CLAUDE_ID));

    assert_eq!(agent.wait(RUN).code(), Some(4), "stdin stays open");
    assert!(agent.stderr().contains("read_message timed out"));
}

#[test]
fn a_claude_expect_mismatch_exits_3() {
    let dir = tempdir();
    let script = write_steps(
        &dir.path().join("s.jsonl"),
        &[
            json!({"print": "a"}),
            json!({"read_message": {"expect": "yes"}}),
            json!({"print": "b"}),
        ],
    );
    let mut agent = Agent::spawn(
        &claude_argv(Session::New(CLAUDE_ID), None),
        dir.path(),
        &[("FAKE_AGENT_SCRIPT", &script)],
    );

    agent.send(&user_message("go", CLAUDE_ID));
    agent.until(RUN, is_result);
    agent.send(&user_message("no", CLAUDE_ID));

    assert_eq!(agent.wait(RUN).code(), Some(3));
    assert_eq!(texts(&agent), ["a"]);
}

#[test]
fn end_turn_ends_the_turn() {
    let steps = [
        json!({"print": "a"}),
        json!({"end_turn": {}}),
        json!({"print": "b"}),
    ];
    let dir = tempdir();
    let script = write_steps(&dir.path().join("s.jsonl"), &steps);
    let env = [("FAKE_AGENT_SCRIPT", script.as_path())];

    // Claude: the turn ends, then an unprompted turn runs the rest at once.
    let mut claude = Agent::spawn(
        &claude_argv(Session::New(CLAUDE_ID), None),
        dir.path(),
        &env,
    );
    claude.send(&user_message("go", CLAUDE_ID));
    claude.until(RUN, is_result);
    let second = claude.until(RUN, is_result);
    assert_eq!(second["result"], "b", "{second}");
    let inits = claude.of_type("system").len();
    assert_eq!((inits, claude.of_type("result").len()), (2, 2));
    claude.close_stdin();
    assert!(claude.wait(RUN).success());

    // Codex: the process ends its turn and exits; the rest waits for `exec resume`.
    let repo = repo(dir.path());
    role_script(&repo, "worker-t1-1", &steps);
    let mcp = Mcp::unused("worker", "t1");
    let mut first = Agent::codex(&codex_argv(None, Some(&mcp), false, "go"), &repo, &[]);
    assert!(first.wait(RUN).success());
    assert_eq!(texts(&first), ["a"]);
    assert_eq!(first.of_type("turn.completed").len(), 1);
    let id = thread_id(&first);
    let mut next = Agent::codex(
        &codex_argv(Some(&id), Some(&mcp), false, "more"),
        &repo,
        &[],
    );
    assert!(next.wait(RUN).success());
    assert_eq!(texts(&next), ["b"]);
}

#[test]
fn an_exit_mid_turn_writes_no_turn_end() {
    let dir = tempdir();
    let script = write_steps(
        &dir.path().join("s.jsonl"),
        &[json!({"print": "a"}), json!({"exit": 7})],
    );
    let env = [("FAKE_AGENT_SCRIPT", script.as_path())];

    let mut claude = Agent::spawn(
        &claude_argv(Session::New(CLAUDE_ID), None),
        dir.path(),
        &env,
    );
    claude.send(&user_message("go", CLAUDE_ID));
    assert_eq!(claude.wait(RUN).code(), Some(7));
    assert_eq!(texts(&claude), ["a"]);
    assert!(claude.of_type("result").is_empty(), "{:?}", claude.seen);

    let mut codex = Agent::codex(&codex_argv(None, None, false, "go"), dir.path(), &env);
    assert_eq!(codex.wait(RUN).code(), Some(7));
    assert_eq!(texts(&codex), ["a"]);
    assert!(
        codex.of_type("turn.completed").is_empty(),
        "{:?}",
        codex.seen
    );
    assert!(codex.of_type("turn.failed").is_empty(), "{:?}", codex.seen);
}

/// A stdio MCP server in `sh` that logs every line it reads and answers `initialize`
/// (id 1) and `tools/call` (id 2).
const LOGGING_SERVER: &str = r#"#!/bin/sh
while IFS= read -r line; do
  printf '%s\n' "$line" >> "$0.log"
  case "$line" in
    *'"id":1,'*) printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-06-18","capabilities":{},"serverInfo":{"name":"log","version":"0"}}}' ;;
    *'"id":2,'*) printf '%s\n' '{"jsonrpc":"2.0","id":2,"result":{"content":[{"type":"text","text":"logged"}]}}' ;;
  esac
done
"#;

#[test]
fn mcp_call_sends_initialized_between_initialize_and_the_call() {
    let dir = tempdir();
    let server = dir.path().join("server.sh");
    fs::write(&server, LOGGING_SERVER).unwrap();
    fs::set_permissions(&server, fs::Permissions::from_mode(0o755)).unwrap();
    let script = write_steps(
        &dir.path().join("s.jsonl"),
        &[json!({"mcp_call": {"tool": "task_done", "args": {"summary": "x"}}})],
    );
    let mcp = Mcp {
        exe: server.clone(),
        role: "worker",
        task: Some("t1"),
        socket: dir.path().join("unused.sock"),
    };

    let mut agent = Agent::codex(
        &codex_argv(None, Some(&mcp), false, "go"),
        dir.path(),
        &[("FAKE_AGENT_SCRIPT", &script)],
    );
    assert!(agent.wait(MCP_RUN).success(), "{}", agent.stderr());

    let sent: Vec<Value> = lines(&dir.path().join("server.sh.log"))
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    let methods: Vec<&str> = sent.iter().map(|m| m["method"].as_str().unwrap()).collect();
    assert_eq!(
        methods,
        ["initialize", "notifications/initialized", "tools/call"]
    );
    assert_eq!(sent[0]["params"]["protocolVersion"], "2025-06-18");
    assert!(sent[1].get("id").is_none(), "a notification has no id");
    assert_eq!(
        sent[2]["params"],
        json!({"name": "task_done", "arguments": {"summary": "x"}})
    );
}
