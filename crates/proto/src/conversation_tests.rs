//! `conversation`'s own unit tests, split out under the repo's
//! `#[path = "..._tests.rs"]` convention (see `AGENTS.md`).

use super::*;
use crate::messages::{ClientMsg, DaemonMsg};
use crate::types::Runtime;
use serde_json::json;

/// Builds the `Conversation` the task brief specifies for
/// `every_conversation_message_round_trips`: every same-typed field pair (the two turns'
/// ids, roles, timestamps, states) carries distinct values so a transposition between
/// them would fail this test.
fn distinct_conversation() -> Conversation {
    Conversation {
        window_id: 7,
        agent_id: Some("agent-3".into()),
        session_id: Some("sess-9".into()),
        runtime: Runtime::Claude,
        rev: 214,
        degraded: Some(DegradeReason::BadRecord),
        dropped_turns: 4,
        dropped_by: Some(DropCause::Turns),
        turns: vec![
            Turn {
                id: 11,
                role: Role::User,
                at_unix_secs: 1_789_123_456,
                state: TurnState::Complete,
                blocks: vec![Block::Text {
                    text: "first turn".into(),
                }],
            },
            Turn {
                id: 12,
                role: Role::Assistant,
                at_unix_secs: 1_789_123_461,
                state: TurnState::Running,
                blocks: vec![Block::Text {
                    text: "second turn".into(),
                }],
            },
        ],
    }
}

fn round_trips<T>(value: &T)
where
    T: serde::Serialize + serde::de::DeserializeOwned + PartialEq + std::fmt::Debug,
{
    let packed = rmp_serde::to_vec_named(value).unwrap();
    let back: T = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(&back, value);
}

#[test]
fn every_conversation_message_round_trips() {
    let conversation = distinct_conversation();

    let subscribe = ClientMsg::SubscribeConversation {
        window_id: 7,
        agent_id: Some("agent-3".into()),
        from_rev: Some(214),
    };
    round_trips(&subscribe);

    let unsubscribe = ClientMsg::UnsubscribeConversation {
        window_id: 7,
        agent_id: Some("agent-3".into()),
    };
    round_trips(&unsubscribe);

    let snapshot = DaemonMsg::ConversationSnapshot {
        window_id: 7,
        agent_id: Some("agent-3".into()),
        conversation: conversation.clone(),
    };
    round_trips(&snapshot);

    let delta = DaemonMsg::ConversationDelta {
        window_id: 7,
        agent_id: Some("agent-3".into()),
        from_rev: 200,
        to_rev: 214,
        turns: vec![
            TurnPatch::Upsert(conversation.turns[0].clone()),
            TurnPatch::Drop { id: 12 },
        ],
        degraded: Some(DegradeReason::BadRecord),
        dropped_turns: 4,
        dropped_by: Some(DropCause::Turns),
    };
    round_trips(&delta);

    let gone = DaemonMsg::ConversationGone {
        window_id: 7,
        agent_id: Some("agent-3".into()),
        reason: GONE_WINDOW_REMOVED.into(),
    };
    round_trips(&gone);
}

#[test]
fn every_block_variant_round_trips() {
    let turn = Turn {
        id: 1,
        role: Role::Assistant,
        at_unix_secs: 1_700_000_000,
        state: TurnState::Complete,
        blocks: vec![
            Block::Text {
                text: "mapping the call sites".into(),
            },
            Block::ToolCall {
                id: Some("tu-1".into()),
                name: "Grep".into(),
                summary: "\"parse_\" — 34 matches".into(),
                input: Some(json!({"pattern": "parse_"})),
                result: Some(ToolResult {
                    ok: true,
                    summary: "34 matches".into(),
                    detail: Some("crates/parse/src/lib.rs:12".into()),
                    truncated: false,
                }),
                state: ToolState::Ok,
                duration_ms: Some(312),
            },
            Block::SubagentSpawn {
                agent_id: "agent-8".into(),
                kind: "Explore".into(),
                label: "find every call site".into(),
                model: Some("haiku".into()),
            },
            Block::Notice {
                kind: NoticeKind::PermissionRequest,
                text: "Bash needs permission".into(),
            },
        ],
    };
    let conversation = Conversation {
        window_id: 1,
        agent_id: None,
        session_id: Some("sess-1".into()),
        runtime: Runtime::Claude,
        rev: 1,
        degraded: None,
        dropped_turns: 0,
        dropped_by: None,
        turns: vec![turn],
    };
    let snapshot = DaemonMsg::ConversationSnapshot {
        window_id: 1,
        agent_id: None,
        conversation,
    };
    round_trips(&snapshot);
}

