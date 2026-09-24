use super::*;
use crate::headless::test_support::{
    CLAUDE_DOCUMENTED, CLAUDE_INPUT, CLAUDE_PROJECT_SETTINGS, CLAUDE_SANDBOX, CLAUDE_STREAM,
    line_type, lines, unmodelled,
};
use crate::headless::{SessionEvent, TurnOutcome};
use proto::TokenUsage;
use serde_json::{Value, json};

/// Every event of a whole fixture, parsed in order with one parser.
fn parse_all(fixture: &str) -> Vec<SessionEvent> {
    let mut parser = ClaudeStream::default();
    lines(fixture)
        .into_iter()
        .flat_map(|line| parser.parse_line(line))
        .collect()
}

/// A compact label for the events the sequence test cares about; `None` for the ones it
/// skips (recognised-but-inert lines, thinking blocks, echoed user text).
fn label(event: &SessionEvent) -> Option<String> {
    Some(match event {
        SessionEvent::Init { .. } => "Init".into(),
        SessionEvent::TurnStarted => "TurnStarted".into(),
        SessionEvent::AssistantText { text, parent } => format!("Text({text},{parent:?})"),
        SessionEvent::ToolUse {
            id, name, parent, ..
        } => format!("ToolUse({name},{id},{parent:?})"),
        SessionEvent::ToolResult { id, ok, parent, .. } => {
            format!("ToolResult({id},{ok},{parent:?})")
        }
        SessionEvent::PermissionDenied { tool, .. } => format!("PermissionDenied({tool})"),
        SessionEvent::TurnEnded {
            outcome, denials, ..
        } => {
            let outcome = match outcome {
                TurnOutcome::Completed => "Completed".to_string(),
                TurnOutcome::Interrupted => "Interrupted".to_string(),
                TurnOutcome::Failed { kind, .. } => format!("Failed({kind:?})"),
            };
            format!("TurnEnded({outcome},{denials:?})")
        }
        SessionEvent::Other { .. } | SessionEvent::UserText { .. } => return None,
        other => format!("{other:?}"),
    })
}

#[test]
fn claude_stream_parses_every_fixture_line() {
    let unmodelled = unmodelled();
    let mut parser = ClaudeStream::default();
    let mut events = Vec::new();
    for line in lines(CLAUDE_STREAM) {
        let parsed = parser.parse_line(line);
        assert!(!parsed.is_empty(), "no event for {line}");
        for event in &parsed {
            if matches!(event, SessionEvent::Unknown { .. }) {
                let kind = line_type(line);
                assert!(unmodelled.contains(&kind), "{kind} parsed as Unknown");
            }
        }
        events.extend(parsed);
    }

    let sequence: Vec<String> = events.iter().filter_map(label).collect();
    let expected = [
        "Init",
        "TurnStarted",
        "ToolUse(Bash,toolu_fixture01,None)",
        "ToolResult(toolu_fixture01,true,None)",
        "ToolUse(Agent,toolu_fixture02,None)",
        "ToolResult(toolu_fixture02,true,None)",
        "Text(Done.,None)",
        "TurnEnded(Completed,[])",
        "Init",
        "TurnStarted",
        "ToolUse(Write,toolu_fixture03,None)",
        "PermissionDenied(Write)",
        "ToolResult(toolu_fixture03,false,None)",
        "Text(Write blocked: path /tmp/fixture/outside.txt is outside allowed working directories.,None)",
        "TurnEnded(Completed,[\"Write\"])",
        "Init",
        "TurnStarted",
        "ToolUse(Bash,toolu_fixture04,None)",
        "ToolResult(toolu_fixture04,false,None)",
        "TurnEnded(Interrupted,[])",
    ];
    assert_eq!(sequence, expected);

    // The first `Init` carries the session, the model and the anthrex MCP server's state.
    assert_eq!(
        events
            .iter()
            .find(|e| matches!(e, SessionEvent::Init { .. })),
        Some(&SessionEvent::Init {
            session_id: "00000000-0000-4000-8000-000000000003".into(),
            model: Some("claude-haiku-4-5-20251001".into()),
            mcp_ok: Some(true),
        })
    );
    // The Bash call's input and result text survive.
    assert!(events.contains(&SessionEvent::ToolUse {
        id: "toolu_fixture01".into(),
        name: "Bash".into(),
        input: json!({"command": "ls", "description": "List files in current directory"}),
        parent: None,
    }));
    assert!(events.contains(&SessionEvent::ToolResult {
        id: "toolu_fixture01".into(),
        text: "README.md".into(),
        ok: true,
        parent: None,
    }));
    // The sub-agent's prompt is echoed as user text, and the hand-back's text parts are
    // joined into the Agent call's result.
    assert!(events.contains(&SessionEvent::UserText {
        text: "Reply with the single word ok. Use no tools.".into()
    }));
    let agent_result = events.iter().find_map(|e| match e {
        SessionEvent::ToolResult { id, text, .. } if id == "toolu_fixture02" => Some(text),
        _ => None,
    });
    assert!(
        agent_result
            .unwrap()
            .starts_with("[Subagent hand-back] The text below")
    );
    // Decision_reason wins over message for a denial's reason.
    assert!(events.contains(&SessionEvent::PermissionDenied {
        tool: "Write".into(),
        reason:
            "no approval surface in this session; permission request denied automatically".into(),
    }));
}

