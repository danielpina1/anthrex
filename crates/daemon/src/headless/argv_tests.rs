use super::*;
use crate::launch;
use proto::{AgentRole, Effort, RunRef, Runtime};
use serde_json::{Value, json};
use std::path::PathBuf;

pub(super) const EXE: &str = "/opt/anthrex/bin/anthrex";
pub(super) const SOCKET: &str = "/tmp/a.sock";
pub(super) const WINDOW: u32 = 7;
pub(super) const UUID: &str = "5f1c9a2e-7b3d-4e8f-9a1b-2c3d4e5f6a7b";
/// The git common dir, apart from the worktree (`cwd`) and the project root.
pub(super) const COMMON: &str = "/tmp/x/.git";

pub(super) fn worker(runtime: Runtime) -> HeadlessSpec {
    HeadlessSpec {
        runtime,
        model: match runtime {
            Runtime::Codex => String::new(),
            _ => "claude-sonnet-5".into(),
        },
        effort: Effort::High,
        cwd: "/tmp/p/.anthrex/wt/t1".into(),
        instructions: "Say \"done\" when done.\nThen stop.".into(),
        mcp: Some(McpTarget {
            role: AgentRole::Worker,
            run_id: "r-3f9a".into(),
            task_id: Some("t1".into()),
        }),
        allowed_tools: [
            "mcp__anthrex__task_done",
            "mcp__anthrex__task_blocked",
            "Bash",
            "Edit",
        ]
        .map(String::from)
        .to_vec(),
        claude_permission_mode: Some("acceptEdits".into()),
        claude_disallowed_tools: vec![],
        claude_sandbox: Some(ClaudeSandbox {
            writable_roots: vec![PathBuf::from(COMMON)],
        }),
        codex_sandbox: "workspace-write".into(),
        codex_writable_roots: vec![PathBuf::from(COMMON)],
        env: vec![],
        claude_auth: config::ClaudeAuth::Login,
        api_key_helper: None,
        run_ref: Some(RunRef {
            run_id: "r-3f9a".into(),
            task_id: Some("t1".into()),
            role: AgentRole::Worker,
            session: 1,
        }),
    }
}

pub(super) fn reviewer(runtime: Runtime) -> HeadlessSpec {
    HeadlessSpec {
        model: match runtime {
            Runtime::Codex => "gpt-5.5".into(),
            _ => "claude-opus-5".into(),
        },
        effort: Effort::Medium,
        instructions: "Review it.".into(),
        mcp: Some(McpTarget {
            role: AgentRole::Reviewer,
            run_id: "r-3f9a".into(),
            task_id: Some("t2".into()),
        }),
        allowed_tools: [
            "mcp__anthrex__submit_review",
            "Read",
            "Glob",
            "Grep",
            "Bash(git diff:*)",
            "Bash(git log:*)",
            "Bash(git show:*)",
        ]
        .map(String::from)
        .to_vec(),
        claude_permission_mode: Some("dontAsk".into()),
        claude_disallowed_tools: ["Edit", "Write", "NotebookEdit"].map(String::from).to_vec(),
        claude_sandbox: Some(ClaudeSandbox {
            writable_roots: Vec::new(),
        }),
        codex_sandbox: "read-only".into(),
        codex_writable_roots: vec![],
        ..worker(runtime)
    }
}

pub(super) fn claude(spec: &HeadlessSpec, session: &SessionArg, caps: &CliCaps) -> Vec<String> {
    claude_args(
        spec,
        session,
        Path::new(EXE),
        WINDOW,
        Path::new(SOCKET),
        caps,
    )
}

pub(super) fn codex(
    spec: &HeadlessSpec,
    session: &SessionArg,
    message: &str,
    caps: &CliCaps,
) -> Vec<String> {
    codex_args(
        spec,
        session,
        message,
        Path::new(EXE),
        WINDOW,
        Path::new(SOCKET),
        caps,
    )
}

pub(super) fn new_session() -> SessionArg {
    SessionArg::New {
        uuid: Some(UUID.into()),
    }
}

/// The value after `flag`, parsed as JSON.
pub(super) fn json_after(argv: &[String], flag: &str) -> Value {
    let at = argv.iter().position(|a| a == flag).expect(flag);
    serde_json::from_str(&argv[at + 1]).unwrap()
}

