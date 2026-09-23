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

/// Fix round 1, F2: the hook's `truncated` records that the CLI cut `tool_response`, the
/// summary's source. Once the transcript's full detail replaces `detail`, `truncated`
/// describes that detail: it is `false` when the detail fit the cap. A reset restores the
/// hook's flag.
#[test]
fn a_complete_detail_clears_the_hooks_truncated_flag_until_reset() {
    let mut cut = post("tu-2", "summary two");
    cut.tool_result_truncated = Some(true);
    let mut draft = draft_of(&[
        prompt("p-two"),
        pre("tu-2", "Read", json!({"file_path": "/a"})),
        cut,
    ]);
    let hook_flag = |draft: &crate::conversation::Draft| {
        tool_result(tool(&draft.turns, "tu-2"))
            .as_ref()
            .map(|r| r.truncated)
    };
    assert_eq!(hook_flag(&draft), Some(true), "fixture: the hook was cut");
    assert!(enrich(&mut draft, &[result("tu-2", "D2 full", true)]));
    let enriched = tool_result(tool(&draft.turns, "tu-2")).as_ref().unwrap();
    assert_eq!(enriched.detail.as_deref(), Some("D2 full"));
    assert!(!enriched.truncated);
    assert!(reset(&mut draft));
    assert_eq!(hook_flag(&draft), Some(true));
}

/// Fix round 1, F3: the same tool-use id in two different turns. The detail joins the
/// newest turn's call, as `build.rs`'s id tier does; the older call is untouched, and a
/// reset restores only the one that changed.
#[test]
fn a_tool_use_id_reused_in_a_later_turn_enriches_the_later_call() {
    let mut draft = draft_of(&[
        prompt("first"),
        pre("tu-1", "Bash", json!({"command": "old"})),
        post("tu-1", "old output"),
        stop(),
        prompt("second"),
        pre("tu-1", "Bash", json!({"command": "new"})),
        post("tu-1", "new output"),
        stop(),
    ]);
    let before = conversation(&draft);
    assert!(enrich(
        &mut draft,
        &[
            call("tu-1", json!({"command": "new, from the transcript"})),
            result("tu-1", "new detail", true),
        ]
    ));
    assert_eq!(
        draft.turns[1].blocks, before.turns[1].blocks,
        "older call untouched"
    );
    let newer = &draft.turns[3].blocks[0];
    assert_eq!(
        tool_input(newer),
        &Some(json!({"command": "new, from the transcript"}))
    );
    assert_eq!(
        tool_result(newer)
            .as_ref()
            .and_then(|r| r.detail.as_deref()),
        Some("new detail")
    );
    assert!(reset(&mut draft));
    assert_eq!(conversation(&draft), before);
}