#[test]
fn serde_names_are_stable() {
    assert_eq!(serde_json::to_value(Role::User).unwrap(), json!("user"));
    assert_eq!(
        serde_json::to_value(Role::Assistant).unwrap(),
        json!("assistant")
    );
    assert_eq!(serde_json::to_value(Role::System).unwrap(), json!("system"));

    assert_eq!(
        serde_json::to_value(TurnState::Running).unwrap(),
        json!("running")
    );
    assert_eq!(
        serde_json::to_value(TurnState::Complete).unwrap(),
        json!("complete")
    );

    assert_eq!(
        serde_json::to_value(ToolState::Pending).unwrap(),
        json!("pending")
    );
    assert_eq!(serde_json::to_value(ToolState::Ok).unwrap(), json!("ok"));
    assert_eq!(
        serde_json::to_value(ToolState::Failed).unwrap(),
        json!("failed")
    );
    assert_eq!(
        serde_json::to_value(ToolState::Denied).unwrap(),
        json!("denied")
    );

    assert_eq!(
        serde_json::to_value(NoticeKind::PermissionRequest).unwrap(),
        json!("permission_request")
    );
    assert_eq!(
        serde_json::to_value(NoticeKind::Error).unwrap(),
        json!("error")
    );
    assert_eq!(
        serde_json::to_value(NoticeKind::Compaction).unwrap(),
        json!("compaction")
    );

    assert_eq!(
        serde_json::to_value(DropCause::Turns).unwrap(),
        json!("turns")
    );
    assert_eq!(
        serde_json::to_value(DropCause::Bytes).unwrap(),
        json!("bytes")
    );

    assert_eq!(
        serde_json::to_value(DegradeReason::NoTranscriptPath).unwrap(),
        json!("no_transcript_path")
    );
    assert_eq!(
        serde_json::to_value(DegradeReason::Unreadable).unwrap(),
        json!("unreadable")
    );
    assert_eq!(
        serde_json::to_value(DegradeReason::UnknownFormat).unwrap(),
        json!("unknown_format")
    );
    assert_eq!(
        serde_json::to_value(DegradeReason::TooLarge).unwrap(),
        json!("too_large")
    );
    assert_eq!(
        serde_json::to_value(DegradeReason::BadRecord).unwrap(),
        json!("bad_record")
    );
    assert_eq!(
        serde_json::to_value(DegradeReason::Misaligned).unwrap(),
        json!("misaligned")
    );
}

/// Task M6.5.8: `Misaligned` crosses the wire as itself. A round trip alone pins only
/// the structure (a codec that decoded every reason as `BadRecord` would still compare
/// equal to a `BadRecord` it encoded), so the decoded value is also asserted by name,
/// field by field, against `Misaligned` specifically.
#[test]
fn misaligned_round_trips_by_name() {
    let conversation = Conversation {
        degraded: Some(DegradeReason::Misaligned),
        ..distinct_conversation()
    };
    let packed = rmp_serde::to_vec_named(&conversation).unwrap();
    let back: Conversation = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(back.degraded, Some(DegradeReason::Misaligned));
    assert_ne!(back.degraded, Some(DegradeReason::BadRecord));
    assert_eq!(back, conversation);
    assert_eq!(
        DegradeReason::Misaligned.message(),
        "the transcript's prompts do not line up with the hook timeline — timeline only"
    );
}

