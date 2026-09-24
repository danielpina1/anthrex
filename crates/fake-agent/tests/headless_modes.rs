//! M8a.20: `fake-agent`'s headless Claude (`-p` stream-json) and Codex (`exec --json`)
//! modes, over real pipes, a real git repository and the real `anthrex mcp` against a
//! stub daemon.

mod headless_support;

use std::fs;
use std::time::Duration;

use headless_support::*;
use proto::{AgentRole, ToolCall};
use serde_json::{Value, json};

const CLAUDE_ID: &str = "00000000-0000-4000-8000-000000000003";

#[test]
fn claude_mode_reads_the_recorded_envelope_and_rejects_others() {
    let dir = tempdir();
    let stdin_file = dir.path().join("stdin.log");
    let recorded = lines(&fixture("claude-2.1.278-input.jsonl"));
    assert_eq!(recorded.len(), 4, "three user messages and an interrupt");
    let mut agent = Agent::spawn(
        &claude_argv(Session::New(CLAUDE_ID), None),
        dir.path(),
        &[("FAKE_AGENT_STDIN_FILE", &stdin_file)],
    );

    for line in &recorded[..3] {
        agent.send(line);
        let result = agent.until(RUN, is_result);
        assert_eq!(result["is_error"], json!(false), "an empty turn: {result}");
    }
    agent.send(&recorded[3]);
    let reply = agent.until(RUN, |v| v["type"] == "control_response");
    assert_eq!(reply["response"]["request_id"], "1", "{reply}");
    assert_eq!(lines(&stdin_file), recorded, "every stdin line is recorded");

    // A content string instead of the recorded text block is refused.
    agent.send(
        r#"{"type":"user","message":{"role":"user","content":"hi"},"parent_tool_use_id":null}"#,
    );
    assert_eq!(agent.wait(RUN).code(), Some(5));
    assert!(
        agent.stderr().contains("fake-agent: bad input line"),
        "{}",
        agent.stderr()
    );

    let mut plain = Agent::spawn(&claude_argv(Session::New(CLAUDE_ID), None), dir.path(), &[]);
    plain.send("do the task");
    assert_eq!(plain.wait(RUN).code(), Some(5));
    assert!(plain.of_type("system").is_empty(), "no turn for a bad line");
}

#[test]
fn a_codex_session_continues_its_script_across_processes() {
    let dir = tempdir();
    let repo = repo(dir.path());
    role_script(
        &repo,
        "worker-t1-1",
        &[
            json!({"print": "one"}),
            json!({"read_message": {"expect": "second"}}),
            json!({"print": "two"}),
            json!({"read_message": {"expect": "third"}}),
            json!({"print": "three"}),
        ],
    );
    let mcp = Mcp::unused("worker", "t1");

    let mut first = Agent::codex(&codex_argv(None, Some(&mcp), false, "first"), &repo, &[]);
    assert!(first.wait(RUN).success());
    assert_eq!(texts(&first), ["one"]);
    assert_eq!(first.of_type("turn.completed").len(), 1);
    let id = thread_id(&first);

    let mut second = Agent::codex(
        &codex_argv(Some(&id), Some(&mcp), false, "the second"),
        &repo,
        &[],
    );
    assert!(second.wait(RUN).success());
    assert_eq!(texts(&second), ["two"]);

    let mut wrong = Agent::codex(
        &codex_argv(Some(&id), Some(&mcp), false, "nope"),
        &repo,
        &[],
    );
    assert_eq!(wrong.wait(RUN).code(), Some(3), "expect is not contained");

    let mut third = Agent::codex(
        &codex_argv(Some(&id), Some(&mcp), false, "a third"),
        &repo,
        &[],
    );
    assert!(third.wait(RUN).success());
    assert_eq!(texts(&third), ["three"]);

    // After the script's end, a message gets an empty turn.
    let mut idle = Agent::codex(
        &codex_argv(Some(&id), Some(&mcp), false, "more?"),
        &repo,
        &[],
    );
    assert!(idle.wait(RUN).success());
    assert!(texts(&idle).is_empty());
    assert_eq!(idle.of_type("turn.completed").len(), 1);
}