/// `argv` with the value after each of `flags` replaced by `<json>`, so the rest can be
/// compared as strings and the JSON as values.
fn masked(argv: &[String], flags: &[&str]) -> Vec<String> {
    let mut out = argv.to_vec();
    for flag in flags {
        let at = out.iter().position(|a| a == flag).expect(flag);
        out[at + 1] = "<json>".into();
    }
    out
}

fn mcp_config(role: &str, task: &str) -> Value {
    json!({"mcpServers": {"anthrex": {
        "type": "stdio",
        "command": EXE,
        "args": ["mcp", "--role", role, "--run", "r-3f9a", "--task", task,
                 "--window", "7", "--socket", SOCKET],
    }}})
}

fn sandbox_block() -> Value {
    json!({
        "enabled": true,
        "allowUnsandboxedCommands": false,
        "failIfUnavailable": true,
        "filesystem": {"allowWrite": [COMMON]},
    })
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| s.to_string()).collect()
}

#[test]
fn mcp_args_for_a_worker() {
    let target = McpTarget {
        role: AgentRole::Worker,
        run_id: "r-3f9a".into(),
        task_id: Some("t1".into()),
    };
    assert_eq!(
        mcp_args(&target, 7, Path::new("/tmp/a.sock")),
        strs(&[
            "mcp",
            "--role",
            "worker",
            "--run",
            "r-3f9a",
            "--task",
            "t1",
            "--window",
            "7",
            "--socket",
            "/tmp/a.sock"
        ])
    );
    let no_task = McpTarget {
        task_id: None,
        role: AgentRole::Reviewer,
        ..target
    };
    assert_eq!(
        mcp_args(&no_task, 12, Path::new("/s")),
        strs(&[
            "mcp", "--role", "reviewer", "--run", "r-3f9a", "--window", "12", "--socket", "/s"
        ])
    );
}