#[test]
fn a_snapshots_envelope_matches_its_payload() {
    let mismatched_window_id = DaemonMsg::ConversationSnapshot {
        window_id: 7,
        agent_id: Some("agent-3".into()),
        conversation: Conversation {
            window_id: 8,
            ..distinct_conversation()
        },
    };
    assert!(!mismatched_window_id.conversation_envelope_is_consistent());

    // Review finding F4: the first version of this test only ever varied
    // `window_id`, so deleting the `agent_id` half of the check in
    // `conversation_envelope_is_consistent` still passed. These two cases exercise
    // `agent_id` on its own: same `window_id`, one sub-agent id against another, and
    // the window's own agent (`None`) against a sub-agent (`Some`).
    let mismatched_agent_id = DaemonMsg::ConversationSnapshot {
        window_id: 7,
        agent_id: Some("agent-3".into()),
        conversation: Conversation {
            agent_id: Some("agent-4".into()),
            ..distinct_conversation()
        },
    };
    assert!(!mismatched_agent_id.conversation_envelope_is_consistent());

    let mismatched_agent_id_none_vs_some = DaemonMsg::ConversationSnapshot {
        window_id: 7,
        agent_id: None,
        conversation: Conversation {
            agent_id: Some("agent-3".into()),
            ..distinct_conversation()
        },
    };
    assert!(!mismatched_agent_id_none_vs_some.conversation_envelope_is_consistent());

    let matched = DaemonMsg::ConversationSnapshot {
        window_id: 7,
        agent_id: Some("agent-3".into()),
        conversation: distinct_conversation(),
    };
    assert!(matched.conversation_envelope_is_consistent());

    // Every other variant has no envelope to check against and is vacuously consistent.
    assert!(
        DaemonMsg::ConversationGone {
            window_id: 7,
            agent_id: Some("agent-3".into()),
            reason: GONE_WINDOW_REMOVED.into(),
        }
        .conversation_envelope_is_consistent()
    );
}

#[test]
fn degrade_messages_are_distinct_and_end_the_same_way() {
    let reasons = [
        DegradeReason::NoTranscriptPath,
        DegradeReason::Unreadable,
        DegradeReason::UnknownFormat,
        DegradeReason::TooLarge,
        DegradeReason::BadRecord,
        DegradeReason::Misaligned,
    ];
    let messages: Vec<&str> = reasons.iter().map(|r| r.message()).collect();

    for message in &messages {
        assert!(
            message.ends_with("— timeline only"),
            "{message:?} does not end with the expected suffix"
        );
    }

    for i in 0..messages.len() {
        for j in 0..messages.len() {
            if i != j {
                assert_ne!(
                    messages[i], messages[j],
                    "DegradeReason::message() must be pairwise distinct, but {:?} and {:?} match",
                    reasons[i], reasons[j]
                );
            }
        }
    }
}

#[test]
fn byte_size_counts_structure_and_content() {
    let mut turn = Turn {
        id: 1,
        role: Role::Assistant,
        at_unix_secs: 1,
        state: TurnState::Complete,
        blocks: vec![Block::Text {
            text: "a".repeat(100),
        }],
    };
    assert_eq!(turn.byte_size(), TURN_OVERHEAD + BLOCK_OVERHEAD + 100);

    let before = turn.byte_size();
    turn.blocks.push(Block::Text {
        text: "b".repeat(50),
    });
    assert_eq!(turn.byte_size(), before + BLOCK_OVERHEAD + 50);

    let id = "tu-99".to_string();
    let name = "Grep".to_string();
    let summary = "34 matches found".to_string();
    let result_summary = "34 matches".to_string();
    let result_detail = "crates/parse/src/lib.rs:12".to_string();
    // Every string below has a distinct length, so a formula that mixed up which
    // field contributes which length would not accidentally still balance.
    assert_eq!(
        [
            id.len(),
            name.len(),
            summary.len(),
            result_summary.len(),
            result_detail.len()
        ]
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len(),
        5,
        "fixture lengths must be pairwise distinct"
    );

    let tool_turn = Turn {
        id: 2,
        role: Role::Assistant,
        at_unix_secs: 1,
        state: TurnState::Complete,
        blocks: vec![Block::ToolCall {
            id: Some(id.clone()),
            name: name.clone(),
            summary: summary.clone(),
            input: Some(json!({"a": 1})),
            result: Some(ToolResult {
                ok: true,
                summary: result_summary.clone(),
                detail: Some(result_detail.clone()),
                truncated: false,
            }),
            state: ToolState::Ok,
            duration_ms: Some(10),
        }],
    };
    // Amendment to the task-1 brief (review finding F2): `id` counts. It is
    // `tool_use_id` from the runtime's own hook payload, so its length is set by the
    // runtime, not by us — a cap that ignored it would be an under-estimate, and
    // `byte_size` feeds a cap where an under-estimate is unbounded. Measured:
    // `id: Some("x".repeat(100_000))` on an otherwise empty `ToolCall` encodes to
    // 100,129 bytes; the pre-amendment formula (excluding `id`) reported 96.
    let expected = TURN_OVERHEAD
        + BLOCK_OVERHEAD
        + id.len()
        + name.len()
        + summary.len()
        + "{\"a\":1}".len()
        + result_summary.len()
        + result_detail.len();
    assert_eq!(tool_turn.byte_size(), expected);

    // Review finding F6: `Conversation::byte_size` was called by no test — returning
    // `0` from it would have passed. Fold two turns into a `Conversation` and check the
    // total is their sum, not e.g. only the first or only the last.
    let conversation = Conversation {
        window_id: 1,
        agent_id: None,
        session_id: None,
        runtime: Runtime::Claude,
        rev: 1,
        degraded: None,
        dropped_turns: 0,
        dropped_by: None,
        turns: vec![turn.clone(), tool_turn.clone()],
    };
    assert_eq!(
        conversation.byte_size(),
        turn.byte_size() + tool_turn.byte_size()
    );
}

