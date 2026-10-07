use super::*;
use crate::headless::claude_stream::ClaudeStream;
use proptest::prelude::*;

/// Both parsers on one line, with a fresh Claude parser and one that has just seen a
/// failed turn's category line (the only state it carries).
fn parse_both(line: &str) -> Vec<SessionEvent> {
    let mut primed = ClaudeStream::default();
    primed.parse_line(
        r#"{"type":"assistant","error":"rate_limit","message":{"content":[{"type":"text","text":"x"}]}}"#,
    );
    let mut events = ClaudeStream::default().parse_line(line);
    events.extend(primed.parse_line(line));
    events.extend(codex_stream::parse_line(line));
    events
}

#[test]
fn garbage_never_panics() {
    let huge = format!(
        r#"{{"type":"assistant","message":{{"content":"{}"}}}}"#,
        "x".repeat(5 << 20)
    );
    let cases = [
        "{not json".to_string(),
        "[1,2,3]".to_string(),
        huge,
        r#"{"type":"no.such.type","item":{}}"#.to_string(),
        r#"{"type":"result","subtype":"success","is_error":false}"#.to_string(),
        r#"{"type":"turn.completed"}"#.to_string(),
        r#"{"type":"item.completed","item":"not an object"}"#.to_string(),
        r#"{"type":"item.completed","item":{"type":"command_execution"}}"#.to_string(),
        r#"{"type":"user","message":{"content":[{"type":"tool_result"}]}}"#.to_string(),
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":7}]}}"#.to_string(),
        r#"{"type":"system","subtype":"api_retry","attempt":-1,"retry_delay_ms":"soon"}"#
            .to_string(),
        r#"{"type":"turn.failed","error":{"message":"{\"status\":\"x\"}"}}"#.to_string(),
        String::new(),
    ];
    for case in &cases {
        let events = parse_both(case);
        assert!(!events.is_empty());
        for event in events {
            if let SessionEvent::Unknown { line } = &event {
                assert!(line.chars().count() <= UNKNOWN_LINE_CHARS);
            }
        }
    }

    // A `result` or `turn.completed` with no `usage` is still a turn end, best effort.
    let ended = ClaudeStream::default()
        .parse_line(r#"{"type":"result","subtype":"success","is_error":false}"#);
    assert_eq!(
        ended,
        [SessionEvent::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: None,
            denials: vec![],
        }]
    );
    assert_eq!(
        codex_stream::parse_line(r#"{"type":"turn.completed"}"#),
        [SessionEvent::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: None,
            denials: vec![],
        }]
    );
    for garbage in ["{not json", "[1,2,3]", r#"{"type":"no.such.type"}"#] {
        assert!(matches!(
            ClaudeStream::default().parse_line(garbage).as_slice(),
            [SessionEvent::Unknown { .. }]
        ));
        assert!(matches!(
            codex_stream::parse_line(garbage).as_slice(),
            [SessionEvent::Unknown { .. }]
        ));
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1000))]

    #[test]
    fn garbage_never_panics_on_arbitrary_strings(line in any::<String>()) {
        prop_assert!(!parse_both(&line).is_empty());
    }

    #[test]
    fn garbage_never_panics_on_json_shaped_strings(
        kind in prop::sample::select(vec![
            "result", "assistant", "user", "system", "item.started", "item.completed",
            "turn.failed", "turn.completed", "thread.started",
        ]),
        body in any::<String>(),
    ) {
        let line = format!(r#"{{"type":"{kind}","subtype":{body:?},"item":{body:?},"message":{{"content":[{{"type":{body:?}}}]}}}}"#);
        prop_assert!(!parse_both(&line).is_empty());
    }
}