#[test]
fn argv_builders() {
    let worker_argv = claude(&worker(Runtime::Claude), &new_session(), &CLI_CAPS);
    assert_eq!(
        masked(&worker_argv, &["--settings", "--mcp-config"]),
        strs(&[
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-prompts",
            "none",
            "--session-id",
            UUID,
            "--setting-sources",
            "user",
            "--strict-mcp-config",
            "--settings",
            "<json>",
            "--mcp-config",
            "<json>",
            "--allowedTools",
            "mcp__anthrex__task_done,mcp__anthrex__task_blocked,Bash,Edit",
            "--append-system-prompt",
            "Say \"done\" when done.\nThen stop.",
            "--permission-mode",
            "acceptEdits",
            "--model",
            "claude-sonnet-5",
            "--effort",
            "high",
        ])
    );
    let mut settings = launch::claude::settings(Path::new(EXE), WINDOW);
    settings["sandbox"] = sandbox_block();
    assert_eq!(json_after(&worker_argv, "--settings"), settings);
    assert_eq!(
        json_after(&worker_argv, "--mcp-config"),
        mcp_config("worker", "t1")
    );

    // The reviewer: no sandbox, the dontAsk fallback M8a.1 found necessary, and the
    // disallowed tools before the next flag.
    let reviewer_argv = claude(&reviewer(Runtime::Claude), &new_session(), &CLI_CAPS);
    assert_eq!(
        masked(&reviewer_argv, &["--settings", "--mcp-config"]),
        strs(&[
            "-p",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
            "--verbose",
            "--permission-prompts",
            "none",
            "--session-id",
            UUID,
            "--setting-sources",
            "user",
            "--strict-mcp-config",
            "--settings",
            "<json>",
            "--mcp-config",
            "<json>",
            "--allowedTools",
            "mcp__anthrex__submit_review,Read,Glob,Grep,Bash(git diff:*),Bash(git log:*),Bash(git show:*)",
            "--disallowedTools",
            "Edit,Write,NotebookEdit",
            "--append-system-prompt",
            "Review it.",
            "--permission-mode",
            "dontAsk",
            "--model",
            "claude-opus-5",
            "--effort",
            "medium",
        ])
    );
    // F1c round 3 (N4): the reviewer's settings now carry a read-only sandbox block.
    let mut reviewer_expected = launch::claude::settings(Path::new(EXE), WINDOW);
    reviewer_expected["sandbox"] = json!({
        "enabled": true,
        "allowUnsandboxedCommands": false,
        "failIfUnavailable": true,
        "filesystem": {"allowWrite": []},
    });
    assert_eq!(json_after(&reviewer_argv, "--settings"), reviewer_expected);
    assert_eq!(
        json_after(&reviewer_argv, "--mcp-config"),
        mcp_config("reviewer", "t2")
    );

    // A resume re-passes every flag; only the session argument differs.
    let resume = SessionArg::Resume {
        session_id: "sess-42".into(),
    };
    let resumed = claude(&worker(Runtime::Claude), &resume, &CLI_CAPS);
    let mut expected = worker_argv.clone();
    expected[8] = "--resume".into();
    expected[9] = "sess-42".into();
    assert_eq!(resumed, expected);

    // Codex: a worker's first turn, on Codex's default model.
    let codex_worker = codex(
        &worker(Runtime::Codex),
        &SessionArg::New { uuid: None },
        "Do t1",
        &CLI_CAPS,
    );
    let mcp_array = r#"["mcp","--role","worker","--run","r-3f9a","--task","t1","--window","7","--socket","/tmp/a.sock"]"#;
    assert_eq!(
        codex_worker,
        strs(&[
            "exec",
            "--json",
            "-c",
            &format!("mcp_servers.anthrex.command=\"{EXE}\""),
            "-c",
            &format!("mcp_servers.anthrex.args={mcp_array}"),
            "-c",
            "mcp_servers.anthrex.tool_timeout_sec=120",
            "-c",
            "mcp_servers.anthrex.default_tools_approval_mode=\"approve\"",
            "-c",
            "developer_instructions=\"Say \\\"done\\\" when done.\\nThen stop.\"",
            "-c",
            "model_reasoning_effort=\"high\"",
            "-c",
            "approval_policy=\"never\"",
            "-s",
            "workspace-write",
            "-c",
            "sandbox_workspace_write.writable_roots=[\"/tmp/x/.git\"]",
            "--",
            "Do t1",
        ])
    );

    // A resume: `exec resume <id>`, the sandbox through `-c sandbox_mode` (M8a.1 item 6:
    // resume rejects `-s`), and a named model.
    let mut named = worker(Runtime::Codex);
    named.model = "gpt-5.5".into();
    let codex_resume = codex(
        &named,
        &SessionArg::Resume {
            session_id: "th-157".into(),
        },
        "Fix the test.",
        &CLI_CAPS,
    );
    let mut expected = codex_worker.clone();
    expected.splice(0..2, strs(&["exec", "resume", "th-157", "--json"]));
    let s = expected.iter().position(|a| a == "-s").unwrap();
    expected.splice(s..s + 2, strs(&["-c", "sandbox_mode=\"workspace-write\""]));
    let dashes = expected.iter().position(|a| a == "--").unwrap();
    expected.splice(dashes.., strs(&["-m", "gpt-5.5", "--", "Fix the test."]));
    assert_eq!(codex_resume, expected);

    // A reviewer: read-only, no writable roots, its own MCP role and effort.
    let codex_reviewer = codex(
        &reviewer(Runtime::Codex),
        &SessionArg::New { uuid: None },
        "Review t2",
        &CLI_CAPS,
    );
    let reviewer_array = mcp_array.replace("worker", "reviewer").replace("t1", "t2");
    assert_eq!(
        codex_reviewer,
        strs(&[
            "exec",
            "--json",
            "-c",
            &format!("mcp_servers.anthrex.command=\"{EXE}\""),
            "-c",
            &format!("mcp_servers.anthrex.args={reviewer_array}"),
            "-c",
            "mcp_servers.anthrex.tool_timeout_sec=120",
            "-c",
            "mcp_servers.anthrex.default_tools_approval_mode=\"approve\"",
            "-c",
            "developer_instructions=\"Review it.\"",
            "-c",
            "model_reasoning_effort=\"medium\"",
            "-c",
            "approval_policy=\"never\"",
            "-s",
            "read-only",
            "-m",
            "gpt-5.5",
            "--",
            "Review t2",
        ])
    );
}