/// Review finding F3: `TURN_OVERHEAD`/`BLOCK_OVERHEAD` must make `byte_size` a
/// conservative *over*-estimate of what actually goes over the wire, never an
/// under-estimate — `byte_size` feeds a cap, and an under-estimate there is unbounded
/// while an over-estimate merely makes the cap bite a little early. This is the fix that
/// matters, not any particular pair of numbers: it pins the *direction* of the error for
/// every `Block` variant, present and future, rather than one measured case.
///
/// Fixture set spans empty, typical and one long-`id` case per the finding, across every
/// variant. Every fixture also uses the *longest* variant name for each small enum
/// (`Role::Assistant`, `TurnState::Complete`, `ToolState::Pending`), because those
/// variant names are wire content `byte_size` does not count towards its content sum
/// (they're bounded, unlike a runtime-supplied string) but that a real over-estimate
/// guarantee still has to clear.
#[test]
fn byte_size_never_underestimates_its_encoded_size() {
    fn assert_over_estimates(turn: &Turn, label: &str) {
        let encoded = rmp_serde::to_vec_named(turn).unwrap().len();
        assert!(
            turn.byte_size() >= encoded,
            "{label}: byte_size() = {} is an under-estimate of the encoded {encoded} bytes",
            turn.byte_size()
        );
    }

    fn turn_with(blocks: Vec<Block>) -> Turn {
        Turn {
            id: u64::MAX,
            role: Role::Assistant,
            at_unix_secs: u64::MAX,
            state: TurnState::Complete,
            blocks,
        }
    }

    // Empty: the smallest legal value of every variant.
    assert_over_estimates(
        &turn_with(vec![Block::Text {
            text: String::new(),
        }]),
        "empty text",
    );
    assert_over_estimates(
        &turn_with(vec![Block::ToolCall {
            id: None,
            name: String::new(),
            summary: String::new(),
            input: None,
            result: None,
            state: ToolState::Pending,
            duration_ms: None,
        }]),
        "empty tool call",
    );
    assert_over_estimates(
        &turn_with(vec![Block::SubagentSpawn {
            agent_id: String::new(),
            kind: String::new(),
            label: String::new(),
            model: None,
        }]),
        "empty subagent spawn",
    );
    assert_over_estimates(
        &turn_with(vec![Block::Notice {
            kind: NoticeKind::PermissionRequest,
            text: String::new(),
        }]),
        "empty notice",
    );
    assert_over_estimates(&turn_with(vec![]), "empty turn, no blocks");

    // Typical: realistic content in every field, including the nested `ToolResult`.
    assert_over_estimates(
        &turn_with(vec![Block::Text {
            text: "mapping the call sites".into(),
        }]),
        "typical text",
    );
    assert_over_estimates(
        &turn_with(vec![Block::ToolCall {
            id: Some("tu-1".into()),
            name: "Grep".into(),
            summary: "\"parse_\" — 34 matches".into(),
            input: Some(json!({"pattern": "parse_"})),
            result: Some(ToolResult {
                ok: true,
                summary: "34 matches".into(),
                detail: Some("crates/parse/src/lib.rs:12".into()),
                truncated: false,
            }),
            state: ToolState::Pending,
            duration_ms: Some(312),
        }]),
        "typical tool call",
    );
    assert_over_estimates(
        &turn_with(vec![Block::SubagentSpawn {
            agent_id: "agent-8".into(),
            kind: "Explore".into(),
            label: "find every call site".into(),
            model: Some("haiku".into()),
        }]),
        "typical subagent spawn",
    );
    assert_over_estimates(
        &turn_with(vec![Block::Notice {
            kind: NoticeKind::PermissionRequest,
            text: "Bash needs permission".into(),
        }]),
        "typical notice",
    );

    // Long id: the case the review measured directly (F2). A 100,000-byte
    // `tool_use_id` with everything else empty.
    assert_over_estimates(
        &turn_with(vec![Block::ToolCall {
            id: Some("x".repeat(100_000)),
            name: String::new(),
            summary: String::new(),
            input: None,
            result: None,
            state: ToolState::Pending,
            duration_ms: None,
        }]),
        "long id tool call",
    );

    // Worst case: every field in the biggest variant (`ToolCall` + a populated
    // `ToolResult`) present and long enough to need the largest MessagePack string
    // header simultaneously, and two such blocks in one turn — this is what the
    // constants were actually sized against.
    let big = "y".repeat(70_000);
    let maxed_tool_call = || Block::ToolCall {
        id: Some(big.clone()),
        name: big.clone(),
        summary: big.clone(),
        input: Some(json!(null)),
        result: Some(ToolResult {
            ok: true,
            summary: big.clone(),
            detail: Some(big.clone()),
            truncated: true,
        }),
        state: ToolState::Pending,
        duration_ms: Some(u32::MAX),
    };
    assert_over_estimates(&turn_with(vec![maxed_tool_call()]), "one maxed tool call");
    assert_over_estimates(
        &turn_with(vec![maxed_tool_call(), maxed_tool_call()]),
        "two maxed tool calls",
    );
}

