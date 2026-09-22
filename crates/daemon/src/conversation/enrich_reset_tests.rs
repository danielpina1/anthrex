//! Tests for `enrich::reset` (task M6.5.8): it undoes exactly the enrichment -- every
//! replaced prompt, inserted prose block, tool `input`/`detail` and the alignment state
//! -- and nothing a hook set, before or after. Split out of `enrich_tests.rs` by
//! responsibility (the ~600-line rule); the fixtures are that file's.

use super::reset;
use super::tests::{
    call, conversation, draft_of, enrich, leading_text, post, pre, prompt, result, said, stop,
    tool, tool_input, tool_result, user,
};
use crate::conversation::{Caps, build};
use proto::{Block, TurnState};
use serde_json::json;
use std::time::Instant;

#[test]
fn reset_restores_every_enriched_value_and_nothing_else() {
    let mut draft = draft_of(&[
        prompt("  first  "),
        pre("tu-1", "Bash", json!({"command": "ls"})),
        post("tu-1", "a.rs"),
        stop(),
        prompt("second"),
        stop(),
        prompt("third"),
        stop(),
    ]);
    let hook_only = conversation(&draft);
    enrich(
        &mut draft,
        &[
            user(0, "first"),
            said(0, "Looking."),
            call("tu-1", json!({"command": "ls -la"})),
            result("tu-1", "a.rs\nb.rs", true),
            user(1, "second"),
            said(1, "Second reply."),
            user(2, "not third"),
        ],
    );
    assert!(draft.enrichment.is_misaligned());
    assert!(reset(&mut draft));
    assert_eq!(conversation(&draft), hook_only);
    assert!(!draft.enrichment.is_misaligned());
    // After a reset, alignment starts over: the ordinal that mismatched before can be
    // checked again against a corrected transcript.
    assert!(enrich(
        &mut draft,
        &[user(0, "first"), user(2, "third"), said(2, "Third reply.")]
    ));
    assert!(!draft.enrichment.is_misaligned());
    assert_eq!(
        leading_text(&draft.turns, 5).as_deref(),
        Some("Third reply.")
    );
}

/// A hook that arrives after enrichment keeps its own effect through a reset: reset
/// undoes the transcript's values, not the hooks' later ones.
#[test]
fn reset_keeps_hook_changes_made_after_enrichment() {
    let mut draft = draft_of(&[
        prompt("go"),
        pre("tu-1", "Bash", json!({"command": "make"})),
    ]);
    enrich(
        &mut draft,
        &[
            user(0, "go"),
            said(0, "Making."),
            call("tu-1", json!({"command": "make all"})),
        ],
    );
    let now = Instant::now();
    build::apply(
        &mut draft,
        proto::Runtime::Claude,
        &post("tu-1", "built"),
        None,
        9,
        now,
        Caps::default(),
    );
    build::apply(
        &mut draft,
        proto::Runtime::Claude,
        &stop(),
        None,
        10,
        now,
        Caps::default(),
    );
    assert!(reset(&mut draft));
    let turns = &draft.turns;
    assert_eq!(turns[1].blocks.len(), 1, "the inserted Text block is gone");
    assert_eq!(turns[1].state, TurnState::Complete);
    let block = tool(turns, "tu-1");
    assert_eq!(tool_input(block), &Some(json!({"command": "make"})));
    let result = tool_result(block)
        .as_ref()
        .expect("the hook's result survives");
    assert_eq!(result.summary, "built");
    assert_eq!(result.detail, None);
}

/// Two calls sharing one tool-use id in one turn (a redelivered `PreToolUse`): the
/// detail joins the last, as `build.rs`'s own id tier does, and a reset restores only
/// that one -- the other call's input must never be overwritten with its twin's.
#[test]
fn a_duplicated_tool_use_id_enriches_and_restores_only_the_last_call() {
    let mut draft = draft_of(&[
        prompt("twice"),
        pre("tu-1", "Bash", json!({"command": "first"})),
        pre("tu-1", "Bash", json!({"command": "second"})),
    ]);
    let before = conversation(&draft);
    assert!(enrich(
        &mut draft,
        &[call("tu-1", json!({"command": "from the transcript"}))]
    ));
    let inputs = |turns: &[proto::Turn]| -> Vec<Option<serde_json::Value>> {
        turns[1]
            .blocks
            .iter()
            .filter_map(|b| match b {
                Block::ToolCall { input, .. } => Some(input.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(
        inputs(&draft.turns),
        vec![
            Some(json!({"command": "first"})),
            Some(json!({"command": "from the transcript"})),
        ]
    );
    assert!(reset(&mut draft));
    assert_eq!(conversation(&draft), before);
}