/// Ruling T7-C1 (codex 0.156.1, `codex-0.156.1-mcp-approval-*.jsonl`): under
/// `approval_policy="never"`, `"auto"` fails every MCP call and `"approve"` completes it.
#[test]
fn the_anthrex_mcp_server_is_approved_under_never() {
    for spec in [worker(Runtime::Codex), reviewer(Runtime::Codex)] {
        for session in [
            SessionArg::New { uuid: None },
            SessionArg::Resume {
                session_id: "th-1".into(),
            },
        ] {
            let argv = codex(&spec, &session, "go", &CLI_CAPS);
            let modes: Vec<&String> = argv
                .iter()
                .filter(|a| a.contains("default_tools_approval_mode"))
                .collect();
            assert_eq!(
                modes,
                ["mcp_servers.anthrex.default_tools_approval_mode=\"approve\""]
            );
            assert!(argv.contains(&"approval_policy=\"never\"".to_string()));
        }
    }
}

#[test]
fn codex_worker_args_add_the_git_common_dir_as_writable() {
    let roots = "sandbox_workspace_write.writable_roots=[\"/tmp/x/.git\"]";
    for session in [
        SessionArg::New { uuid: None },
        SessionArg::Resume {
            session_id: "th-1".into(),
        },
    ] {
        let argv = codex(&worker(Runtime::Codex), &session, "go", &CLI_CAPS);
        let at = argv
            .iter()
            .position(|a| a == roots)
            .expect("the writable root");
        assert_eq!(argv[at - 1], "-c");
        // Codex's default `/tmp` and `$TMPDIR` stay writable: M8a.1 excluded them only to
        // make its recording prove something.
        assert!(!argv.iter().any(|a| a.contains("exclude_slash_tmp")));
        assert!(!argv.iter().any(|a| a.contains("exclude_tmpdir_env_var")));
        let reviewer = codex(&reviewer(Runtime::Codex), &session, "go", &CLI_CAPS);
        assert!(!reviewer.iter().any(|a| a.contains("writable_roots")));
    }
    // Several roots make one TOML array.
    let mut two = worker(Runtime::Codex);
    two.codex_writable_roots.push("/tmp/y dir/.git".into());
    let argv = codex(&two, &SessionArg::New { uuid: None }, "go", &CLI_CAPS);
    assert!(argv.contains(
        &"sandbox_workspace_write.writable_roots=[\"/tmp/x/.git\",\"/tmp/y dir/.git\"]".to_string()
    ));
}

#[test]
fn worker_settings_json_enables_the_sandbox() {
    let sandbox = ClaudeSandbox {
        writable_roots: vec![PathBuf::from(COMMON)],
    };
    let mut expected = launch::claude::settings(Path::new(EXE), WINDOW);
    expected["sandbox"] = sandbox_block();
    assert_eq!(
        claude_settings(Path::new(EXE), WINDOW, Some(&sandbox), &CLI_CAPS),
        expected
    );
    // The key names come from the caps.
    let renamed = CliCaps {
        claude_sandbox_keys: SandboxKeys {
            enabled: "on",
            allow_unsandboxed: "escape",
            write_allow: "fs.write.paths",
            fail_if_unavailable: "strict",
        },
        ..CLI_CAPS
    };
    assert_eq!(
        claude_settings(Path::new(EXE), WINDOW, Some(&sandbox), &renamed)["sandbox"],
        json!({"on": true, "escape": false, "strict": true,
               "fs": {"write": {"paths": [COMMON]}}})
    );
    // A reviewer's has none, and neither has a worker's with `worker_sandbox = false`.
    let plain = launch::claude::settings(Path::new(EXE), WINDOW);
    assert_eq!(
        claude_settings(Path::new(EXE), WINDOW, None, &CLI_CAPS),
        plain
    );
    // F1c round 3 (N4): a Claude reviewer runs read-only, so its settings carry a
    // sandbox block whose `allowWrite` is empty.
    let reviewer_settings = json_after(
        &claude(&reviewer(Runtime::Claude), &new_session(), &CLI_CAPS),
        "--settings",
    );
    assert_eq!(
        reviewer_settings["sandbox"],
        json!({
            "enabled": true,
            "allowUnsandboxedCommands": false,
            "failIfUnavailable": true,
            "filesystem": {"allowWrite": []},
        })
    );
    // A worker with `worker_sandbox = false` (no sandbox spec) still has none.
    let mut unsandboxed = worker(Runtime::Claude);
    unsandboxed.claude_sandbox = None;
    let settings = json_after(
        &claude(&unsandboxed, &new_session(), &CLI_CAPS),
        "--settings",
    );
    assert!(settings.get("sandbox").is_none(), "{settings}");
    assert_eq!(settings, plain);
}

#[path = "argv_caps_tests.rs"]
mod caps;