#[test]
fn claude_permission_denied_falls_back_to_the_message() {
    let line = r#"{"type":"system","subtype":"permission_denied","tool_name":"Bash","tool_use_id":"t9","message":"Permission for this tool use was denied."}"#;
    assert_eq!(
        ClaudeStream::default().parse_line(line),
        vec![SessionEvent::PermissionDenied {
            tool: "Bash".into(),
            reason: "Permission for this tool use was denied.".into(),
        }]
    );
}

#[test]
fn claude_usage_is_per_turn() {
    // M8a.1: `result.usage` is per turn in a streaming-input session, so the parser passes
    // it through; the cumulative figures are `total_cost_usd` and `modelUsage`.
    let usages: Vec<Option<TokenUsage>> = parse_all(CLAUDE_STREAM)
        .into_iter()
        .filter_map(|e| match e {
            SessionEvent::TurnEnded { usage, .. } => Some(usage),
            _ => None,
        })
        .collect();
    assert_eq!(
        usages,
        [
            Some(TokenUsage {
                input: 26,
                output: 477,
                cache_read: 66953,
                cache_write: 7175,
            }),
            Some(TokenUsage {
                input: 18,
                output: 284,
                cache_read: 50323,
                cache_write: 526,
            }),
            Some(TokenUsage {
                input: 10,
                output: 215,
                cache_read: 25616,
                cache_write: 183,
            }),
        ]
    );
}

#[test]
fn claude_failed_results_are_classified() {
    let events = parse_all(CLAUDE_DOCUMENTED);
    let retries: Vec<&SessionEvent> = events
        .iter()
        .filter(|e| matches!(e, SessionEvent::ApiRetry { .. }))
        .collect();
    assert_eq!(
        retries,
        [
            &SessionEvent::ApiRetry {
                error: "rate_limit".into(),
                attempt: 1,
                delay_ms: 1000,
            },
            &SessionEvent::ApiRetry {
                error: "overloaded".into(),
                attempt: 2,
                delay_ms: 2000,
            },
            &SessionEvent::ApiRetry {
                error: "server_error".into(),
                attempt: 3,
                delay_ms: 4000,
            },
        ]
    );
    assert!(events.contains(&SessionEvent::Compacted));

    let outcomes: Vec<TurnOutcome> = events
        .into_iter()
        .filter_map(|e| match e {
            SessionEvent::TurnEnded { outcome, .. } => Some(outcome),
            _ => None,
        })
        .collect();
    assert_eq!(
        outcomes,
        [
            TurnOutcome::Failed {
                error: "You've hit your session limit · resets 3:45pm".into(),
                kind: FailureKind::RateLimit,
            },
            TurnOutcome::Failed {
                error: "Credit balance is too low".into(),
                kind: FailureKind::Billing,
            },
            TurnOutcome::Failed {
                error: "Not logged in · Please run /login".into(),
                kind: FailureKind::Authentication,
            },
        ]
    );
}

