//! Ruling F-3 (2026-10-01): a failed Codex turn shows its error in the conversation, as
//! Claude's synthetic API-error message already does, instead of an empty assistant
//! turn. The text is one safe line.

use super::super::*;
use super::turn_end::Window;
use crate::headless::codex_stream;
use proto::{Block, Role};
use serde_json::json;

/// The trial's lines (2026-10-01), as Codex wrote them.
fn trial_lines(message: &str) -> Vec<String> {
    vec![
        json!({"type": "thread.started", "thread_id": "01a0f69e-ef66-7252-b72e-7b1d882eb258"})
            .to_string(),
        json!({"type": "turn.started"}).to_string(),
        json!({"type": "error", "message": message}).to_string(),
        json!({"type": "turn.failed", "error": {"message": message}}).to_string(),
    ]
}

fn assistant_texts(w: &Window) -> Vec<String> {
    let conversation = w.conversation();
    let turn = conversation.turns.last().expect("a turn");
    assert_eq!(turn.role, Role::Assistant, "{conversation:?}");
    turn.blocks
        .iter()
        .filter_map(|b| match b {
            Block::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_failed_codex_turn_shows_its_error() {
    let message = r#"{"type":"error","status":400,"error":{"type":"invalid_request_error","message":"The 'gpt-6.1-sol' model is not supported when using Codex with a ChatGPT account."}}"#;
    let mut w = Window::new(Runtime::Codex, false, StreamCursor::default(), false);
    w.sent("review this change");
    for line in trial_lines(message) {
        for event in codex_stream::parse_line(&line) {
            w.event(&event);
        }
    }
    w.event(&SessionEvent::ProcessExited {
        code: Some(1),
        signal: None,
    });
    assert_eq!(
        assistant_texts(&w),
        [
            "Turn failed: The 'gpt-6.1-sol' model is not supported when using Codex with a ChatGPT account."
        ]
    );
}

#[test]
fn a_failed_codex_turns_error_is_one_safe_line() {
    let message = "first\nsecond\u{1b}[31m red\u{202e} end";
    let mut w = Window::new(Runtime::Codex, false, StreamCursor::default(), false);
    w.sent("go");
    for line in trial_lines(message) {
        for event in codex_stream::parse_line(&line) {
            w.event(&event);
        }
    }
    let texts = assistant_texts(&w);
    assert_eq!(texts.len(), 1, "{texts:?}");
    assert!(
        texts[0].starts_with("Turn failed: first second"),
        "{texts:?}"
    );
    assert!(
        !texts[0].chars().any(|c| c.is_control() || c == '\u{202e}'),
        "{texts:?}"
    );
}
