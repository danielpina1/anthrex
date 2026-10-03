//! Milestone 9 decisions 7, 8 and 10, with the M9.1 real-CLI rulings: the orchestrator's
//! whole argv and environment on both runtimes, and the user's own windows unchanged.

use super::*;
use crate::headless::argv::CLI_CAPS;
use crate::launch::{LaunchContext, LaunchPlan, claude, plan};
use proto::{AgentRole, WindowSpec};
use std::path::Path;

const TOKEN: &str = "0123456789abcdef0123456789abcdef";
const SOCKET: &str = "/tmp/a.sock";
const EXE: &str = "/opt/anthrex/bin/anthrex";

fn role() -> RoleLaunch {
    RoleLaunch {
        run_ref: RunRef {
            run_id: "r-7a2c".into(),
            task_id: None,
            role: AgentRole::Orchestrator,
            session: 1,
        },
        mcp: McpTarget {
            role: AgentRole::Orchestrator,
            run_id: "r-7a2c".into(),
            task_id: None,
            scout_id: None,
            epic: None,
            chain: None,
        },
        instructions: "THE CONTRACT".into(),
        effort: Effort::High,
        claude_allowed_tools: ORCHESTRATOR_ALLOWED_TOOLS
            .iter()
            .map(|t| t.to_string())
            .collect(),
        claude_disallowed_tools: ORCHESTRATOR_DISALLOWED_TOOLS
            .iter()
            .map(|t| t.to_string())
            .collect(),
        env: otlp_env("http://127.0.0.1:4318", "r-7a2c", TOKEN),
        remove_env: vec!["ANTHROPIC_API_KEY".into(), "OPENAI_API_KEY".into()],
    }
}

fn spec(runtime: Runtime, model: &str) -> WindowSpec {
    WindowSpec {
        name: Some("7a2c/orchestrator".into()),
        runtime,
        cwd: "/tmp/repo".into(),
        worktree_branch: None,
        model: Some(model.into()),
        initial_prompt: Some("plan the goal".into()),
    }
}

fn ctx<'a>(role: Option<&'a RoleLaunch>, caps: &'a CliCaps) -> LaunchContext<'a> {
    LaunchContext {
        window_id: 4,
        name: "7a2c/orchestrator",
        socket_path: Path::new(SOCKET),
        shell: "/bin/zsh",
        exe: Path::new(EXE),
        claude_bin: "/opt/agents/claude",
        codex_bin: "/opt/agents/codex",
        codex_hook_source: None,
        codex_bypass_hook_trust: false,
        resume: None,
        caps,
        role,
    }
}

fn settings() -> String {
    serde_json::to_string(&claude::settings(Path::new(EXE), 4)).unwrap()
}

const MCP_ARGS: &str =
    r#"["mcp","--role","orchestrator","--run","r-7a2c","--window","4","--socket","/tmp/a.sock"]"#;

fn mcp_config() -> String {
    format!(
        r#"{{"mcpServers":{{"anthrex":{{"args":{MCP_ARGS},"command":"{EXE}","type":"stdio"}}}}}}"#
    )
}

const ALLOWED: &str = "mcp__anthrex__get_context,mcp__anthrex__spawn_scout,mcp__anthrex__spawn_subplanner,mcp__anthrex__edit_plan,mcp__anthrex__run_status,mcp__anthrex__task_result,mcp__anthrex__start_goal,Read,Glob,Grep";
const DISALLOWED: &str = "Edit,Write,NotebookEdit,Bash,Agent,Task,Artifact,CronCreate,CronDelete,RemoteTrigger,PushNotification,SendMessage,Workflow,WebFetch,WebSearch,Monitor,EnterWorktree,ExitWorktree,ScheduleWakeup,DesignSync";