/// The documented rate-limit pair, with its assistant line's category replaced.
fn failed_pair(category: &str) -> (String, String) {
    let all = lines(CLAUDE_DOCUMENTED);
    let mut assistant: Value = serde_json::from_str(all[4]).unwrap();
    assistant["error"] = json!(category);
    (assistant.to_string(), all[5].to_string())
}

#[test]
fn an_unknown_error_category_is_other() {
    let (assistant, result) = failed_pair("overloaded");
    let mut parser = ClaudeStream::default();
    parser.parse_line(&assistant);
    let ended = parser.parse_line(&result);
    assert!(matches!(
        ended.as_slice(),
        [SessionEvent::TurnEnded {
            outcome: TurnOutcome::Failed {
                kind: FailureKind::Other,
                ..
            },
            ..
        }]
    ));
}

#[test]
fn a_category_does_not_leak_into_the_next_turn() {
    // A rate-limited turn, then a turn that fails with no category line before it: the
    // second must not inherit `RateLimit`.
    let (assistant, result) = failed_pair("rate_limit");
    let mut parser = ClaudeStream::default();
    parser.parse_line(&assistant);
    parser.parse_line(&result);
    let ended = parser.parse_line(&result);
    assert!(matches!(
        ended.as_slice(),
        [SessionEvent::TurnEnded {
            outcome: TurnOutcome::Failed {
                kind: FailureKind::Other,
                ..
            },
            ..
        }]
    ));
}

#[test]
fn a_failed_resume_and_an_unavailable_sandbox_are_failed_turns() {
    // M8a.1 item 4: a resume of an unknown session prints one `result` and exits.
    let resume = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"num_turns":0,"errors":["No conversation found with session ID: 0f0e"],"session_id":"0f0e"}"#;
    assert_eq!(
        ClaudeStream::default().parse_line(resume),
        vec![SessionEvent::TurnEnded {
            outcome: TurnOutcome::Failed {
                error: "No conversation found with session ID: 0f0e".into(),
                kind: FailureKind::Other,
            },
            usage: None,
            denials: vec![],
        }]
    );
    // Item 4b: the SDK reports a sandbox that cannot start in `errors`.
    let sandbox = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["sandbox required but unavailable: bubblewrap not found"]}"#;
    assert!(matches!(
        ClaudeStream::default().parse_line(sandbox).as_slice(),
        [SessionEvent::TurnEnded {
            outcome: TurnOutcome::Failed {
                kind: FailureKind::SandboxUnavailable,
                ..
            },
            ..
        }]
    ));
}

#[test]
fn a_sandbox_refusal_is_a_failed_tool_result_not_a_denial() {
    let events = parse_all(CLAUDE_SANDBOX);
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, SessionEvent::PermissionDenied { .. }))
    );
    assert!(events.contains(&SessionEvent::ToolResult {
        id: "toolu_fixture06".into(),
        text: "Exit code 1\ntouch: ../escape.txt: Operation not permitted".into(),
        ok: false,
        parent: None,
    }));
    let ended = events
        .iter()
        .filter(|e| {
            matches!(
                e,
                SessionEvent::TurnEnded {
                    outcome: TurnOutcome::Completed,
                    ..
                }
            )
        })
        .count();
    assert_eq!(ended, 2);
    // The project-settings recording parses with nothing unknown outside the list.
    let unmodelled = unmodelled();
    let mut parser = ClaudeStream::default();
    for line in lines(CLAUDE_PROJECT_SETTINGS)
        .into_iter()
        .chain(lines(CLAUDE_SANDBOX))
    {
        for event in parser.parse_line(line) {
            if matches!(event, SessionEvent::Unknown { .. }) {
                assert!(unmodelled.contains(&line_type(line)), "{line}");
            }
        }
    }
}