/// Review finding F1: a `Block::byte_size` arm that returned `0` for some variant
/// passed every other test in this file, and a future variant added with `=> 0` would
/// too. The match below is exhaustive with **no `_` arm** — adding a fifth `Block`
/// variant breaks this test's own compilation until its author states its expected
/// contribution, which is the actual guarantee (not the specific numbers). Every field
/// uses a distinct-length filler so a formula that mixed up which field contributes
/// which length would not accidentally still balance.
#[test]
fn every_block_variant_contributes_its_content() {
    let text = Block::Text {
        text: "t".repeat(11),
    };
    let tool_call = Block::ToolCall {
        id: Some("i".repeat(13)),
        name: "n".repeat(17),
        summary: "s".repeat(19),
        input: Some(json!({"a": 1})), // encodes to `{"a":1}`, 7 bytes
        result: Some(ToolResult {
            ok: true,
            summary: "r".repeat(23),
            detail: Some("d".repeat(29)),
            truncated: false,
        }),
        state: ToolState::Ok,
        duration_ms: Some(5),
    };
    let subagent_spawn = Block::SubagentSpawn {
        agent_id: "a".repeat(31),
        kind: "k".repeat(37),
        label: "l".repeat(41),
        model: Some("m".repeat(43)),
    };
    let notice = Block::Notice {
        kind: NoticeKind::Error,
        text: "z".repeat(47),
    };

    for block in [&text, &tool_call, &subagent_spawn, &notice] {
        let expected_content = match block {
            Block::Text { text } => text.len(),
            Block::ToolCall {
                id,
                name,
                summary,
                input,
                result,
                state: _,
                duration_ms: _,
            } => {
                id.as_ref().map(|s| s.len()).unwrap_or(0)
                    + name.len()
                    + summary.len()
                    + input
                        .as_ref()
                        .map(|v| serde_json::to_string(v).unwrap().len())
                        .unwrap_or(0)
                    + result
                        .as_ref()
                        .map(|r| r.summary.len() + r.detail.as_ref().map(|d| d.len()).unwrap_or(0))
                        .unwrap_or(0)
            }
            Block::SubagentSpawn {
                agent_id,
                kind,
                label,
                model,
            } => {
                agent_id.len()
                    + kind.len()
                    + label.len()
                    + model.as_ref().map(|m| m.len()).unwrap_or(0)
            }
            Block::Notice { kind: _, text } => text.len(),
        };
        assert_eq!(
            block.byte_size(),
            BLOCK_OVERHEAD + expected_content,
            "{block:?}"
        );
    }
}

