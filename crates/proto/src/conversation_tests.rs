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
}

#[test]
fn a_snapshots_envelope_matches_its_payload() {
    let mismatched = DaemonMsg::ConversationSnapshot {
        window_id: 7,
        agent_id: Some("agent-3".into()),
        conversation: Conversation {
            window_id: 8,
            ..distinct_conversation()
        },
    };
    assert!(!mismatched.conversation_envelope_is_consistent());

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

    let name = "Grep".to_string();
    let summary = "34 matches found".to_string();
    let result_summary = "34 matches".to_string();
    let result_detail = "crates/parse/src/lib.rs:12".to_string();
    // Every string below has a distinct length, so a formula that mixed up which
    // field contributes which length would not accidentally still balance.
    assert_eq!(
        [
            name.len(),
            summary.len(),
            result_summary.len(),
            result_detail.len()
        ]
        .iter()
        .collect::<std::collections::HashSet<_>>()
        .len(),
        4,
        "fixture lengths must be pairwise distinct"
    );

    let tool_turn = Turn {
        id: 2,
        role: Role::Assistant,
        at_unix_secs: 1,
        state: TurnState::Complete,
        blocks: vec![Block::ToolCall {
            id: Some("tu-9".into()),
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
    let expected = TURN_OVERHEAD
        + BLOCK_OVERHEAD
        + name.len()
        + summary.len()
        + "{\"a\":1}".len()
        + result_summary.len()
        + result_detail.len();
    assert_eq!(tool_turn.byte_size(), expected);
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
