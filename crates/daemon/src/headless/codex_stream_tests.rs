use super::*;
use crate::headless::test_support::{
    CODEX_EXEC, CODEX_ITEM_SHAPES, CODEX_MCP_APPROVE, CODEX_MCP_AUTO, CODEX_PROJECT_CONFIG,
    CODEX_RESUME, CODEX_USAGE_LIMIT, lines,
};
use crate::headless::{FailureKind, TurnOutcome};
use proto::TokenUsage;
use serde_json::json;

fn parse_all(fixture: &str) -> Vec<SessionEvent> {
    lines(fixture).into_iter().flat_map(parse_line).collect()
}

#[test]
fn codex_stream_parses_exec_and_resume() {
    for fixture in [CODEX_EXEC, CODEX_RESUME, CODEX_PROJECT_CONFIG] {
        for line in lines(fixture) {
            let events = parse_line(line);
            assert!(!events.is_empty(), "no event for {line}");
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, SessionEvent::Unknown { .. })),
                "{line}"
            );
        }
    }

    let exec = parse_all(CODEX_EXEC);
    let command =
        "/bin/zsh -lc \"printf 'one\\\\n' > a.txt\ngit add a.txt && git commit -m \\\"add a\\\"\"";
    let first_run = [
        SessionEvent::Init {
            session_id: "00000000-0000-4000-8000-000000000157".into(),
            model: None,
            mcp_ok: None,
        },
        SessionEvent::TurnStarted,
        SessionEvent::Other {
            kind: "item/error".into(),
        },
        SessionEvent::AssistantText {
            text: "I’ll create a.txt and commit it as requested.\n".into(),
            parent: None,
        },
        SessionEvent::ToolUse {
            id: "item_2".into(),
            name: "Bash".into(),
            input: json!({"command": command}),
            parent: None,
        },
        SessionEvent::ToolResult {
            id: "item_2".into(),
            text: "[task-b 9d0ec2a] add a\n 1 file changed, 1 insertion(+)\n create mode 100644 a.txt\n"
                .into(),
            ok: true,
            parent: None,
        },
        SessionEvent::AssistantText {
            text: "done".into(),
            parent: None,
        },
        // `input_tokens` counts the cached ones too (Codex's `cached_input_tokens` is a
        // part of it), so `input` is the uncached remainder: 41170 - 28416.
        SessionEvent::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: Some(TokenUsage {
                input: 41170 - 28416,
                output: 73,
                cache_read: 28416,
                cache_write: 0,
            }),
            denials: vec![],
        },
    ];
    assert_eq!(exec[..first_run.len()], first_run);

    // The `-m no-such-model` run: the notice items and the error line are inert; the
    // turn fails with the API's own message, not a rate limit.
    let failed = &exec[first_run.len()..];
    assert_eq!(
        failed[0],
        SessionEvent::Init {
            session_id: "00000000-0000-4000-8000-000000000158".into(),
            model: None,
            mcp_ok: None,
        }
    );
    // The top-level `error` line is a diagnostic the driver logs (M8a.7 fix round 1,
    // M9): the parser returns it rather than logging.
    assert!(failed.contains(&SessionEvent::Diagnostic {
        text: "{\"type\":\"error\",\"status\":400,\"error\":{\"type\":\"invalid_request_error\",\"message\":\"The 'no-such-model' model is not supported when using Codex with a ChatGPT account.\"}}".into()
    }));
    assert_eq!(
        failed.last(),
        Some(&SessionEvent::TurnEnded {
            outcome: TurnOutcome::Failed {
                error: "The 'no-such-model' model is not supported when using Codex with a ChatGPT account."
                    .into(),
                kind: FailureKind::Other,
            },
            usage: None,
            denials: vec![],
        })
    );

    // Resume repeats the thread id, and its two commands pair up by id.
    let resume = parse_all(CODEX_RESUME);
    assert_eq!(
        resume[0],
        SessionEvent::Init {
            session_id: "00000000-0000-4000-8000-000000000157".into(),
            model: None,
            mcp_ok: None,
        }
    );
    let tools: Vec<(String, &str)> = resume
        .iter()
        .filter_map(|e| match e {
            SessionEvent::ToolUse { id, .. } => Some((format!("use {id}"), "")),
            SessionEvent::ToolResult { id, ok, .. } => {
                Some((format!("result {id}"), if *ok { "ok" } else { "failed" }))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        tools,
        [
            ("use item_2".to_string(), ""),
            ("result item_2".to_string(), "ok"),
            ("use item_3".to_string(), ""),
            ("result item_3".to_string(), "ok"),
        ]
    );
    assert!(matches!(
        resume.last(),
        Some(SessionEvent::TurnEnded {
            usage: Some(TokenUsage {
                input: 19230,
                output: 201,
                cache_read: 75136,
                cache_write: 0
            }),
            ..
        })
    ));
}

#[test]
fn documented_item_types_map_as_the_table_says() {
    // Not in M8a.1's recordings (none was triggered); the working shapes of the table.
    let mcp_started = r#"{"type":"item.started","item":{"id":"item_5","type":"mcp_tool_call","server":"anthrex","tool":"task_done","arguments":{"summary":"did it"},"status":"in_progress"}}"#;
    assert_eq!(
        parse_line(mcp_started),
        [SessionEvent::ToolUse {
            id: "item_5".into(),
            name: "mcp__anthrex__task_done".into(),
            input: json!({"summary": "did it"}),
            parent: None,
        }]
    );
    let mcp_done = r#"{"type":"item.completed","item":{"id":"item_5","type":"mcp_tool_call","server":"anthrex","tool":"task_done","arguments":{},"result":{"content":[{"type":"text","text":"accepted"},{"type":"text","text":"queued for check"}]},"status":"completed"}}"#;
    assert_eq!(
        parse_line(mcp_done),
        [SessionEvent::ToolResult {
            id: "item_5".into(),
            text: "accepted\nqueued for check".into(),
            ok: true,
            parent: None,
        }]
    );
    let mcp_failed = r#"{"type":"item.completed","item":{"id":"item_6","type":"mcp_tool_call","server":"anthrex","tool":"task_done","arguments":{},"error":{"message":"not a worker"},"status":"failed"}}"#;
    assert_eq!(
        parse_line(mcp_failed),
        [SessionEvent::ToolResult {
            id: "item_6".into(),
            text: "not a worker".into(),
            ok: false,
            parent: None,
        }]
    );

    let patch = r#"{"type":"item.completed","item":{"id":"item_7","type":"file_change","changes":[{"path":"a.txt","kind":"add"}],"status":"completed"}}"#;
    assert_eq!(
        parse_line(patch),
        [
            SessionEvent::ToolUse {
                id: "item_7".into(),
                name: "apply_patch".into(),
                input: json!({"changes": [{"path": "a.txt", "kind": "add"}]}),
                parent: None,
            },
            SessionEvent::ToolResult {
                id: "item_7".into(),
                text: String::new(),
                ok: true,
                parent: None,
            },
        ]
    );
    let rejected = patch
        .replace("item_7", "item_8")
        .replace("\"status\":\"completed\"", "\"status\":\"failed\"");
    assert!(matches!(
        parse_line(&rejected).as_slice(),
        [
            SessionEvent::ToolUse { .. },
            SessionEvent::ToolResult { ok: false, .. }
        ]
    ));
    // A file change reports only on completion; its start is inert.
    assert!(matches!(
        parse_line(&patch.replace("item.completed", "item.started")).as_slice(),
        [SessionEvent::Other { .. }]
    ));

    let search = r#"{"type":"item.completed","item":{"id":"item_9","type":"web_search","query":"rust toml"}}"#;
    assert!(matches!(
        parse_line(search).as_slice(),
        [SessionEvent::ToolUse { name, id, .. }, SessionEvent::ToolResult { ok: true, .. }]
            if name == "WebSearch" && id == "item_9"
    ));
    for inert in ["reasoning", "todo_list"] {
        let line = format!(
            r#"{{"type":"item.completed","item":{{"id":"item_1","type":"{inert}","text":"x"}}}}"#
        );
        assert_eq!(
            parse_line(&line),
            [SessionEvent::Other {
                kind: format!("item/{inert}")
            }]
        );
    }
    // A failed command (should one ever be reported) is a failed result.
    let failed = r#"{"type":"item.completed","item":{"id":"item_4","type":"command_execution","command":"false","aggregated_output":"nope","exit_code":1,"status":"failed"}}"#;
    assert!(matches!(
        parse_line(failed).as_slice(),
        [SessionEvent::ToolResult { ok: false, text, .. }] if text == "nope"
    ));
}

#[test]
fn a_rate_limited_turn_failure_is_classified() {
    let failed = |message: &str| {
        let line = json!({"type": "turn.failed", "error": {"message": message}}).to_string();
        match parse_line(&line).as_slice() {
            [
                SessionEvent::TurnEnded {
                    outcome: TurnOutcome::Failed { kind, error },
                    ..
                },
            ] => (*kind, error.clone()),
            other => panic!("{other:?}"),
        }
    };
    for message in [
        "Usage limit reached. You've reached your usage limit.",
        "exceeded retry limit, last status: 429 Too Many Requests",
        "Rate Limit exceeded",
        "too many requests",
    ] {
        assert_eq!(failed(message), (FailureKind::RateLimit, message.into()));
    }
    // A JSON error string is classified by its `status` and reported by its message.
    let json_error = r#"{"type":"error","status":429,"error":{"type":"x","message":"slow down"}}"#;
    assert_eq!(
        failed(json_error),
        (FailureKind::RateLimit, "slow down".into())
    );
    assert_eq!(
        failed("stream disconnected"),
        (FailureKind::Other, "stream disconnected".into())
    );
}

#[test]
fn a_command_result_is_cut_on_a_char_boundary() {
    let line = json!({"type": "item.completed", "item": {
        "id": "item_2", "type": "command_execution", "command": "cat big",
        "aggregated_output": "世".repeat(3000), "exit_code": 0, "status": "completed"
    }})
    .to_string();
    match parse_line(&line).as_slice() {
        [SessionEvent::ToolResult { text, .. }] => {
            assert_eq!(text.len(), 4095);
            assert_eq!(text.chars().count(), 1365);
        }
        other => panic!("{other:?}"),
    }
}

/// Every line of the 0.156.1 captures parses to at least one event, none `Unknown`.
#[test]
fn codex_0_156_captures_parse_every_line() {
    for fixture in [
        CODEX_ITEM_SHAPES,
        CODEX_MCP_AUTO,
        CODEX_MCP_APPROVE,
        CODEX_USAGE_LIMIT,
    ] {
        for line in lines(fixture) {
            let events = parse_line(line);
            assert!(!events.is_empty(), "no event for {line}");
            assert!(
                !events
                    .iter()
                    .any(|e| matches!(e, SessionEvent::Unknown { .. })),
                "{line}"
            );
        }
    }
}

#[test]
fn recorded_item_shapes_map_as_the_table_says() {
    let events = parse_all(CODEX_ITEM_SHAPES);
    let denied = "MCP tool call requires approval, but approval policy is never";
    // A failed MCP call: its use, then a failed result carrying the error message.
    let at = events
        .iter()
        .position(|e| matches!(e, SessionEvent::ToolUse { id, .. } if id == "item_1"))
        .unwrap();
    assert_eq!(
        events[at..at + 2],
        [
            SessionEvent::ToolUse {
                id: "item_1".into(),
                name: "mcp__probe__echo_probe".into(),
                input: json!({"text": "hello"}),
                parent: None,
            },
            SessionEvent::ToolResult {
                id: "item_1".into(),
                text: denied.into(),
                ok: false,
                parent: None,
            },
        ]
    );
    // A file change: its absolute path and kind, reported on completion.
    assert!(events.contains(&SessionEvent::ToolUse {
        id: "item_2".into(),
        name: "apply_patch".into(),
        input: json!({"changes": [{"path": "/tmp/anthrex-fixture/repo/notes.txt", "kind": "add"}]}),
        parent: None,
    }));
    assert!(events.contains(&SessionEvent::ToolResult {
        id: "item_2".into(),
        text: String::new(),
        ok: true,
        parent: None,
    }));
    // A web search whose item names `id` twice: the line parses (the last key wins),
    // and the completed search is one use and one result with its query.
    let search_id = "exec-2bd9810d-6dbe-47ed-b77a-d1da61028683";
    // The completed line's own query (read from the fixture: the search was about an
    // async runtime whose name decision 2's purity grep forbids in these files).
    let completed = lines(CODEX_ITEM_SHAPES)
        .into_iter()
        .find(|l| l.contains("web_search") && l.contains("item.completed"))
        .unwrap();
    let query: serde_json::Value = serde_json::from_str(completed).unwrap();
    let query = query["item"]["query"].as_str().unwrap().to_owned();
    assert!(query.ends_with(" broadcast Lagged"), "{query}");
    let search: Vec<&SessionEvent> = events
        .iter()
        .filter(|e| match e {
            SessionEvent::ToolUse { id, .. } | SessionEvent::ToolResult { id, .. } => {
                id == search_id
            }
            _ => false,
        })
        .collect();
    assert_eq!(
        search,
        [
            &SessionEvent::ToolUse {
                id: search_id.into(),
                name: "WebSearch".into(),
                input: json!({ "query": query }),
                parent: None,
            },
            &SessionEvent::ToolResult {
                id: search_id.into(),
                text: String::new(),
                ok: true,
                parent: None,
            },
        ]
    );
    assert_eq!(
        events.last(),
        Some(&SessionEvent::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: Some(TokenUsage {
                input: 88309 - 78720,
                output: 282,
                cache_read: 78720,
                cache_write: 0,
            }),
            denials: vec![],
        })
    );

    // "auto" under "never" fails the call; "approve" completes it with its text.
    assert!(
        parse_all(CODEX_MCP_AUTO).contains(&SessionEvent::ToolResult {
            id: "item_0".into(),
            text: denied.into(),
            ok: false,
            parent: None,
        })
    );
    assert!(
        parse_all(CODEX_MCP_APPROVE).contains(&SessionEvent::ToolResult {
            id: "item_0".into(),
            text: "probe says: hi".into(),
            ok: true,
            parent: None,
        })
    );
}

