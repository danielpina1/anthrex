//! M8c.11: a Codex tool call's one-line summary is its response's text, not the
//! synthesised `{"output": …}` as JSON. Hostile texts, and the result caps, through the
//! mapping into a real M6.5 `ConversationSet`.

use super::super::*;
use crate::conversation::{Caps, ConversationSet};
use proto::{Block, ToolState};
use serde_json::{Value, json};
use std::time::Instant;

/// One Codex turn with one command whose result is `text`, through the mapping into a
/// real `ConversationSet`. `string_response` swaps the synthesised `{"output": text}` for
/// the bare string, the shape a Claude tool's string response has, to compare the two.
fn codex_call_result(text: &str, string_response: bool, caps: Caps) -> proto::ToolResult {
    codex_call(text, string_response, caps, true)
}

/// [`codex_call_result`], with the records' enrichment (the full `detail`) applied or
/// not: without it the result is exactly what the synthesised `PostToolUse` built.
fn codex_call(text: &str, string_response: bool, caps: Caps, enrich: bool) -> proto::ToolResult {
    let mut set = ConversationSet::new(6, Runtime::Codex);
    let mut cursor = StreamCursor::default();
    let apply = |set: &mut ConversationSet, input: ConversationInput| {
        for hook in &input.hooks {
            set.on_hook(Runtime::Codex, hook, None, 0, Instant::now(), caps);
        }
        if enrich {
            set.enrich(&input.records, caps);
        }
    };
    apply(
        &mut set,
        sent_turn(Runtime::Codex, false, "go", &mut cursor),
    );
    let events = [
        init("th"),
        SessionEvent::ToolUse {
            id: "item_1".into(),
            name: "Bash".into(),
            input: json!({"command": "cat big"}),
            parent: None,
        },
        SessionEvent::ToolResult {
            id: "item_1".into(),
            text: text.into(),
            ok: true,
            parent: None,
        },
    ];
    for event in &events {
        let mut input = map(Runtime::Codex, false, event, &mut cursor);
        if string_response {
            for hook in &mut input.hooks {
                if hook.kind == crate::hooks::HookKind::PostToolUse {
                    hook.tool_response = Some(Value::String(text.into()));
                }
            }
        }
        apply(&mut set, input);
    }
    let conversation = set.snapshot(None).unwrap();
    conversation
        .turns
        .iter()
        .flat_map(|t| &t.blocks)
        .find_map(|b| match b {
            Block::ToolCall { result, .. } => result.clone(),
            _ => None,
        })
        .expect("the call has a result")
}

fn init(session: &str) -> SessionEvent {
    SessionEvent::Init {
        session_id: session.into(),
        model: None,
        mcp_ok: None,
    }
}

/// M8c.11, hostile: a 1 MB `output` summarises to its first line cut to the summary cap,
/// exactly as the same text as a string response does.
#[test]
fn a_one_megabyte_output_summarises_to_the_capped_first_line() {
    let text = "x".repeat(1 << 20);
    let result = codex_call_result(&text, false, Caps::default());
    let want = format!(
        "{}…",
        "x".repeat(crate::conversation::SUMMARY_MAX_GRAPHEMES)
    );
    assert_eq!(result.summary, want);
    let as_string = codex_call_result(&text, true, Caps::default());
    assert_eq!(result.summary, as_string.summary);
    // The stored result is unchanged by the summary's source: still marked truncated.
    assert!(result.truncated);
    assert!(result.ok);
}

/// M8c.11, hostile: an `output` with ANSI escapes gets the same summary as the same text
/// as a string response. `summary.rs` strips nothing (the client's `rows::clean` drops
/// control characters and escape sequences when it draws), so neither does this.
#[test]
fn an_output_with_escapes_summarises_like_a_string_response() {
    let text = "\u{1b}[31mred\u{1b}[0m\u{7}\tend\r\nsecond line";
    let result = codex_call_result(text, false, Caps::default());
    let as_string = codex_call_result(text, true, Caps::default());
    assert_eq!(result.summary, as_string.summary);
    assert_eq!(result.summary, "\u{1b}[31mred\u{1b}[0m\u{7}\tend");
}

/// M8c.11: the text field is cut to `max_result_bytes` like any response before its first
/// line is taken, and a response over that cap is still marked truncated, though the
/// hook itself was not.
#[test]
fn a_text_field_over_the_result_cap_is_cut_and_marked_truncated() {
    let caps = Caps {
        max_result_bytes: 64,
        ..Caps::default()
    };
    let text = "y".repeat(100);
    for enrich in [false, true] {
        let result = codex_call(&text, false, caps, enrich);
        assert_eq!(result.summary, "y".repeat(64));
        assert!(result.truncated, "enrich {enrich}");
        assert_eq!(result.detail.is_some(), enrich);
        assert_eq!(
            result.summary,
            codex_call(&text, true, caps, enrich).summary
        );
        // Under the cap, nothing is marked.
        let short = codex_call("short", false, caps, enrich);
        assert_eq!(short.summary, "short");
        assert!(!short.truncated);
    }
    // A failed call's `{"error": …}` is summarised by its text and still fails.
    let failed = {
        let mut set = ConversationSet::new(6, Runtime::Codex);
        let mut cursor = StreamCursor::default();
        let mut hooks = sent_turn(Runtime::Codex, false, "go", &mut cursor).hooks;
        for event in [
            SessionEvent::ToolUse {
                id: "item_1".into(),
                name: "Bash".into(),
                input: json!({"command": "false"}),
                parent: None,
            },
            SessionEvent::ToolResult {
                id: "item_1".into(),
                text: "exit 1\nstderr".into(),
                ok: false,
                parent: None,
            },
        ] {
            hooks.extend(map(Runtime::Codex, false, &event, &mut cursor).hooks);
        }
        for hook in &hooks {
            set.on_hook(Runtime::Codex, hook, None, 0, Instant::now(), caps);
        }
        let conversation = set.snapshot(None).unwrap();
        conversation.turns[1].blocks.iter().find_map(|b| match b {
            Block::ToolCall { result, state, .. } => Some((result.clone().unwrap(), *state)),
            _ => None,
        })
    };
    let (result, state) = failed.expect("the call");
    assert_eq!(result.summary, "exit 1");
    assert!(!result.ok);
    assert_eq!(state, ToolState::Failed);
}
