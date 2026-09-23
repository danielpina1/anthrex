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
        effort: Effort::High,
        cwd: "/tmp/p/wt/t1".into(),
        instructions: "contract".into(),
        mcp: Some(McpTarget {
            role: AgentRole::Worker,
            run_id: "r-3f9a".into(),
            task_id: Some("t1".into()),
        }),
        allowed_tools: vec!["Bash".into()],
        claude_permission_mode: Some("acceptEdits".into()),
        claude_disallowed_tools: vec![],
        claude_sandbox: Some(ClaudeSandbox {
            writable_roots: vec!["/tmp/x/.git".into()],
        }),
        codex_sandbox: "workspace-write".into(),
        codex_writable_roots: vec![],
        env: vec![("A".into(), "1".into())],
        claude_auth: config::ClaudeAuth::ApiKey,
        api_key_helper: Some("/bin/key".into()),
        run_ref: None,
    };
    let json = serde_json::to_value(&spec).unwrap();
    assert_eq!(json["claude_auth"], "api_key");
    assert_eq!(serde_json::from_value::<HeadlessSpec>(json).unwrap(), spec);
}
