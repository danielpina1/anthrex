//! Milestone 9 task M9.12: `fake-agent` as a run's orchestrator in a real PTY, with the
//! argv the daemon's own launcher builds (`daemon::launch::plan` with a `RoleLaunch`),
//! its `read_message` over bracketed paste and typed input, the script names of
//! sub-planners, integration reviewers and run scouts, the `mcp_until`, `capture_json`,
//! `expect` and `expect_error_contains` steps, and `FAKE_AGENT_MCP_LOG`. The MCP server
//! is the real `anthrex mcp` against the stub daemon; no real daemon, no real agent.

mod headless_support;
mod orch_support;

use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;

use daemon::headless::McpTarget;
use headless_support::*;
use orch_support::*;
use proto::{AgentRole, Runtime};
use serde_json::{Value, json};
#[test]
fn pty_mode_parses_the_mcp_role_from_claude_and_codex_argv() {
    let orch = Orch::new(
        &[
            json!({"mcp_call": {"tool": "get_context", "args": {}}}),
            json!({"exit": 0}),
        ],
        &[(true, r#"{"goal":"g"}"#)],
    );
    role_script(
        &orch.repo,
        "orchestrator-run-2",
        &[
            json!({"mcp_call": {"tool": "run_status", "args": {}}}),
            json!({"exit": 6}),
        ],
    );
    let fallback = write_steps(
        &orch.path("fallback.jsonl"),
        &[
            json!({"mcp_call": {"tool": "task_result", "args": {"task": "t1"}}}),
            json!({"exit": 5}),
        ],
    );
    let args_dir = orch.path("args");
    fs::create_dir(&args_dir).unwrap();
    let env = [("FAKE_AGENT_ARGS_FILE", args_dir.as_path())];

    assert_eq!(orch.spawn(Runtime::Claude, &env).wait(MCP_RUN), 0);
    assert_eq!(orch.spawn(Runtime::Codex, &env).wait(MCP_RUN), 6);
    // With every orchestrator script claimed, M3's `FAKE_AGENT_SCRIPT`.
    let env = [
        ("FAKE_AGENT_ARGS_FILE", args_dir.as_path()),
        ("FAKE_AGENT_SCRIPT", fallback.as_path()),
    ];
    assert_eq!(orch.spawn(Runtime::Claude, &env).wait(MCP_RUN), 5);

    let calls: Vec<(String, AgentRole, String, u32, Value)> = orch
        .stub
        .calls()
        .into_iter()
        .map(|c| (c.tool, c.role, c.run_id, c.window_id, c.args))
        .collect();
    let call = |tool: &str, args: Value| {
        (
            tool.to_string(),
            AgentRole::Orchestrator,
            RUN_ID.to_string(),
            ORCH_WINDOW,
            args,
        )
    };
    assert_eq!(
        calls,
        [
            call("get_context", json!({})),
            call("run_status", json!({})),
            call("task_result", json!({"task": "t1"})),
        ]
    );
    let claude = lines(&args_dir.join("orchestrator-run-1.args"));
    assert!(claude[0].contains("--mcp-config"), "{claude:?}");
    let codex = lines(&args_dir.join("orchestrator-run-2.args"));
    assert!(codex[0].contains("mcp_servers.anthrex.args="), "{codex:?}");
    assert_eq!(lines(&args_dir.join("fallback.args")).len(), 1);
}

#[test]
fn pty_read_message_takes_a_bracketed_paste_and_typed_input() {
    let orch = Orch::new(
        &[
            json!({"read_message": {"expect": "first line"}}),
            json!({"read_message": {"timeout_ms": 60000, "expect": "typed words"}}),
            json!({"exit": 7}),
        ],
        &[(true, "{}")],
    );
    role_script(
        &orch.repo,
        "orchestrator-run-2",
        &[
            json!({"read_message": {"expect": "codex"}}),
            json!({"exit": 8}),
        ],
    );
    let stdin = orch.path("stdin.jsonl");
    let log = orch.path("hooks.log");
    let events = |n: usize, event: &str| {
        let log = log.clone();
        let event = event.to_string();
        move || hooks(&log).iter().filter(|(e, _)| *e == event).count() >= n
    };

    let mut pty = orch.spawn(Runtime::Claude, &[("FAKE_AGENT_STDIN_FILE", &stdin)]);
    wait_for("the first Stop", RUN, events(1, "Stop"));
    pty.write(b"\x1b[200~first line\nsecond line\x1b[201~\r");
    wait_for("the second Stop", RUN, events(2, "Stop"));
    pty.write(b"typed words\r");
    assert_eq!(pty.wait(RUN), 7, "screen {:?}", pty.screen());

    let seen: Vec<(String, Value)> = hooks(&log)
        .into_iter()
        .map(|(e, p)| (e, p["prompt"].clone()))
        .collect();
    assert_eq!(
        seen,
        [
            ("Stop".to_string(), Value::Null),
            (
                "UserPromptSubmit".to_string(),
                json!("first line\nsecond line")
            ),
            ("Stop".to_string(), Value::Null),
            ("UserPromptSubmit".to_string(), json!("typed words")),
        ]
    );
    let reads: Vec<Value> = lines(&stdin)
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(reads.len(), 2, "{reads:?}");
    assert_eq!(
        reads[0]["raw"],
        json!("\u{1b}[200~first line\nsecond line\u{1b}[201~\r")
    );
    assert_eq!(reads[1]["raw"], json!("typed words\r"));
    for read in &reads {
        let at = read["at"].as_str().unwrap();
        assert!(at.len() == 24 && at.ends_with('Z'), "{read}");
    }

    // Codex: the turn's end is its `notify`, and it has no prompt hook.
    fs::remove_file(&log).unwrap();
    let mut pty = orch.spawn(Runtime::Codex, &[]);
    wait_for(
        "the Codex turn's notify",
        RUN,
        events(1, "agent-turn-complete"),
    );
    pty.write(b"\x1b[200~codex\x1b[201~\r");
    assert_eq!(pty.wait(RUN), 8, "screen {:?}", pty.screen());
    let seen: Vec<String> = hooks(&log).into_iter().map(|(e, _)| e).collect();
    assert_eq!(seen, ["agent-turn-complete"]);
}

#[test]
fn script_names_for_planners_integration_reviewers_and_run_scouts() {
    let dir = tempdir();
    let repo = repo(dir.path());
    let target = |role, task: Option<&str>, scout: Option<&str>, epic: Option<&str>| McpTarget {
        role,
        run_id: RUN_ID.into(),
        task_id: task.map(String::from),
        scout_id: scout.map(String::from),
        epic: epic.map(String::from),
        chain: None,
        lane: None,
    };
    let cases = [
        (
            "planner-e1-1",
            target(AgentRole::Planner, None, None, Some("e1")),
        ),
        (
            "reviewer-e1-int1-1",
            target(AgentRole::Reviewer, Some("e1-int1"), None, None),
        ),
        (
            "scout-api-1",
            target(AgentRole::Scout, None, Some("7a2c-api"), None),
        ),
        (
            "scout-r1-1",
            target(AgentRole::Scout, Some("r1"), None, None),
        ),
    ];
    for (code, (name, _)) in cases.iter().enumerate() {
        role_script(&repo, name, &[json!({"exit": 11 + code})]);
    }
    // The unstripped scout id is not the script's name.
    role_script(&repo, "scout-7a2c-api-1", &[json!({"exit": 99})]);

    let (exe, socket) = (Path::new("/nonexistent/anthrex"), Path::new("/tmp/no.sock"));
    for (code, (name, target)) in cases.into_iter().enumerate() {
        let args = codex_argv_for(target, exe, socket, "go");
        let mut agent = Agent::codex(&args, &repo, &[]);
        let status = agent.wait(RUN);
        assert_eq!(
            status.code(),
            Some(11 + code as i32),
            "{name}: {}",
            agent.stderr()
        );
        assert!(
            repo.join(format!(".git/fake-agent/{name}.jsonl.claimed"))
                .is_file()
        );
    }
}

#[test]
fn mcp_until_polls_until_the_pointer_matches() {
    let open = r#"{"gate":{"state":"open"}}"#;
    let orch = Orch::new(
        &[
            json!({"mcp_until": {"tool": "run_status", "args": {"wait_secs": 0},
                "until": {"pointer": "/gate/state", "equals": "approved"},
                "timeout_ms": 60000}}),
            json!({"expect": {"pointer": "/n", "equals": 3}}),
            json!({"exit": 0}),
        ],
        &[
            (true, open),
            (true, open),
            (true, r#"{"gate":{"state":"approved"},"n":3}"#),
        ],
    );

    assert_eq!(orch.run(&[]), 0);
    assert_eq!(orch.stub.calls().len(), 3);
}

#[test]
fn mcp_until_times_out() {
    let orch = Orch::new(
        &[
            json!({"mcp_until": {"tool": "run_status", "args": {},
                "until": {"pointer": "/gate/state", "equals": "approved"},
                "timeout_ms": 300}}),
            json!({"exit": 0}),
        ],
        &[(true, r#"{"gate":{"state":"open"}}"#)],
    );

    assert_eq!(orch.run(&[]), 4);
    assert!(!orch.stub.calls().is_empty());
}

#[test]
fn capture_json_substitutes() {
    let orch = Orch::new(
        &[
            json!({"mcp_call": {"tool": "get_context", "args": {}}}),
            json!({"capture_json": {"name": "sid", "pointer": "/scout_id"}}),
            json!({"capture_json": {"name": "gate", "pointer": "/gate"}}),
            json!({"mcp_call": {"tool": "task_result",
                "args": {"task": "{{sid}}", "note": "{{gate}} n"}}}),
            json!({"exit": 0}),
        ],
        &[(true, r#"{"scout_id":"7a2c-api","gate":{"state":"open"}}"#)],
    );

    assert_eq!(orch.run(&[]), 0);
    let calls = orch.stub.calls();
    assert_eq!(
        calls[1].args,
        json!({"task": "7a2c-api", "note": r#"{"state":"open"} n"#})
    );
}

#[test]
fn expect_fails_with_exit_3() {
    let orch = Orch::new(
        &[
            json!({"mcp_call": {"tool": "run_status", "args": {}}}),
            json!({"expect": {"pointer": "/gate/state", "equals": "open"}}),
            json!({"mcp_call": {"tool": "edit_plan", "args": {}, "expect_error": true}}),
            json!({"expect_error_contains": {"text": "is closed"}}),
            json!({"expect": {"pointer": "/gate/state", "equals": "approved"}}),
            json!({"exit": 0}),
        ],
        &[
            (true, r#"{"gate":{"state":"open"}}"#),
            (false, "the gate is closed"),
        ],
    );
    // `expect` reads the last result, now the error text: not JSON, so it differs.
    assert_eq!(orch.run(&[]), 3);
    assert_eq!(orch.stub.calls().len(), 2);

    let orch = Orch::new(
        &[
            json!({"mcp_call": {"tool": "edit_plan", "args": {}, "expect_error": true}}),
            json!({"expect_error_contains": {"text": "approved"}}),
            json!({"exit": 0}),
        ],
        &[(false, "the gate is closed")],
    );
    assert_eq!(orch.run(&[]), 3);

    let orch = Orch::new(
        &[
            json!({"mcp_call": {"tool": "run_status", "args": {}}}),
            json!({"expect": {"pointer": "/gate/state", "equals": "approved"}}),
            json!({"exit": 0}),
        ],
        &[(true, r#"{"gate":{"state":"open"}}"#)],
    );
    assert_eq!(orch.run(&[]), 3);
}

#[test]
fn mcp_log_records_each_call() {
    let orch = Orch::new(
        &[
            json!({"mcp_call": {"tool": "get_context", "args": {}}}),
            json!({"mcp_call": {"tool": "edit_plan", "args": {"edits": []}, "expect_error": true}}),
            json!({"exit": 0}),
        ],
        &[
            (true, r#"{"goal":"g"}"#),
            (false, "no edits"),
            (true, "noted"),
        ],
    );
    let log = orch.path("mcp.jsonl");
    assert_eq!(orch.run(&[("FAKE_AGENT_MCP_LOG", &log)]), 0);

    // A headless worker logs to the same file.
    role_script(
        &orch.repo,
        "worker-t1-1",
        &[json!({"mcp_call": {"tool": "task_done", "args": {"summary": "s"}}})],
    );
    let mcp = orch.stub.mcp("worker", "t1");
    let args = codex_argv(None, Some(&mcp), false, "go");
    let mut agent = Agent::codex(&args, &orch.repo, &[("FAKE_AGENT_MCP_LOG", &log)]);
    assert_eq!(agent.wait(MCP_RUN).code(), Some(0), "{}", agent.stderr());

    // M9.16: each line says how long its call took; the rest is compared exactly.
    let logged: Vec<Value> = lines(&log)
        .iter()
        .map(|l| {
            let mut line: Value = serde_json::from_str(l).unwrap();
            let ms = line.as_object_mut().unwrap().remove("ms");
            assert!(ms.as_ref().is_some_and(Value::is_u64), "{l}");
            line
        })
        .collect();
    assert_eq!(
        logged,
        [
            json!({"script": "orchestrator-run-1", "tool": "get_context", "args": {},
                "ok": true, "result": r#"{"goal":"g"}"#}),
            json!({"script": "orchestrator-run-1", "tool": "edit_plan",
                "args": {"edits": []}, "ok": false, "result": "no edits"}),
            json!({"script": "worker-t1-1", "tool": "task_done", "args": {"summary": "s"},
                "ok": true, "result": "noted"}),
        ]
    );
}

/// Review fixes 1 and 2: raw mode is only for `read_message`, and restored after it, so
/// M3's `read_line` reads a typed `\r` as a line end before and after one.
#[test]
fn read_line_works_around_read_message_in_pty_mode() {
    let orch = Orch::new(
        &[
            json!({"read_line": true}),
            json!({"read_message": {"expect": "m"}}),
            json!({"read_line": true}),
            json!({"exit": 9}),
        ],
        &[(true, "{}")],
    );
    let log = orch.path("hooks.log");
    let count = |event: &'static str, n: usize| {
        let log = log.clone();
        move || hooks(&log).iter().filter(|(e, _)| e == event).count() >= n
    };
    let mut pty = orch.spawn(Runtime::Claude, &[]);
    pty.write(b"x\r");
    wait_for("the Stop of read_message", RUN, count("Stop", 1));
    pty.write(b"\x1b[200~m\x1b[201~\r");
    wait_for(
        "the message's prompt hook",
        RUN,
        count("UserPromptSubmit", 1),
    );
    pty.write(b"y\r");
    assert_eq!(pty.wait(RUN), 9, "screen {:?}", pty.screen());
}

/// Review fix 3: `\r\n` is one Enter, and the text's line ends are `\n` while `raw`
/// keeps the bytes.
#[test]
fn crlf_is_one_enter_and_the_text_normalises_line_ends() {
    let orch = Orch::new(
        &[
            json!({"read_message": {}}),
            json!({"read_message": {}}),
            json!({"read_message": {}}),
            json!({"exit": 0}),
        ],
        &[(true, "{}")],
    );
    let (log, stdin) = (orch.path("hooks.log"), orch.path("stdin.jsonl"));
    let stops = |n: usize| {
        let log = log.clone();
        move || hooks(&log).iter().filter(|(e, _)| e == "Stop").count() >= n
    };
    let mut pty = orch.spawn(Runtime::Claude, &[("FAKE_AGENT_STDIN_FILE", &stdin)]);
    wait_for("the first Stop", RUN, stops(1));
    pty.write(b"first\r\n");
    wait_for("the second Stop", RUN, stops(2));
    pty.write(b"\x1b[200~a\r\nb\rc\x1b[201~\r");
    wait_for("the third Stop", RUN, stops(3));
    // The `\n` of the paste's Enter, arriving late.
    pty.write(b"\nthird\r");
    assert_eq!(pty.wait(RUN), 0, "screen {:?}", pty.screen());

    let prompts: Vec<Value> = hooks(&log)
        .into_iter()
        .filter(|(e, _)| e == "UserPromptSubmit")
        .map(|(_, p)| p["prompt"].clone())
        .collect();
    assert_eq!(prompts, [json!("first"), json!("a\nb\nc"), json!("third")]);
    let raws: Vec<Value> = lines(&stdin)
        .iter()
        .map(|l| serde_json::from_str::<Value>(l).unwrap()["raw"].clone())
        .collect();
    assert_eq!(
        raws,
        [
            json!("first\r"),
            json!("\u{1b}[200~a\r\nb\rc\u{1b}[201~\r"),
            json!("third\r")
        ]
    );
}

/// Review fix 4: `expect_error_contains` needs the last `mcp_call` to have failed.
#[test]
fn expect_error_contains_needs_an_error_reply() {
    let orch = Orch::new(
        &[
            json!({"mcp_call": {"tool": "run_status", "args": {}}}),
            json!({"expect_error_contains": {"text": "closed"}}),
            json!({"exit": 0}),
        ],
        &[(true, "the gate is closed")],
    );
    assert_eq!(orch.run(&[]), 3);
}

/// Milliseconds since midnight of a `YYYY-MM-DDTHH:MM:SS.mmmZ` time.
fn millis(at: &Value) -> i64 {
    let at = at.as_str().unwrap();
    let field = |range: std::ops::Range<usize>| at[range].parse::<i64>().unwrap();
    ((field(11..13) * 60 + field(14..16)) * 60 + field(17..19)) * 1000 + field(20..23)
}

/// Review fix 4: each read records when its first byte arrived (`first_at`) beside when
/// it ended (`at`), so typing can be told from a paste.
#[test]
fn stdin_file_records_when_a_message_started() {
    let orch = Orch::new(
        &[json!({"read_message": {}}), json!({"exit": 0})],
        &[(true, "{}")],
    );
    let (log, stdin) = (orch.path("hooks.log"), orch.path("stdin.jsonl"));
    let mut pty = orch.spawn(Runtime::Claude, &[("FAKE_AGENT_STDIN_FILE", &stdin)]);
    wait_for("the Stop", RUN, || !hooks(&log).is_empty());
    pty.write(b"ty");
    thread::sleep(Duration::from_millis(400));
    pty.write(b"ped\r");
    assert_eq!(pty.wait(RUN), 0, "screen {:?}", pty.screen());

    let read: Value = serde_json::from_str(&lines(&stdin)[0]).unwrap();
    // A paste arrives in one read (a gap near 0); typing spans the 400 ms sleep. The
    // lower bound leaves 300 ms for fake-agent to read the first bytes late (whole-branch
    // review, item 6; `docs/timing-budgets.md`); the upper one is a hang guard.
    let typing = millis(&read["at"]) - millis(&read["first_at"]);
    assert!((100..5000).contains(&typing), "{read}");
}

/// Every orchestrator in this binary runs the same `anthrex` stand-in. macOS assesses a
/// newly written executable on its first exec (about 0.5 s) and serialises those
/// assessments: twelve tests each writing their own stand-in cost up to about 5 s at
/// their first hook, which is fake-agent's whole `STEP_TIMEOUT`, so it exited with
/// "step timed out after 5 seconds" and the test waited out `RUN`. One shared file is
/// assessed once. Its per-test values come from the environment each spawn sets.
#[test]
fn every_orch_shares_one_wrapper() {
    let first = Orch::new(&[json!({"exit": 0})], &[(true, "{}")]);
    let second = Orch::new(&[json!({"exit": 0})], &[(true, "{}")]);
    assert_eq!(first.exe, second.exe);
    assert!(!first.exe.starts_with(first.dir.path()), "{:?}", first.exe);
}