#[test]
fn turn_patch_round_trips() {
    let turn = Turn {
        id: 11,
        role: Role::User,
        at_unix_secs: 1_789_123_456,
        state: TurnState::Complete,
        blocks: vec![Block::Text { text: "hi".into() }],
    };

    let upsert = TurnPatch::Upsert(turn.clone());
    round_trips(&upsert);
    let upsert_json = serde_json::to_value(&upsert).unwrap();
    assert_eq!(
        upsert_json["upsert"]["id"],
        serde_json::json!(11),
        "expected an externally tagged {{\"upsert\": {{...}}}} shape, got {upsert_json}"
    );

    let drop = TurnPatch::Drop { id: 11 };
    round_trips(&drop);
    assert_eq!(
        serde_json::to_value(&drop).unwrap(),
        json!({"drop": {"id": 11}})
    );
}

/// Review finding F5: seven mutations survived the original suite, including three
/// same-typed transpositions (`agent_id`⇄`session_id`, `ToolResult.ok`⇄`truncated`,
/// `from_rev`⇄`to_rev`) that a round trip structurally cannot catch — encoding and
/// decoding a swapped-but-internally-consistent value round-trips cleanly. Pins the
/// literal wire shape instead, in the style of
/// `messages::tests::windows_changed_carries_the_project_through_messagepack`: a
/// hand-written JSON literal, decoded into the real type, round-tripped through
/// MessagePack, and re-encoded back to the same literal.
#[test]
fn conversation_wire_shape_is_stable() {
    let value = serde_json::json!({"ConversationSnapshot": {
        "window_id": 7,
        "agent_id": "agent-3",
        "conversation": {
            "window_id": 7,
            "agent_id": "agent-3",
            "session_id": "sess-9",
            "runtime": "claude",
            "rev": 214,
            "degraded": "bad_record",
            "dropped_turns": 4,
            "dropped_by": "turns",
            "turns": [{
                "id": 11,
                "role": "user",
                "at_unix_secs": 1789123456,
                "state": "complete",
                "blocks": [
                    {"text": {"text": "mapping the call sites"}},
                    {"tool_call": {
                        "id": "tu-1",
                        "name": "Grep",
                        "summary": "34 matches",
                        "input": {"pattern": "parse_"},
                        "result": {
                            "ok": true,
                            "summary": "34 matches",
                            "detail": "crates/parse/src/lib.rs:12",
                            "truncated": false
                        },
                        "state": "ok",
                        "duration_ms": 312
                    }},
                    {"subagent_spawn": {
                        "agent_id": "agent-8",
                        "kind": "Explore",
                        "label": "find every call site",
                        "model": "haiku"
                    }},
                    {"notice": {
                        "kind": "permission_request",
                        "text": "Bash needs permission"
                    }}
                ]
            }]
        }
    }});
    let message: DaemonMsg = serde_json::from_value(value.clone()).unwrap();
    let packed = rmp_serde::to_vec_named(&message).unwrap();
    let back: DaemonMsg = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(back, message);
    assert_eq!(serde_json::to_value(back).unwrap(), value);
    // Tie each wire key to the Rust field it must land in, by name. The comparisons
    // above cannot do this: json -> struct -> json is a bijection that commutes with a
    // symmetric rename of two same-typed fields, so swapping the serde names of
    // `agent_id` and `session_id` leaves every comparison equal.
    let DaemonMsg::ConversationSnapshot { conversation, .. } = &message else {
        panic!("not a snapshot");
    };
    assert_eq!(conversation.agent_id.as_deref(), Some("agent-3"));
    assert_eq!(conversation.session_id.as_deref(), Some("sess-9"));
}