#[test]
fn a_usage_limit_is_a_rate_limit() {
    let events = parse_all(CODEX_USAGE_LIMIT);
    let text = "You’ve hit your usage limit. Visit https://chatgpt.com/codex/settings/usage to purchase more credits or try again at Sep 25th, 2026 11:33 AM.";
    assert!(events.contains(&SessionEvent::Diagnostic { text: text.into() }));
    assert_eq!(
        events.last(),
        Some(&SessionEvent::TurnEnded {
            outcome: TurnOutcome::Failed {
                error: text.into(),
                kind: FailureKind::RateLimit,
            },
            usage: None,
            denials: vec![],
        })
    );
}

#[test]
fn a_429_inside_an_id_or_a_count_is_not_a_rate_limit() {
    let kind = |message: &str| {
        let line = json!({"type": "turn.failed", "error": {"message": message}}).to_string();
        match parse_line(&line).as_slice() {
            [
                SessionEvent::TurnEnded {
                    outcome: TurnOutcome::Failed { kind, .. },
                    ..
                },
            ] => *kind,
            other => panic!("{other:?}"),
        }
    };
    for other in [
        "request req_8429 failed: bad model",
        "you requested 142913 tokens, the maximum is 128000",
        "request 0f4291a2-4290-4429-a429-94290b4290ab failed",
        "error 4290",
        "HTTP429",
        "http_429",
    ] {
        assert_eq!(kind(other), FailureKind::Other, "{other}");
    }
    for limited in ["HTTP 429", "status 429.", "429 Too Many", "(429)"] {
        assert_eq!(kind(limited), FailureKind::RateLimit, "{limited}");
    }
    // The JSON `status` counts even when the text has no 429.
    let json_status = r#"{"status":429,"error":{"message":"slow"}}"#;
    assert_eq!(kind(json_status), FailureKind::RateLimit);
}