#[test]
fn claims_scripts_in_order() {
    let dir = tempdir();
    let repo = repo(dir.path());
    for (name, text) in [
        ("worker-t1-10", "s10"),
        ("worker-t1-2", "s2"),
        ("worker-t1-1", "s1"),
        ("worker-t2-1", "other task"),
        ("reviewer-t1-1", "reviewer"),
    ] {
        role_script(&repo, name, &[json!({"print": text})]);
    }
    let fallback = write_steps(
        &dir.path().join("fallback.jsonl"),
        &[json!({"print": "fallback"})],
    );
    let mcp = Mcp::unused("worker", "t1");

    let mut got = Vec::new();
    let mut ids = Vec::new();
    for _ in 0..4 {
        let mut agent = Agent::codex(
            &codex_argv(None, Some(&mcp), false, "go"),
            &repo,
            &[("FAKE_AGENT_SCRIPT", &fallback)],
        );
        assert!(agent.wait(RUN).success());
        got.extend(texts(&agent));
        ids.push(thread_id(&agent));
    }

    assert_eq!(got, ["s1", "s2", "s10", "fallback"]);
    let scripts = repo.join(".git/fake-agent");
    for (name, id) in ["worker-t1-1", "worker-t1-2", "worker-t1-10"]
        .iter()
        .zip(&ids)
    {
        let claim = fs::read_to_string(scripts.join(format!("{name}.jsonl.claimed"))).unwrap();
        assert_eq!(claim.trim(), id, "{name}'s claim holds its session id");
    }
    assert!(!scripts.join("worker-t2-1.jsonl.claimed").exists());
    assert!(!scripts.join("reviewer-t1-1.jsonl.claimed").exists());
}

#[test]
fn mcp_call_talks_to_a_real_mcp_server() {
    let dir = tempdir();
    let repo = repo(dir.path());
    let stub = StubDaemon::start(true, "Task t1 recorded as done.");
    role_script(
        &repo,
        "worker-t1-1",
        &[
            json!({"mcp_call": {"tool": "task_done", "args": {"summary": "did it"}}}),
            json!({"sh": {"cmd": "printf %s \"$FAKE_AGENT_RESULT\" > result.txt"}}),
        ],
    );
    let mcp = stub.mcp("worker", "t1");
    let mut agent = Agent::spawn(
        &claude_argv(Session::New(CLAUDE_ID), Some(&mcp)),
        &repo,
        &[],
    );

    agent.send(&user_message("go", CLAUDE_ID));
    agent.until(MCP_RUN, is_result);
    agent.close_stdin();
    assert!(agent.wait(RUN).success());

    assert_eq!(
        stub.calls(),
        [ToolCall {
            run_id: "r1".into(),
            task_id: Some("t1".into()),
            role: AgentRole::Worker,
            window_id: 7,
            tool: "task_done".into(),
            args: json!({"summary": "did it"}),
        }]
    );
    let results: Vec<Value> = agent
        .of_type("user")
        .iter()
        .map(|v| v["message"]["content"][0].clone())
        .collect();
    assert_eq!(
        results[0]["content"], "Task t1 recorded as done.",
        "{results:?}"
    );
    assert_eq!(results[0]["is_error"], json!(false));
    assert_eq!(
        fs::read_to_string(repo.join("result.txt")).unwrap(),
        "Task t1 recorded as done."
    );
}

#[test]
fn mcp_call_expect_error() {
    let dir = tempdir();
    let refused = StubDaemon::start(false, "not yours");
    let accepted = StubDaemon::start(true, "fine");
    let run = |stub: &StubDaemon, call: Value| {
        let script = write_steps(
            &dir.path().join("s.jsonl"),
            &[json!({"mcp_call": call}), json!({"print": "survived"})],
        );
        let mut agent = Agent::codex(
            &codex_argv(None, Some(&stub.mcp("worker", "t1")), false, "go"),
            dir.path(),
            &[("FAKE_AGENT_SCRIPT", &script)],
        );
        let status = agent.wait(MCP_RUN);
        (status.code(), texts(&agent))
    };

    let expected = json!({"tool": "task_done", "args": {"summary": "x"}, "expect_error": true});
    assert_eq!(
        run(&refused, expected.clone()),
        (Some(0), vec!["survived".into()])
    );
    let plain = json!({"tool": "task_done", "args": {"summary": "x"}});
    assert_eq!(run(&refused, plain).0, Some(3), "an unexpected tool error");
    assert_eq!(
        run(&accepted, expected).0,
        Some(3),
        "an expected error that never came"
    );
}

#[test]
fn sh_step_sees_the_message() {
    let dir = tempdir();
    let script = write_steps(
        &dir.path().join("s.jsonl"),
        &[json!({"sh": {"cmd": "printf %s \"$FAKE_AGENT_MESSAGE\" > seen.txt"}})],
    );
    let mut agent = Agent::codex(
        &codex_argv(None, None, false, "hello there"),
        dir.path(),
        &[("FAKE_AGENT_SCRIPT", &script)],
    );

    assert!(agent.wait(RUN).success());
    assert_eq!(
        fs::read_to_string(dir.path().join("seen.txt")).unwrap(),
        "hello there"
    );
    let started = agent.of_type("item.started");
    assert_eq!(
        started[0]["item"]["type"], "command_execution",
        "{started:?}"
    );
}

