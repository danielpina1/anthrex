//! Tests for `enrich::apply`'s alignment check (task M6.5.8): every positional record
//! is checked against the hook-built prompt its ordinal maps to, and the first mismatch
//! stops positional enrichment. Split out of `enrich_tests.rs` by responsibility (the
//! ~600-line rule); the fixtures are that file's.

use super::tests::{
    draft_of, enrich, leading_text, post, pre, prompt, result, said, session, stop, three_prompts,
    tool, tool_result, user,
};
use crate::transcript::Record;
use serde_json::json;

/// Prose for ordinal `k` lands only once `k`'s own prompt has been checked. A prompt the
/// transcript counted but gave no text for (an image-only prompt) is never checked, so
/// its prose is not applied: missing prose, never prose on an unchecked turn.
#[test]
fn prose_without_a_checked_prompt_is_not_applied() {
    let mut draft = draft_of(&three_prompts());
    assert!(enrich(
        &mut draft,
        &[
            user(0, "p-one"),
            said(0, "first reply"),
            said(1, "second reply, prompt never checked"),
            user(2, "p-three"),
            said(2, "third reply"),
        ],
    ));
    assert_eq!(
        leading_text(&draft.turns, 1).as_deref(),
        Some("first reply")
    );
    assert_eq!(leading_text(&draft.turns, 3), None);
    assert_eq!(
        leading_text(&draft.turns, 5).as_deref(),
        Some("third reply")
    );
}

#[test]
fn a_misaligned_prompt_stops_positional_enrichment() {
    let mut draft = draft_of(&[
        prompt("one"),
        stop(),
        prompt("two"),
        stop(),
        prompt("three"),
        pre("tu-3", "Read", json!({"file_path": "/three.rs"})),
        post("tu-3", "fn three()"),
        stop(),
    ]);
    // The transcript missed prompt "two": its ordinal 1 is the hooks' "three".
    assert!(enrich(
        &mut draft,
        &[
            user(0, "one"),
            said(0, "reply to one"),
            user(1, "three"),
            said(1, "reply to three"),
            user(2, "four"),
            said(2, "reply to four"),
            result("tu-3", "fn three() { 3 }", true),
        ],
    ));
    let turns = &draft.turns;
    assert_eq!(leading_text(turns, 1).as_deref(), Some("reply to one"));
    assert_eq!(leading_text(turns, 3), None, "no prose on the hooks' 'two'");
    assert_eq!(
        leading_text(turns, 5),
        None,
        "no Text block inserted before tu-3"
    );
    assert_eq!(leading_text(turns, 2).as_deref(), Some("two"));
    assert_eq!(leading_text(turns, 4).as_deref(), Some("three"));
    assert!(draft.enrichment.is_misaligned());
    assert_eq!(
        tool_result(tool(turns, "tu-3"))
            .as_ref()
            .and_then(|r| r.detail.as_deref()),
        Some("fn three() { 3 }"),
        "ToolDetail joins by id, not by position"
    );

    // A later batch cannot resume past the mismatch, even with records that would line
    // up: ordinal 1 against the hooks' "two", and ordinal 2 against the hooks' "three",
    // both match on their face. Nothing at or past the first bad ordinal applies.
    let before = draft.turns.clone();
    assert!(!enrich(
        &mut draft,
        &[
            user(1, "two"),
            said(1, "late, and unchecked"),
            user(2, "three"),
            said(2, "plausible, and wrong"),
        ]
    ));
    assert_eq!(draft.turns, before);
    assert!(draft.enrichment.is_misaligned());
}

#[test]
fn a_prompt_from_another_session_is_a_mismatch() {
    let mut draft = draft_of(&[session("sess-a"), prompt("same words"), stop()]);
    assert!(enrich(
        &mut draft,
        &[
            Record::UserText {
                session_id: Some("sess-b".into()),
                ordinal: 0,
                text: "same words".into(),
                human: true,
            },
            Record::AssistantText {
                session_id: Some("sess-b".into()),
                ordinal: 0,
                text: "another session's reply".into(),
            },
        ],
    ));
    assert_eq!(leading_text(&draft.turns, 1), None);
    assert!(draft.enrichment.is_misaligned());

    let mut matching = draft_of(&[session("sess-a"), prompt("same words"), stop()]);
    assert!(enrich(
        &mut matching,
        &[
            Record::UserText {
                session_id: Some("sess-a".into()),
                ordinal: 0,
                text: "same words".into(),
                human: true,
            },
            Record::AssistantText {
                session_id: Some("sess-a".into()),
                ordinal: 0,
                text: "this session's reply".into(),
            },
        ],
    ));
    assert_eq!(
        leading_text(&matching.turns, 1).as_deref(),
        Some("this session's reply")
    );
    assert!(!matching.enrichment.is_misaligned());
}

/// Replaces the brief's `a_truncated_hook_prompt_still_aligns`: `build.rs` stores
/// `hook.prompt` verbatim and nothing upstream truncates it (see the task report), so
/// the only difference alignment tolerates is surrounding whitespace, and the
/// transcript's text replaces the hook's.
#[test]
fn a_hook_prompt_differing_only_in_surrounding_whitespace_aligns() {
    let mut draft = draft_of(&[prompt("  fix the parser\n"), stop()]);
    assert!(enrich(
        &mut draft,
        &[user(0, "fix the parser"), said(0, "Fixing it.")]
    ));
    assert_eq!(
        leading_text(&draft.turns, 0).as_deref(),
        Some("fix the parser")
    );
    assert_eq!(leading_text(&draft.turns, 1).as_deref(), Some("Fixing it."));
    assert!(!draft.enrichment.is_misaligned());
}

/// With no truncation to allow for, a prefix is a different prompt: accepting one would
/// line "yes" up with "yes please", which is how misattributed prose gets through.
#[test]
fn a_prompt_that_is_only_a_prefix_does_not_align() {
    for (hook_text, transcript_text) in [("yes", "yes please"), ("", "anything at all")] {
        let mut draft = draft_of(&[prompt(hook_text), stop()]);
        enrich(
            &mut draft,
            &[user(0, transcript_text), said(0, "misattributed")],
        );
        assert_eq!(leading_text(&draft.turns, 1), None, "{hook_text:?}");
        assert_eq!(leading_text(&draft.turns, 0).as_deref(), Some(hook_text));
        assert!(draft.enrichment.is_misaligned(), "{hook_text:?}");
    }
}