/// Decision 7's order, with rulings 1 and 2: the user-settings-only flags, the MCP
/// server, the tool lists, `--permission-mode default` after `--disallowedTools`, the
/// contract and the effort, then M3's `--model` and `--` before the prompt; on resume,
/// `--resume <id>` instead.
#[test]
fn claude_orchestrator_argv_is_exact() {
    let role = role();
    let p = plan(&spec(Runtime::Claude, "opus"), &ctx(Some(&role), &CLI_CAPS));
    let settings = settings();
    let mcp = mcp_config();
    let middle = [
        "--setting-sources",
        "user",
        "--strict-mcp-config",
        "--mcp-config",
        &mcp,
        "--allowedTools",
        ALLOWED,
        "--disallowedTools",
        DISALLOWED,
        "--permission-mode",
        "default",
        "--append-system-prompt",
        "THE CONTRACT",
        "--effort",
        "high",
    ];
    let mut expected = vec!["--name", "7a2c/orchestrator", "--settings", &settings];
    expected.extend(middle);
    expected.extend(["--model", "opus", "--", "plan the goal"]);
    assert_eq!(p.program, "/opt/agents/claude");
    assert_eq!(p.args, expected);

    let mut resumed = ctx(Some(&role), &CLI_CAPS);
    resumed.resume = Some("sess-1");
    let p = plan(&spec(Runtime::Claude, "opus"), &resumed);
    let mut expected = vec!["--name", "7a2c/orchestrator", "--settings", &settings];
    expected.extend(middle);
    expected.extend(["--model", "opus", "--resume", "sess-1"]);
    assert_eq!(p.args, expected);
}

/// Decision 7: with no user-settings-only flags in the caps, neither
/// `--setting-sources` nor `--strict-mcp-config` appears (both come from the caps), and
/// no `--effort` without the effort flag. Everything else is unchanged.
#[test]
fn claude_argv_without_user_settings_only_caps() {
    let role = role();
    let caps = CliCaps {
        claude_user_settings_only: None,
        claude_effort_flag: false,
        ..CLI_CAPS
    };
    let args = claude_role_args(&role, &ctx(Some(&role), &caps), &caps);
    let mcp = mcp_config();
    assert_eq!(
        args,
        [
            "--mcp-config",
            &mcp,
            "--allowedTools",
            ALLOWED,
            "--disallowedTools",
            DISALLOWED,
            "--permission-mode",
            "default",
            "--append-system-prompt",
            "THE CONTRACT",
        ]
    );
}