#[test]
fn capture_fills_a_template() {
    let dir = tempdir();
    let repo = repo(dir.path());
    let head = git(&repo, &["rev-parse", "HEAD"]);
    let stub = StubDaemon::start(true, "ok");
    role_script(
        &repo,
        "worker-t1-1",
        &[
            json!({"capture": {"name": "red", "sh": "git --no-optional-locks rev-parse HEAD"}}),
            json!({"mcp_call": {"tool": "task_done", "args": {"summary": "red is {{red}}", "red": "{{red}}"}}}),
        ],
    );

    let mut agent = Agent::codex(
        &codex_argv(None, Some(&stub.mcp("worker", "t1")), false, "go"),
        &repo,
        &[],
    );

    assert!(agent.wait(MCP_RUN).success());
    let calls = stub.calls();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(
        calls[0].args,
        json!({"summary": format!("red is {head}"), "red": head})
    );
}

#[test]
fn interrupt_ends_a_hang() {
    let dir = tempdir();
    let script = write_steps(
        &dir.path().join("s.jsonl"),
        &[
            json!({"print": "working"}),
            json!({"hang": {}}),
            json!({"print": "after"}),
        ],
    );
    let mut agent = Agent::spawn(
        &claude_argv(Session::New(CLAUDE_ID), None),
        dir.path(),
        &[("FAKE_AGENT_SCRIPT", &script)],
    );

    agent.send(&user_message("go", CLAUDE_ID));
    agent.until(RUN, |v| v["type"] == "assistant");
    assert!(
        agent.next_within(Duration::from_millis(300)).is_none(),
        "a hang is silent"
    );
    agent.send(&interrupt("7"));
    let reply = agent.until(RUN, |v| v["type"] == "control_response");
    assert_eq!(reply["response"]["subtype"], "success");
    assert_eq!(reply["response"]["request_id"], "7");
    let stopped = agent.until(RUN, is_result);
    assert_eq!(stopped["subtype"], "error_during_execution", "{stopped}");
    assert_eq!(stopped["is_error"], json!(true));
    assert!(
        matches!(
            stopped["terminal_reason"].as_str(),
            Some("aborted_tools" | "aborted_streaming")
        ),
        "{stopped}"
    );
    assert!(
        agent.is_running(),
        "an interrupt ends the turn, not the process"
    );

    agent.send(&user_message("next", CLAUDE_ID));
    let done = agent.until(RUN, is_result);
    assert_eq!(done["subtype"], "success", "{done}");
    assert_eq!(texts(&agent), ["working", "after"]);
    agent.close_stdin();
    assert!(agent.wait(RUN).success());
}

#[test]
fn api_retry_and_fail_turn_shapes() {
    let dir = tempdir();
    let script = write_steps(
        &dir.path().join("s.jsonl"),
        &[
            json!({"api_retry": {"error": "rate_limit", "delay_ms": 20, "times": 3}}),
            json!({"fail_turn": {"error": "rate_limit"}}),
        ],
    );
    let env = [("FAKE_AGENT_SCRIPT", script.as_path())];

    let mut claude = Agent::spawn(
        &claude_argv(Session::New(CLAUDE_ID), None),
        dir.path(),
        &env,
    );
    claude.send(&user_message("go", CLAUDE_ID));
    let result = claude.until(RUN, is_result);
    let retries: Vec<Value> = claude
        .of_type("system")
        .into_iter()
        .filter(|v| v["subtype"] == "api_retry")
        .collect();
    assert_eq!(retries.len(), 3, "{retries:?}");
    for (n, retry) in retries.iter().enumerate() {
        assert_eq!(retry["attempt"], json!(n + 1));
        assert_eq!(retry["retry_delay_ms"], json!(20));
        assert_eq!(retry["error"], "rate_limit");
        assert_eq!(retry["error_status"], json!(429));
    }
    let failure = claude.of_type("assistant").pop().unwrap();
    assert_eq!(failure["error"], "rate_limit", "{failure}");
    assert_eq!(failure["is_api_error_message"], json!(true));
    assert_eq!(failure["message"]["model"], "<synthetic>");
    assert_eq!(result["subtype"], "success");
    assert_eq!(result["is_error"], json!(true));
    assert_eq!(result["terminal_reason"], "api_error");
    assert_eq!(result["api_error_status"], json!(429));
    claude.close_stdin();
    assert!(claude.wait(RUN).success());
    assert_conforms("claude", &claude.seen);

    let recorded = lines(&fixture("codex-0.156.1-usage-limit.jsonl"));
    let recorded: Value = serde_json::from_str(&recorded[3]).unwrap();
    let mut codex = Agent::codex(&codex_argv(None, None, false, "go"), dir.path(), &env);
    assert_eq!(codex.wait(RUN).code(), Some(1));
    assert!(
        codex.of_type("system").is_empty(),
        "api_retry is Claude only"
    );
    let failed = codex.of_type("turn.failed").pop().unwrap();
    assert_eq!(failed, recorded, "the recorded usage-limit message");
    assert_eq!(
        codex.of_type("error")[0]["message"],
        recorded["error"]["message"]
    );
    assert_conforms("codex", &codex.seen);
}

