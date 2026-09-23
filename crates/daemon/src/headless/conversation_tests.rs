use super::*;
use crate::headless::{FailureKind, TurnOutcome};
use crate::hooks::HookKind;
use proto::HookSource;
use serde_json::{Value, json};

/// A synthesised hook as the table says it looks: source `Stream`, no session source.
fn stream_hook(kind: HookKind, session_id: Option<&str>) -> ParsedHook {
    ParsedHook {
        source: HookSource::Stream,
        kind,
        session_id: session_id.map(str::to_owned),
        agent_id: None,
        agent_type: None,
        tool_name: None,
        tool_input: None,
        notification_type: None,
        transcript_path: None,
        tool_use_id: None,
        tool_response: None,
        tool_result_truncated: None,
        tool_result_stringified: None,
        prompt: None,
        session_source: None,
    }
}

fn init(session: &str) -> SessionEvent {
    SessionEvent::Init {
        session_id: session.into(),
        model: Some("m".into()),
        mcp_ok: Some(true),
    }
}

/// A cursor that has seen `Init { s-7 }` and sent two turns, so the latest sent turn's
/// ordinal is 1 (not 0, not the count 2).
fn primed(runtime: Runtime, hooks_fire: bool) -> StreamCursor {
    let mut cursor = StreamCursor::default();
    map(runtime, hooks_fire, &init("s-7"), &mut cursor);
    sent_turn(runtime, hooks_fire, "first", &mut cursor);
    map(
        runtime,
        hooks_fire,
        &SessionEvent::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: None,
            denials: vec![],
        },
        &mut cursor,
    );
    sent_turn(runtime, hooks_fire, "second", &mut cursor);
    cursor
}

/// A tool id as the mapping gives it: Codex ids are namespaced by the latest sent turn's
/// ordinal (1 after `primed`), because every Codex process numbers its items from 0.
fn tool_id(runtime: Runtime, id: &str) -> String {
    match runtime {
        Runtime::Codex => format!("t1:{id}"),
        _ => id.to_string(),
    }
}

/// Runs one table row for both `hooks_fire` values: the records are the same either
/// way, and hooks appear only when the runtime's own do not fire.
fn row(
    runtime: Runtime,
    setup: &[SessionEvent],
    event: Option<&SessionEvent>,
    records: Vec<Record>,
    hooks: Vec<ParsedHook>,
) {
    for hooks_fire in [false, true] {
        let mut cursor = primed(runtime, hooks_fire);
        for e in setup {
            map(runtime, hooks_fire, e, &mut cursor);
        }
        let input = match event {
            Some(e) => map(runtime, hooks_fire, e, &mut cursor),
            None => sent_turn(runtime, hooks_fire, "third", &mut cursor),
        };
        assert_eq!(input.records, records, "{event:?}, hooks_fire {hooks_fire}");
        let expected = if hooks_fire { vec![] } else { hooks.clone() };
        assert_eq!(input.hooks, expected, "{event:?}, hooks_fire {hooks_fire}");
    }
}