/// Decision 8, with ruling 3 (`"approve"`): the role's block after M3's hook block and
/// before `-m`; the project-config exclusion when the caps have one; `-s read-only -a
/// on-request` ahead of `-m` and of `resume <id>`. No trust flag (ruling 7).
#[test]
fn codex_orchestrator_argv_is_exact() {
    let role = role();
    let p = plan(
        &spec(Runtime::Codex, "gpt-5.6"),
        &ctx(Some(&role), &CLI_CAPS),
    );
    let head = [
        "-C",
        "/tmp/repo",
        "-c",
        r#"notify=["/opt/anthrex/bin/anthrex","hook","--window","4","--source","codex-notify"]"#,
        "-c",
        r#"tui.terminal_title=["status"]"#,
        "-c",
        r#"tui.notifications=["approval-requested"]"#,
        "-c",
        r#"tui.notification_method="bel""#,
        "-c",
        r#"tui.notification_condition="always""#,
    ];
    let command = format!(r#"mcp_servers.anthrex.command="{EXE}""#);
    let args = format!("mcp_servers.anthrex.args={MCP_ARGS}");
    let block = [
        "-c",
        &command,
        "-c",
        &args,
        "-c",
        "mcp_servers.anthrex.tool_timeout_sec=120",
        "-c",
        r#"mcp_servers.anthrex.default_tools_approval_mode="approve""#,
        "-c",
        r#"developer_instructions="THE CONTRACT""#,
        "-c",
        r#"model_reasoning_effort="high""#,
    ];
    let tail = ["-s", "read-only", "-a", "on-request"];
    let mut expected: Vec<&str> = head.to_vec();
    expected.extend(block);
    expected.extend(tail);
    expected.extend(["-m", "gpt-5.6", "--", "plan the goal"]);
    assert_eq!(p.program, "/opt/agents/codex");
    assert_eq!(p.args, expected);
    assert!(!p.args.iter().any(|a| a.contains("trust_level")));

    let mut resumed = ctx(Some(&role), &CLI_CAPS);
    resumed.resume = Some("thr-1");
    let p = plan(&spec(Runtime::Codex, "gpt-5.6"), &resumed);
    let mut expected: Vec<&str> = head.to_vec();
    expected.extend(block);
    expected.extend(tail);
    expected.extend(["-m", "gpt-5.6", "resume", "thr-1"]);
    assert_eq!(p.args, expected);

    // Decision 9: the exclusion flags when the CLI has them, before `-s`.
    let caps = CliCaps {
        codex_user_config_only: Some(&["--no-project-config"]),
        ..CLI_CAPS
    };
    let args = codex_role_args(&role, &ctx(Some(&role), &caps), &caps);
    let mut expected: Vec<&str> = block.to_vec();
    expected.push("--no-project-config");
    expected.extend(tail);
    assert_eq!(args, expected);
}

/// The user's own windows: `role: None` adds no flag, no variable and no scrub, for
/// every runtime (M3's argv tests in `launch/mod.rs` pass untouched).
#[test]
fn plain_windows_are_unchanged() {
    let four = |p: &LaunchPlan| {
        let keys: Vec<&str> = p.env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(
            keys,
            ["TERM", "COLORTERM", "ANTHREX_WINDOW_ID", "ANTHREX_SOCKET"]
        );
        assert!(!p.scrub_agent_env);
        assert!(p.remove_env.is_empty());
    };
    let p = plan(&spec(Runtime::Claude, "opus"), &ctx(None, &CLI_CAPS));
    let settings = settings();
    assert_eq!(
        p.args,
        [
            "--name",
            "7a2c/orchestrator",
            "--settings",
            &settings,
            "--model",
            "opus",
            "--",
            "plan the goal"
        ]
    );
    four(&p);
    let p = plan(&spec(Runtime::Codex, "gpt-5.6"), &ctx(None, &CLI_CAPS));
    assert_eq!(p.args.len(), 16, "{:?}", p.args);
    assert_eq!(&p.args[12..], ["-m", "gpt-5.6", "--", "plan the goal"]);
    four(&p);
    let p = plan(&spec(Runtime::Shell, "x"), &ctx(None, &CLI_CAPS));
    assert_eq!(p.args, ["-l"]);
    four(&p);
}

/// Decision 10 with ruling 4: M3's four variables first, then the OTLP variables and
/// the token header, then `ENABLE_TOOL_SEARCH=false` last. No `MCP_TOOL_TIMEOUT`.
#[test]
fn role_env_order_and_otlp() {
    let role = role();
    let p = plan(&spec(Runtime::Claude, "opus"), &ctx(Some(&role), &CLI_CAPS));
    let env: Vec<(&str, &str)> = p
        .env
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let header = format!("authorization=Bearer {TOKEN}");
    assert_eq!(
        env,
        [
            ("TERM", "xterm-256color"),
            ("COLORTERM", "truecolor"),
            ("ANTHREX_WINDOW_ID", "4"),
            ("ANTHREX_SOCKET", SOCKET),
            ("CLAUDE_CODE_ENABLE_TELEMETRY", "1"),
            ("OTEL_METRICS_EXPORTER", "otlp"),
            ("OTEL_EXPORTER_OTLP_PROTOCOL", "http/json"),
            ("OTEL_EXPORTER_OTLP_ENDPOINT", "http://127.0.0.1:4318"),
            ("OTEL_METRIC_EXPORT_INTERVAL", "1000"),
            (
                "OTEL_RESOURCE_ATTRIBUTES",
                "anthrex.run=r-7a2c,anthrex.role=orchestrator"
            ),
            ("OTEL_EXPORTER_OTLP_HEADERS", header.as_str()),
            ("ENABLE_TOOL_SEARCH", "false"),
        ]
    );
    assert!(!p.env.iter().any(|(k, _)| k.starts_with("MCP_")));
    assert!(p.scrub_agent_env);
    assert_eq!(p.remove_env, ["ANTHROPIC_API_KEY", "OPENAI_API_KEY"]);
}

/// Decision 10: tool search is off for a Claude orchestrator, last and once even when the
/// role's own env names it; a Codex orchestrator never gets it.
#[test]
fn claude_orchestrator_env_turns_tool_search_off_and_codex_does_not() {
    let mut role = role();
    role.env
        .insert(0, ("ENABLE_TOOL_SEARCH".into(), "true".into()));
    let env = launch_env(&role, Runtime::Claude);
    assert_eq!(
        env.last(),
        Some(&("ENABLE_TOOL_SEARCH".to_string(), "false".to_string()))
    );
    assert_eq!(
        env.iter()
            .filter(|(k, _)| k == "ENABLE_TOOL_SEARCH")
            .count(),
        1
    );

    let mut codex = self::role();
    codex.env = Vec::new();
    let p = plan(
        &spec(Runtime::Codex, "gpt-5.6"),
        &ctx(Some(&codex), &CLI_CAPS),
    );
    assert!(!p.env.iter().any(|(k, _)| k == "ENABLE_TOOL_SEARCH"));
    assert_eq!(p.env.len(), 4, "{:?}", p.env);
    assert!(p.scrub_agent_env);
}

/// M9.10 review: a persisted role missing a field a later daemon made optional still
/// parses, so an upgrade rarely strands an orchestrator window. The fields that make it
/// read-only (the tool lists, the contract, the effort) are never defaulted.
#[test]
fn a_role_record_missing_optional_fields_still_parses() {
    let mut value = serde_json::to_value(role()).unwrap();
    let object = value.as_object_mut().unwrap();
    object.remove("env");
    object.remove("remove_env");
    object["run_ref"].as_object_mut().unwrap().remove("task_id");
    let mcp = object["mcp"].as_object_mut().unwrap();
    for key in ["task_id", "scout_id", "epic"] {
        mcp.remove(key);
    }
    let parsed: RoleLaunch = serde_json::from_value(value).expect("parses");
    assert_eq!(parsed.run_ref, role().run_ref);
    assert_eq!(parsed.mcp, role().mcp);
    assert!(parsed.env.is_empty() && parsed.remove_env.is_empty());
    for required in [
        "claude_disallowed_tools",
        "claude_allowed_tools",
        "instructions",
        "effort",
    ] {
        let mut value = serde_json::to_value(role()).unwrap();
        value.as_object_mut().unwrap().remove(required);
        assert!(
            serde_json::from_value::<RoleLaunch>(value).is_err(),
            "{required}"
        );
    }
}

/// Milestone 9.3 task M9.3.7 fix round 1 (review I1): Claude's allowlist and the MCP
/// crate's orchestrator tools never drift. Every tool `anthrex mcp` lists for the
/// orchestrator is pre-allowed as `mcp__anthrex__<name>` (a tool neither allowed nor
/// disallowed asks the user, decision 7), and every `mcp__anthrex__` entry is one of
/// them; the rest are decision 7's read-only tools.
#[test]
fn the_orchestrators_allowlist_names_every_mcp_tool_and_no_other() {
    let tools: Vec<String> = mcp::tools::tools_for(AgentRole::Orchestrator)
        .iter()
        .map(|t| format!("mcp__anthrex__{}", t.name))
        .collect();
    for tool in &tools {
        assert!(
            ORCHESTRATOR_ALLOWED_TOOLS.contains(&tool.as_str()),
            "{tool} is not pre-allowed"
        );
    }
    let (anthrex, rest): (Vec<&str>, Vec<&str>) = ORCHESTRATOR_ALLOWED_TOOLS
        .iter()
        .partition(|t| t.starts_with("mcp__anthrex__"));
    assert_eq!(
        anthrex, tools,
        "the allowlist's anthrex tools, in the MCP order"
    );
    assert_eq!(rest, ["Read", "Glob", "Grep"]);
}