#[test]
fn resume_argv_is_recorded() {
    let dir = tempdir();
    let repo = repo(dir.path());
    let io = dir.path().join("agent-io");
    fs::create_dir(&io).unwrap();
    let env = [
        ("FAKE_AGENT_ARGS_FILE", io.as_path()),
        ("FAKE_AGENT_STDIN_FILE", io.as_path()),
    ];
    role_script(
        &repo,
        "worker-t1-1",
        &[
            json!({"print": "a"}),
            json!({"read_message": {}}),
            json!({"print": "x"}),
        ],
    );
    role_script(
        &repo,
        "worker-t2-1",
        &[
            json!({"print": "first"}),
            json!({"read_message": {"expect": "b"}}),
            json!({"print": "resumed"}),
        ],
    );

    let codex_mcp = Mcp::unused("worker", "t1");
    let first_argv = codex_argv(None, Some(&codex_mcp), true, "first");
    let mut first = Agent::codex(&first_argv, &repo, &env);
    assert!(first.wait(RUN).success());
    let id = thread_id(&first);
    let resume_argv = codex_argv(Some(&id), Some(&codex_mcp), true, "second");
    let mut second = Agent::codex(&resume_argv, &repo, &env);
    assert!(second.wait(RUN).success());
    assert_eq!(texts(&second), ["x"]);
    let recorded: Vec<Vec<String>> = lines(&io.join("worker-t1-1.args"))
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(recorded, [first_argv, resume_argv.clone()]);
    assert_eq!(resume_argv[..3], ["exec", "resume", id.as_str()]);
    assert!(resume_argv.contains(&"--anthrex-test-exclude-project-config".to_string()));

    let claude_mcp = Mcp::unused("worker", "t2");
    let new_argv = claude_argv(Session::New(CLAUDE_ID), Some(&claude_mcp));
    let mut claude = Agent::spawn(&new_argv, &repo, &env);
    claude.send(&user_message("a", CLAUDE_ID));
    claude.until(RUN, is_result);
    claude.close_stdin();
    assert!(claude.wait(RUN).success());
    let resumed_argv = claude_argv(Session::Resume(CLAUDE_ID), Some(&claude_mcp));
    let mut resumed = Agent::spawn(&resumed_argv, &repo, &env);
    resumed.send(&user_message("b", CLAUDE_ID));
    resumed.until(RUN, is_result);
    resumed.close_stdin();
    assert!(resumed.wait(RUN).success());
    assert_eq!(
        texts(&resumed),
        ["resumed"],
        "--resume continues the claimed script"
    );
    let recorded: Vec<Vec<String>> = lines(&io.join("worker-t2-1.args"))
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(recorded, [new_argv, resumed_argv]);
    assert_eq!(
        lines(&io.join("worker-t2-1.stdin")),
        [user_message("a", CLAUDE_ID), user_message("b", CLAUDE_ID)]
    );
}

/// Carry T17-C1: the real `codex exec` 0.156.1 reads piped stdin to EOF before it
/// starts, so a driver that leaves stdin open must hang here too.
#[test]
fn codex_mode_reads_stdin_to_eof_before_it_starts() {
    let dir = tempdir();
    let stdin_file = dir.path().join("stdin.log");
    let args_file = dir.path().join("args.json");
    let script = write_steps(&dir.path().join("s.jsonl"), &[json!({"print": "hi"})]);
    let mut agent = Agent::spawn(
        &codex_argv(None, None, false, "go"),
        dir.path(),
        &[
            ("FAKE_AGENT_SCRIPT", &script),
            ("FAKE_AGENT_STDIN_FILE", &stdin_file),
            ("FAKE_AGENT_ARGS_FILE", &args_file),
        ],
    );

    agent.send("left over");
    // The argv is recorded before stdin is read: from here on only the read stands
    // between the process and its first line.
    let deadline = std::time::Instant::now() + RUN;
    while !args_file.exists() {
        assert!(
            std::time::Instant::now() < deadline,
            "fake-agent never started"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        agent.next_within(Duration::from_millis(500)).is_none(),
        "nothing before stdin's EOF: {:?}",
        agent.seen
    );
    assert!(agent.is_running());
    agent.close_stdin();
    assert!(agent.wait(RUN).success());
    assert_eq!(texts(&agent), ["hi"]);
    assert_eq!(lines(&stdin_file), ["left over"]);
}