#[test]
fn conversation_map_follows_the_table() {
    for runtime in [Runtime::Codex, Runtime::Claude] {
        // The daemon starts a turn with text T: ordinal 2, after two sent turns.
        row(
            runtime,
            &[],
            None,
            vec![Record::UserText {
                session_id: Some("s-7".into()),
                ordinal: 2,
                text: "third".into(),
                human: true,
            }],
            vec![ParsedHook {
                prompt: Some("third".into()),
                ..stream_hook(HookKind::UserPromptSubmit, Some("s-7"))
            }],
        );
        // Init: a session start, and the session id every later hook and record carries.
        row(
            runtime,
            &[],
            Some(&init("s-8")),
            vec![],
            vec![stream_hook(HookKind::SessionStart, Some("s-8"))],
        );
        // Claude repeats `Init` every turn and Codex every process: the same session
        // again starts nothing.
        row(runtime, &[], Some(&init("s-7")), vec![], vec![]);
        // Top-level prose lands on the latest sent turn.
        row(
            runtime,
            &[],
            Some(&SessionEvent::AssistantText {
                text: "on it".into(),
                parent: None,
            }),
            vec![Record::AssistantText {
                session_id: Some("s-7".into()),
                ordinal: 1,
                text: "on it".into(),
            }],
            vec![],
        );
        // A top-level tool use.
        let tool_use = SessionEvent::ToolUse {
            id: "tu-1".into(),
            name: "Bash".into(),
            input: json!({"command": "ls"}),
            parent: None,
        };
        row(
            runtime,
            &[],
            Some(&tool_use),
            vec![Record::ToolDetail {
                tool_use_id: tool_id(runtime, "tu-1"),
                input: Some(json!({"command": "ls"})),
                detail: None,
                ok: None,
            }],
            vec![ParsedHook {
                tool_name: Some("Bash".into()),
                tool_input: Some(json!({"command": "ls"})),
                tool_use_id: Some(tool_id(runtime, "tu-1")),
                ..stream_hook(HookKind::PreToolUse, Some("s-7"))
            }],
        );
        // Its result, succeeded and failed: the name comes from the cursor.
        for (ok, key) in [(true, "output"), (false, "error")] {
            row(
                runtime,
                std::slice::from_ref(&tool_use),
                Some(&SessionEvent::ToolResult {
                    id: "tu-1".into(),
                    text: "a.txt".into(),
                    ok,
                    parent: None,
                }),
                vec![Record::ToolDetail {
                    tool_use_id: tool_id(runtime, "tu-1"),
                    input: None,
                    detail: Some("a.txt".into()),
                    ok: Some(ok),
                }],
                vec![ParsedHook {
                    tool_name: Some("Bash".into()),
                    tool_use_id: Some(tool_id(runtime, "tu-1")),
                    tool_response: Some(json!({ key: "a.txt" })),
                    tool_result_truncated: Some(false),
                    tool_result_stringified: Some(false),
                    ..stream_hook(HookKind::PostToolUse, Some("s-7"))
                }],
            );
        }
        // A sub-agent's tool use and result join by id, with no hook.
        row(
            runtime,
            &[],
            Some(&SessionEvent::ToolUse {
                id: "tu-2".into(),
                name: "Read".into(),
                input: json!({"file_path": "a"}),
                parent: Some("tu-agent".into()),
            }),
            vec![Record::ToolDetail {
                tool_use_id: tool_id(runtime, "tu-2"),
                input: Some(json!({"file_path": "a"})),
                detail: None,
                ok: None,
            }],
            vec![],
        );
        row(
            runtime,
            &[],
            Some(&SessionEvent::ToolResult {
                id: "tu-2".into(),
                text: "contents".into(),
                ok: false,
                parent: Some("tu-agent".into()),
            }),
            vec![Record::ToolDetail {
                tool_use_id: tool_id(runtime, "tu-2"),
                input: None,
                detail: Some("contents".into()),
                ok: Some(false),
            }],
            vec![],
        );
        // A sub-agent's prose: nothing.
        row(
            runtime,
            &[],
            Some(&SessionEvent::AssistantText {
                text: "sub".into(),
                parent: Some("tu-agent".into()),
            }),
            vec![],
            vec![],
        );
        // A turn's end, whatever its outcome.
        for outcome in [
            TurnOutcome::Completed,
            TurnOutcome::Interrupted,
            TurnOutcome::Failed {
                error: "x".into(),
                kind: FailureKind::Other,
            },
        ] {
            row(
                runtime,
                &[],
                Some(&SessionEvent::TurnEnded {
                    outcome,
                    usage: None,
                    denials: vec![],
                }),
                vec![],
                vec![stream_hook(HookKind::Stop, Some("s-7"))],
            );
        }
        // Anything else.
        for other in [
            SessionEvent::TurnStarted,
            SessionEvent::UserText {
                text: "echo".into(),
            },
            SessionEvent::ApiRetry {
                error: "rate_limit".into(),
                attempt: 1,
                delay_ms: 5,
            },
            SessionEvent::PermissionDenied {
                tool: "Write".into(),
                reason: "no".into(),
            },
            SessionEvent::Compacted,
            SessionEvent::Other {
                kind: "thinking".into(),
            },
            SessionEvent::Unknown { line: "?".into() },
            SessionEvent::StderrLine { line: "e".into() },
            SessionEvent::ProcessExited {
                code: Some(0),
                signal: None,
            },
        ] {
            row(runtime, &[], Some(&other), vec![], vec![]);
        }
    }
}