#[test]
fn a_failed_mcp_call_is_failed_even_when_its_status_says_completed() {
    let line = r#"{"type":"item.completed","item":{"id":"item_6","type":"mcp_tool_call","server":"anthrex","tool":"task_done","arguments":{},"result":null,"error":{"message":"bad args"},"status":"completed"}}"#;
    assert_eq!(
        parse_line(line),
        [SessionEvent::ToolResult {
            id: "item_6".into(),
            text: "bad args".into(),
            ok: false,
            parent: None,
        }]
    );
}

#[test]
fn codex_usage_counts_cache_writes_inside_input_and_never_goes_negative() {
    // Decision 40: Codex billable is input - cached + output. `cache_write_input_tokens`
    // is part of `input_tokens` like the cached reads, so it is not counted again.
    let usage =
        |body: &str| match parse_line(&format!(r#"{{"type":"turn.completed","usage":{body}}}"#))
            .as_slice()
        {
            [SessionEvent::TurnEnded { usage, .. }] => usage.unwrap(),
            other => panic!("{other:?}"),
        };
    assert_eq!(
        usage(
            r#"{"input_tokens":900,"cached_input_tokens":300,"cache_write_input_tokens":70,"output_tokens":41,"reasoning_output_tokens":11}"#
        ),
        TokenUsage {
            input: 600,
            output: 41,
            cache_read: 300,
            cache_write: 0,
        }
    );
    // More cached than input (never seen, but a wrap would bill 1.8e19 tokens).
    assert_eq!(
        usage(r#"{"input_tokens":5,"cached_input_tokens":9,"output_tokens":2}"#),
        TokenUsage {
            input: 0,
            output: 2,
            cache_read: 9,
            cache_write: 0,
        }
    );
}
