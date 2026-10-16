//! Milestone 9.16 decision 25: the handoff prompt's last line and the handoff's texts.

use super::{
    FILL_UNKNOWN_TEXT, fill_attention_text, handed_off_name, handed_off_text, handoff_prompt,
};
use crate::run::model::HandoffRecord;

fn record(window_tokens: Option<u64>) -> HandoffRecord {
    HandoffRecord {
        from_run: "3f9a".into(),
        from_window: 7,
        fill_tokens: 170_000,
        window_tokens,
        at: 1_790_000_000,
    }
}

#[test]
fn handoff_prompt_names_the_fill() {
    let ended = ("o-3f9a", "3f9a", "accepted");
    let plain = handoff_prompt("first", ended, Some("added login"), None, None);
    let known = handoff_prompt(
        "first",
        ended,
        Some("added login"),
        None,
        Some(&record(Some(200_000))),
    );
    assert_eq!(
        known,
        format!("{plain}The previous session was 85% full (170000/200000); this one starts fresh.")
    );
    let unknown = handoff_prompt(
        "first",
        ended,
        Some("added login"),
        None,
        Some(&record(None)),
    );
    assert_eq!(
        unknown,
        format!("{plain}The previous session had 170000 tokens of context; this one starts fresh.")
    );
    assert!(!plain.contains("starts fresh"), "{plain}");
}

#[test]
fn the_handoffs_texts() {
    assert_eq!(handed_off_name("3f9a"), "3f9a/orchestrator (handed off)");
    assert_eq!(
        handed_off_text(&record(Some(200_000))),
        "handed off from 3f9a: context 85% full"
    );
    assert_eq!(
        handed_off_text(&record(None)),
        "handed off from 3f9a: context 170000 tokens"
    );
    assert_eq!(fill_attention_text(92), "orchestrator context 92% full");
    assert_eq!(
        FILL_UNKNOWN_TEXT,
        "context fill unknown; continuing in the same session"
    );
}