#[test]
fn prose_of_a_turn_the_daemon_did_not_send_is_not_put_on_the_last_one() {
    // M8a.1: a background sub-agent finishing makes Claude Code start a turn by itself.
    // Its prose must not be appended to the last turn the daemon sent.
    let mut cursor = StreamCursor::default();
    let text = |t: &str| SessionEvent::AssistantText {
        text: t.into(),
        parent: None,
    };
    let end = SessionEvent::TurnEnded {
        outcome: TurnOutcome::Completed,
        usage: None,
        denials: vec![],
    };
    map(Runtime::Claude, true, &init("s-1"), &mut cursor);
    sent_turn(Runtime::Claude, true, "go", &mut cursor);
    assert_eq!(
        map(Runtime::Claude, true, &text("mine"), &mut cursor)
            .records
            .len(),
        1
    );
    map(Runtime::Claude, true, &end, &mut cursor);
    map(Runtime::Claude, true, &init("s-1"), &mut cursor);
    assert_eq!(
        map(Runtime::Claude, true, &text("unprompted"), &mut cursor),
        ConversationInput::default()
    );
    // Before any turn was sent there is no turn to land on either.
    let mut fresh = StreamCursor::default();
    assert_eq!(
        map(Runtime::Codex, false, &text("early"), &mut fresh),
        ConversationInput::default()
    );
}

#[test]
fn only_an_init_after_a_turn_with_hooks_firing_counts_as_an_unprompted_turn() {
    let ordinal_of_next_sent = |runtime: Runtime, hooks_fire: bool, events: &[SessionEvent]| {
        let mut cursor = StreamCursor::default();
        for e in events {
            map(runtime, hooks_fire, e, &mut cursor);
        }
        match sent_turn(runtime, hooks_fire, "next", &mut cursor)
            .records
            .as_slice()
        {
            [Record::UserText { ordinal, .. }] => *ordinal,
            other => panic!("{other:?}"),
        }
    };
    let end = SessionEvent::TurnEnded {
        outcome: TurnOutcome::Completed,
        usage: None,
        denials: vec![],
    };
    // The session's first `Init`, before any turn: not an unprompted turn.
    assert_eq!(ordinal_of_next_sent(Runtime::Claude, true, &[init("s")]), 0);
    // After a sent turn ends, an `Init` with no turn sent is one, counted once.
    let mut cursor = StreamCursor::default();
    sent_turn(Runtime::Claude, true, "go", &mut cursor);
    for e in [init("s"), end.clone(), init("s"), init("s")] {
        map(Runtime::Claude, true, &e, &mut cursor);
    }
    match sent_turn(Runtime::Claude, true, "next", &mut cursor)
        .records
        .as_slice()
    {
        [Record::UserText { ordinal, .. }] => assert_eq!(*ordinal, 2),
        other => panic!("{other:?}"),
    }
    // Without hooks nothing builds that turn, so nothing is counted.
    let codex = [init("s"), end.clone(), init("s")];
    assert_eq!(ordinal_of_next_sent(Runtime::Codex, false, &codex), 0);
}

#[test]
fn synthesised_tool_responses_are_bounded_like_the_hooks() {
    let line = json!({"type": "item.completed", "item": {
        "id": "item_2", "type": "command_execution", "command": "cat big",
        "aggregated_output": "世".repeat(3000), "exit_code": 0, "status": "completed"
    }})
    .to_string();
    let result = crate::headless::codex_stream::parse_line(&line).remove(0);
    let SessionEvent::ToolResult { text, .. } = &result else {
        panic!("{result:?}");
    };
    let (response, truncated, stringified) =
        proto::conversation::bound_tool_response_value(json!({ "output": text }));

    let mut cursor = StreamCursor::default();
    sent_turn(Runtime::Codex, false, "cat it", &mut cursor);
    map(
        Runtime::Codex,
        false,
        &SessionEvent::ToolUse {
            id: "item_2".into(),
            name: "Bash".into(),
            input: json!({"command": "cat big"}),
            parent: None,
        },
        &mut cursor,
    );
    let hooks = map(Runtime::Codex, false, &result, &mut cursor).hooks;
    let [hook] = hooks.as_slice() else {
        panic!("{hooks:?}");
    };
    assert_eq!(hook.kind, HookKind::PostToolUse);
    assert_eq!(hook.tool_response.as_ref(), Some(&response));
    assert_eq!(hook.tool_result_truncated, Some(truncated));
    assert_eq!(hook.tool_result_stringified, Some(stringified));
    // The bound did bite: the event's 4095 bytes plus the object's keys exceed 4 KiB.
    assert!(truncated);
    let encoded = serde_json::to_string(hook.tool_response.as_ref().unwrap()).unwrap();
    assert!(encoded.len() <= proto::conversation::TOOL_RESULT_SUMMARY_MAX);
    assert!(matches!(response.get("output"), Some(Value::String(_))));
}

#[path = "conversation_session_tests.rs"]
mod session;

#[path = "conversation_content_tests.rs"]
mod content;