#[test]
fn a_headless_spec_round_trips_through_json() {
    let spec = HeadlessSpec {
        runtime: Runtime::Claude,
        model: "claude-sonnet-5".into(),
        effort: Effort::HIGH,
        cwd: "/tmp/p/wt/t1".into(),
        instructions: "contract".into(),
        mcp: Some(McpTarget {
            role: AgentRole::Worker,
            run_id: "r-3f9a".into(),
            task_id: Some("t1".into()),
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
            agent_label: None,
        }),
        allowed_tools: vec!["Bash".into()],
        claude_permission_mode: Some("acceptEdits".into()),
        claude_disallowed_tools: vec![],
        claude_sandbox: Some(ClaudeSandbox {
            deny_read: Vec::new(),
            writable_roots: vec!["/tmp/x/.git".into()],
            deny_write: vec![],
        }),
        codex_sandbox: "workspace-write".into(),
        codex_writable_roots: vec![],
        codex_read_only: Vec::new(),
        codex_grant_dialect: None,
        env: vec![("A".into(), "1".into())],
        claude_auth: config::ClaudeAuth::ApiKey,
        api_key_helper: Some("/bin/key".into()),
        run_ref: None,
        codex_config_guard: None,
        output_filter: None,
    };
    let json = serde_json::to_value(&spec).unwrap();
    assert_eq!(json["claude_auth"], "api_key");
    assert_eq!(serde_json::from_value::<HeadlessSpec>(json).unwrap(), spec);
}

/// Milestone 9 decision 31: a spec persisted before `McpTarget.epic` existed still
/// loads, with no epic.
#[test]
fn a_persisted_spec_without_an_epic_still_loads() {
    let target = McpTarget {
        role: AgentRole::Worker,
        run_id: "r-3f9a".into(),
        task_id: Some("t1".into()),
        scout_id: None,
        epic: None,
        chain: None,
        lane: None,
        agent_label: None,
    };
    let mut json = serde_json::to_value(&target).unwrap();
    json.as_object_mut().unwrap().remove("epic");
    assert_eq!(serde_json::from_value::<McpTarget>(json).unwrap(), target);
}

/// Milestone 9.3 (KG §3.4): a target persisted before `McpTarget.chain` existed still
/// loads, with no chain; one with a chain round-trips.
#[test]
fn persisted_mcp_target_without_chain_still_loads() {
    let target = McpTarget {
        role: AgentRole::Orchestrator,
        run_id: "r-3f9a".into(),
        task_id: None,
        scout_id: None,
        epic: None,
        chain: None,
        lane: None,
        agent_label: None,
    };
    let mut json = serde_json::to_value(&target).unwrap();
    json.as_object_mut().unwrap().remove("chain");
    assert_eq!(serde_json::from_value::<McpTarget>(json).unwrap(), target);
    let chained = McpTarget {
        chain: Some("o-3f9a".into()),
        ..target
    };
    let json = serde_json::to_value(&chained).unwrap();
    assert_eq!(json["chain"], "o-3f9a");
    assert_eq!(serde_json::from_value::<McpTarget>(json).unwrap(), chained);
}

/// Milestone 9.5 task 2: a target persisted before `McpTarget.lane` existed still loads,
/// with no lane; a racer's round-trips with its lane.
#[test]
fn persisted_mcp_target_without_lane_still_loads() {
    let target = McpTarget {
        role: AgentRole::Worker,
        run_id: "r-3f9a".into(),
        task_id: Some("t1".into()),
        scout_id: None,
        epic: None,
        chain: None,
        lane: None,
        agent_label: None,
    };
    let mut json = serde_json::to_value(&target).unwrap();
    json.as_object_mut().unwrap().remove("lane");
    assert_eq!(serde_json::from_value::<McpTarget>(json).unwrap(), target);
    let racer = McpTarget {
        role: AgentRole::Racer,
        lane: Some(proto::RaceLane::B),
        ..target
    };
    let json = serde_json::to_value(&racer).unwrap();
    assert_eq!(
        (&json["role"], &json["lane"]),
        (&"racer".into(), &"b".into())
    );
    assert_eq!(serde_json::from_value::<McpTarget>(json).unwrap(), racer);
}