#[test]
fn claude_user_message_matches_the_recorded_envelope() {
    let recorded = lines(CLAUDE_INPUT)[0];
    let session = "00000000-0000-4000-8000-000000000003";
    let mut expected: Value = serde_json::from_str(recorded).unwrap();
    expected["message"]["content"][0]["text"] = json!("hello");

    let written = user_message("hello", Some(session));
    assert!(!written.contains('\n'));
    assert_eq!(serde_json::from_str::<Value>(&written).unwrap(), expected);
    // Key order too: the recorded line with its text replaced, byte for byte.
    let recorded_text = serde_json::to_string(
        serde_json::from_str::<Value>(recorded).unwrap()["message"]["content"][0]["text"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(written, recorded.replace(&recorded_text, "\"hello\""));

    // M8a.1 found the interrupt control request accepted (item 2).
    let interrupt = lines(CLAUDE_INPUT)[3];
    assert_eq!(interrupt_request(1), interrupt);
    assert_eq!(
        serde_json::from_str::<Value>(&interrupt_request(42)).unwrap()["request_id"],
        json!("42")
    );
}

#[test]
fn a_tool_result_is_cut_on_a_char_boundary() {
    let result = |content: &str| {
        let line = json!({
            "type": "user",
            "message": {"role": "user", "content": [
                {"type": "tool_result", "tool_use_id": "t1", "content": content, "is_error": false}
            ]},
            "parent_tool_use_id": null,
        })
        .to_string();
        match ClaudeStream::default().parse_line(&line).as_slice() {
            [SessionEvent::ToolResult { text, .. }] => text.clone(),
            other => panic!("{other:?}"),
        }
    };
    // 4096 is not a multiple of 3, so the cut must back off one byte.
    let wide = result(&"世".repeat(3000));
    assert_eq!(wide.len(), 4095);
    assert_eq!(wide.chars().count(), 1365);
    assert_eq!(result(&"a".repeat(5000)).len(), 4096);

    let line = "世".repeat(1000);
    match ClaudeStream::default().parse_line(&line).as_slice() {
        [SessionEvent::Unknown { line: kept }] => {
            assert_eq!(kept.chars().count(), 300);
            assert_eq!(*kept, "世".repeat(300));
        }
        other => panic!("{other:?}"),
    }
}

fn failed_kind(events: &[SessionEvent]) -> FailureKind {
    match events {
        [
            SessionEvent::TurnEnded {
                outcome: TurnOutcome::Failed { kind, .. },
                ..
            },
        ] => *kind,
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_failure_category_comes_only_from_the_top_level_line_of_the_same_turn() {
    let (assistant, _) = failed_pair("rate_limit");
    let failed =
        r#"{"type":"result","subtype":"error_during_execution","is_error":true,"errors":["boom"]}"#;
    // A category whose `result` never came (the line was cut), then a new turn.
    let mut parser = ClaudeStream::default();
    parser.parse_line(&assistant);
    parser.parse_line(lines(CLAUDE_STREAM)[12]);
    assert_eq!(failed_kind(&parser.parse_line(failed)), FailureKind::Other);

    // A sub-agent's failed API call is not the turn's.
    let mut sub: Value = serde_json::from_str(&assistant).unwrap();
    sub["error"] = json!("billing_error");
    sub["parent_tool_use_id"] = json!("toolu_1");
    let mut parser = ClaudeStream::default();
    parser.parse_line(&sub.to_string());
    let max_turns = r#"{"type":"result","subtype":"error_max_turns","is_error":true}"#;
    assert_eq!(
        failed_kind(&parser.parse_line(max_turns)),
        FailureKind::Other
    );
}

#[test]
fn a_known_type_in_an_unknown_shape_is_unknown() {
    for line in [
        r#"{"type":"user","message":{"role":"user","content":"hello"}}"#,
        r#"{"type":"assistant","message":{"content":[]}}"#,
        r#"{"type":"assistant","message":{"content":[{"type":"tool_use","id":7}]}}"#,
        r#"{"type":"user"}"#,
    ] {
        assert!(
            matches!(
                ClaudeStream::default().parse_line(line).as_slice(),
                [SessionEvent::Unknown { .. }]
            ),
            "{line}"
        );
    }
}

#[test]
fn an_aborted_stream_is_an_interrupt() {
    let line = r#"{"type":"result","subtype":"error_during_execution","is_error":true,"terminal_reason":"aborted_streaming"}"#;
    assert!(matches!(
        ClaudeStream::default().parse_line(line).as_slice(),
        [SessionEvent::TurnEnded {
            outcome: TurnOutcome::Interrupted,
            ..
        }]
    ));
}

#[test]
fn mcp_ok_reads_the_anthrex_server_status() {
    let init = |servers: &str| {
        let line = format!(
            r#"{{"type":"system","subtype":"init","session_id":"s","mcp_servers":{servers}}}"#
        );
        match ClaudeStream::default().parse_line(&line).first() {
            Some(SessionEvent::Init { mcp_ok, .. }) => *mcp_ok,
            other => panic!("{other:?}"),
        }
    };
    assert_eq!(
        init(r#"[{"name":"anthrex","status":"connected"}]"#),
        Some(true)
    );
    assert_eq!(
        init(r#"[{"name":"other","status":"connected"},{"name":"anthrex","status":"failed"}]"#),
        Some(false)
    );
    assert_eq!(init(r#"[{"name":"other","status":"connected"}]"#), None);
}

#[test]
fn tool_result_text_parts_are_joined_by_newlines() {
    let line = json!({"type": "user", "parent_tool_use_id": null, "message": {"content": [
        {"type": "tool_result", "tool_use_id": "t1", "is_error": false,
         "content": [{"type": "text", "text": "one"}, {"type": "image"}, {"type": "text", "text": "two"}]}
    ]}})
    .to_string();
    assert_eq!(
        ClaudeStream::default().parse_line(&line),
        [SessionEvent::ToolResult {
            id: "t1".into(),
            text: "one\ntwo".into(),
            ok: true,
            parent: None,
        }]
    );
}

/// M8a.24: a top-level `assistant` line carrying an `error` category (the synthetic
/// message a failed API turn ends with) is that failure's text, not the model's: `ApiErrorText`, which
/// the engine does not count as progress, so a retry streak that ran straight into the
/// failure stays one rate-limit event (decision 32).
#[test]
fn a_failed_api_turns_synthetic_message_is_api_error_text() {
    let events = parse_all(CLAUDE_DOCUMENTED);
    let errors: Vec<&SessionEvent> = events
        .iter()
        .filter(|e| matches!(e, SessionEvent::ApiErrorText { .. }))
        .collect();
    assert_eq!(
        errors,
        [
            &SessionEvent::ApiErrorText {
                text: "You've hit your session limit · resets 3:45pm".into()
            },
            &SessionEvent::ApiErrorText {
                text: "Credit balance is too low".into()
            },
            &SessionEvent::ApiErrorText {
                text: "Not logged in · Please run /login".into()
            },
        ]
    );
    assert!(
        !events.iter().any(|e| matches!(e,
            SessionEvent::AssistantText { text, .. } if text.starts_with("Credit balance"))),
        "{events:#?}"
    );
}