/// Final fix batch F2 (review C, M2; round 2, N2): only a Claude `api_key` session keeps
/// the inherited Anthropic credentials, and no session keeps OpenAI's or Codex's.
#[test]
fn only_a_claude_api_key_session_keeps_the_api_credentials() {
    let base = HeadlessSpec {
        runtime: Runtime::Claude,
        model: String::new(),
        effort: Effort::MEDIUM,
        cwd: "/tmp/p/wt/t1".into(),
        instructions: String::new(),
        mcp: None,
        allowed_tools: vec![],
        claude_permission_mode: None,
        claude_disallowed_tools: vec![],
        claude_sandbox: None,
        codex_sandbox: "workspace-write".into(),
        codex_writable_roots: vec![],
        codex_read_only: Vec::new(),
        codex_grant_dialect: None,
        env: vec![],
        claude_auth: config::ClaudeAuth::Login,
        api_key_helper: None,
        run_ref: None,
        codex_config_guard: None,
        output_filter: None,
    };
    let scrub = |runtime, auth| {
        let spec = HeadlessSpec {
            runtime,
            claude_auth: auth,
            ..base.clone()
        };
        credential_scrub(&spec).to_vec()
    };
    let openai = [
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "CODEX_ACCESS_TOKEN",
        "OPENAI_BASE_URL",
    ];
    let all = [
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "CODEX_ACCESS_TOKEN",
        "OPENAI_BASE_URL",
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "ANTHROPIC_BASE_URL",
    ];
    assert_eq!(scrub(Runtime::Claude, config::ClaudeAuth::Login), all);
    assert_eq!(scrub(Runtime::Claude, config::ClaudeAuth::ApiKey), openai);
    assert_eq!(scrub(Runtime::Codex, config::ClaudeAuth::Login), all);
    assert_eq!(scrub(Runtime::Codex, config::ClaudeAuth::ApiKey), all);
}

/// The Claude tool-search fix (2026-09-27): a Claude session's process gets
/// `ENABLE_TOOL_SEARCH=false`, exactly once and last, whatever `env` says; a Codex
/// session's `env` is passed through unchanged.
#[test]
fn a_claude_session_pins_tool_search_off_and_a_codex_session_does_not() {
    let env = vec![
        ("CARGO_TARGET_DIR".to_string(), "/wt/target".to_string()),
        ("ENABLE_TOOL_SEARCH".to_string(), "true".to_string()),
    ];
    let claude = session_vars(Runtime::Claude, &env);
    assert_eq!(
        claude,
        [
            ("CARGO_TARGET_DIR".to_string(), "/wt/target".to_string()),
            ("ENABLE_TOOL_SEARCH".to_string(), "false".to_string()),
        ]
    );
    assert_eq!(session_vars(Runtime::Codex, &env), env);
    assert_eq!(
        session_vars(Runtime::Claude, &[]),
        [("ENABLE_TOOL_SEARCH".to_string(), "false".to_string())]
    );
}

/// Milestone 9.6, task 8's re-review (m3): `never_resumed` refuses only a design
/// agent's session; a worker's and a task reviewer's still resume, on either runtime.
#[test]
fn only_a_design_agents_session_is_never_resumed() {
    let spec = |runtime, role| HeadlessSpec {
        runtime,
        model: "m".into(),
        effort: Effort::HIGH,
        cwd: "/tmp/p/wt/t1".into(),
        instructions: "contract".into(),
        mcp: Some(McpTarget {
            role,
            run_id: "r-3f9a".into(),
            task_id: Some("t1".into()),
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
            agent_label: None,
        }),
        allowed_tools: Vec::new(),
        claude_permission_mode: None,
        claude_disallowed_tools: Vec::new(),
        claude_sandbox: None,
        codex_sandbox: "read-only".into(),
        codex_writable_roots: Vec::new(),
        codex_read_only: Vec::new(),
        codex_grant_dialect: None,
        env: Vec::new(),
        claude_auth: config::ClaudeAuth::default(),
        api_key_helper: None,
        run_ref: None,
        codex_config_guard: None,
        output_filter: None,
    };
    for runtime in [Runtime::Claude, Runtime::Codex] {
        for role in [AgentRole::Worker, AgentRole::Reviewer] {
            assert!(never_resumed(7, &spec(runtime, role)).is_ok(), "{role:?}");
        }
        for role in [AgentRole::Brainstormer, AgentRole::DocReviewer] {
            let error = never_resumed(7, &spec(runtime, role)).unwrap_err();
            assert!(error.to_string().contains("never resumed"), "{role:?}");
        }
    }
}
