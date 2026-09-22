//! Tests for `enrich::apply` and `enrich::reset` (task M6.5.8), each starting from a
//! hook-built timeline with distinct values: every prompt, reply, tool id, tool name,
//! input and result in a test differs from its siblings, so a mapping that reverses,
//! offsets or transposes two of them lands a *wrong* value where a test looks, rather
//! than an equal one. `ConversationSet::enrich`/`reset_enrichment`, the public entry
//! points, are tested in `enrich_store_tests.rs`.

use super::*;
use crate::conversation::build;
use crate::hooks::{HookKind, ParsedHook};
use proto::{Block, Role, ToolResult, ToolState, TurnState};
use serde_json::json;
use std::time::Instant;

pub(super) fn hook(kind: HookKind) -> ParsedHook {
    ParsedHook {
        source: proto::HookSource::Claude,
        kind,
        session_id: None,
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

pub(super) fn prompt(text: &str) -> ParsedHook {
    let mut h = hook(HookKind::UserPromptSubmit);
    h.prompt = Some(text.to_owned());
    h
}

pub(super) fn pre(id: &str, name: &str, input: serde_json::Value) -> ParsedHook {
    let mut h = hook(HookKind::PreToolUse);
    h.tool_use_id = Some(id.to_owned());
    h.tool_name = Some(name.to_owned());
    h.tool_input = Some(input);
    h
}

pub(super) fn post(id: &str, response: &str) -> ParsedHook {
    let mut h = hook(HookKind::PostToolUse);
    h.tool_use_id = Some(id.to_owned());
    h.tool_response = Some(json!(response));
    h
}

pub(super) fn stop() -> ParsedHook {
    hook(HookKind::Stop)
}

pub(super) fn session(id: &str) -> ParsedHook {
    let mut h = hook(HookKind::SessionStart);
    h.session_id = Some(id.to_owned());
    h
}

pub(super) fn user(ordinal: u32, text: &str) -> Record {
    Record::UserText {
        session_id: None,
        ordinal,
        text: text.to_owned(),
    }
}

pub(super) fn said(ordinal: u32, text: &str) -> Record {
    Record::AssistantText {
        session_id: None,
        ordinal,
        text: text.to_owned(),
    }
}

pub(super) fn call(id: &str, input: serde_json::Value) -> Record {
    Record::ToolDetail {
        tool_use_id: id.to_owned(),
        input: Some(input),
        detail: None,
        ok: None,
    }
}

pub(super) fn result(id: &str, detail: &str, ok: bool) -> Record {
    Record::ToolDetail {
        tool_use_id: id.to_owned(),
        input: None,
        detail: Some(detail.to_owned()),
        ok: Some(ok),
    }
}

pub(super) fn build_draft(hooks: &[ParsedHook], caps: Caps) -> Draft {
    let mut draft = Draft::new(4, None, proto::Runtime::Claude);
    let now = Instant::now();
    for (ts, h) in hooks.iter().enumerate() {
        build::apply(
            &mut draft,
            proto::Runtime::Claude,
            h,
            None,
            ts as u64,
            now,
            caps,
        );
    }
    draft
}

pub(super) fn draft_of(hooks: &[ParsedHook]) -> Draft {
    build_draft(hooks, Caps::default())
}

pub(super) fn conversation(draft: &Draft) -> proto::Conversation {
    draft.to_conversation(0, None, 0, None)
}

pub(super) fn enrich(draft: &mut Draft, records: &[Record]) -> bool {
    apply(draft, records, Caps::default())
}

/// The leading `Text` block's text of turn `index`, or `None` when its first block is
/// not a `Text` (or it has no blocks).
pub(super) fn leading_text(turns: &[proto::Turn], index: usize) -> Option<String> {
    match turns[index].blocks.first() {
        Some(Block::Text { text }) => Some(text.clone()),
        _ => None,
    }
}

/// The `ToolCall` block with `id == want`, anywhere in `turns`.
pub(super) fn tool<'a>(turns: &'a [proto::Turn], want: &str) -> &'a Block {
    turns
        .iter()
        .flat_map(|turn| &turn.blocks)
        .find(|block| matches!(block, Block::ToolCall { id: Some(id), .. } if id == want))
        .unwrap_or_else(|| panic!("no ToolCall {want}"))
}

pub(super) fn tool_result(block: &Block) -> &Option<ToolResult> {
    match block {
        Block::ToolCall { result, .. } => result,
        other => panic!("expected a ToolCall, got {other:?}"),
    }
}

pub(super) fn tool_input(block: &Block) -> &Option<serde_json::Value> {
    match block {
        Block::ToolCall { input, .. } => input,
        other => panic!("expected a ToolCall, got {other:?}"),
    }
}

pub(super) fn three_prompts() -> Vec<ParsedHook> {
    vec![
        prompt("p-one"),
        stop(),
        prompt("p-two"),
        stop(),
        prompt("p-three"),
        stop(),
    ]
}

#[test]
fn assistant_prose_lands_on_the_right_turn() {
    let mut draft = draft_of(&three_prompts());
    assert!(enrich(
        &mut draft,
        &[
            user(0, "p-one"),
            said(0, "first reply"),
            user(1, "p-two"),
            said(1, "second reply"),
            user(2, "p-three"),
            said(2, "third reply"),
        ],
    ));
    let turns = &draft.turns;
    assert_eq!(turns.len(), 6);
    assert_eq!(leading_text(turns, 1).as_deref(), Some("first reply"));
    assert_eq!(leading_text(turns, 3).as_deref(), Some("second reply"));
    assert_eq!(leading_text(turns, 5).as_deref(), Some("third reply"));
    // The User turns keep their own prompts; prose never lands on a User turn.
    assert_eq!(leading_text(turns, 0).as_deref(), Some("p-one"));
    assert_eq!(leading_text(turns, 2).as_deref(), Some("p-two"));
    assert_eq!(leading_text(turns, 4).as_deref(), Some("p-three"));
}

#[test]
fn a_turn_with_no_text_block_gains_one_at_the_front() {
    let mut draft = draft_of(&[
        prompt("list the files"),
        pre("tu-1", "Bash", json!({"command": "ls"})),
        post("tu-1", "a.rs"),
        stop(),
    ]);
    let before = draft.turns[1].blocks.clone();
    assert_eq!(before.len(), 1, "fixture: the turn holds only its ToolCall");
    assert!(enrich(
        &mut draft,
        &[user(0, "list the files"), said(0, "Listing them now.")],
    ));
    let blocks = &draft.turns[1].blocks;
    assert_eq!(blocks.len(), 2);
    assert_eq!(
        blocks[0],
        Block::Text {
            text: "Listing them now.".into()
        }
    );
    assert_eq!(blocks[1], before[0], "the ToolCall is byte-identical");
}

fn three_tools_in_three_turns() -> Vec<ParsedHook> {
    vec![
        prompt("first task"),
        pre("tu-1", "Bash", json!({"command": "ls"})),
        post("tu-1", "one.rs"),
        stop(),
        prompt("second task"),
        pre("tu-2", "Read", json!({"file_path": "/two.rs"})),
        post("tu-2", "fn two()"),
        stop(),
        prompt("third task"),
        pre("tu-3", "Grep", json!({"pattern": "three"})),
        post("tu-3", "three matches"),
        stop(),
    ]
}

#[test]
fn tool_detail_matches_by_id() {
    let mut draft = draft_of(&three_tools_in_three_turns());
    let before = draft.turns.clone();
    assert!(enrich(
        &mut draft,
        &[
            call("tu-2", json!({"file_path": "/two.rs", "limit": 20})),
            result("tu-2", "fn two() { 2 }", true),
        ],
    ));
    assert_eq!(tool(&draft.turns, "tu-1"), tool(&before, "tu-1"));
    assert_eq!(tool(&draft.turns, "tu-3"), tool(&before, "tu-3"));
    let two = tool(&draft.turns, "tu-2");
    assert_eq!(
        tool_input(two),
        &Some(json!({"file_path": "/two.rs", "limit": 20}))
    );
    let result = tool_result(two).as_ref().expect("tu-2 has its hook result");
    assert_eq!(result.detail.as_deref(), Some("fn two() { 2 }"));
    // Everything else on the result is the hook's.
    assert_eq!(result.summary, "fn two()");
    assert!(result.ok);
    assert!(!result.truncated);
}

#[test]
fn enrichment_never_creates_reorders_or_removes_a_turn() {
    let mut hooks = three_tools_in_three_turns();
    // A fourth turn with a still-Pending call, so a `ToolDetail` can disagree with a
    // `Pending` state as well as an `Ok` one.
    hooks.push(prompt("fourth task"));
    hooks.push(pre("tu-4", "Glob", json!({"pattern": "*.md"})));
    let mut draft = draft_of(&hooks);
    let before = conversation(&draft);

    enrich(
        &mut draft,
        &[
            user(0, "first task"),
            said(0, "on it"),
            said(9, "prose for a turn that does not exist"),
            user(9, "a prompt that does not exist"),
            call("tu-unknown", json!({"x": 1})),
            result("tu-unknown", "nobody's output", false),
            // `ok` disagrees with the hook-derived `Ok`.
            result("tu-1", "it failed, says the transcript", false),
            // `ok` on a `Pending` call.
            result("tu-4", "it finished, says the transcript", true),
        ],
    );
    let after = conversation(&draft);

    let shape = |c: &proto::Conversation| {
        c.turns
            .iter()
            .map(|t| {
                let states: Vec<(ToolState, Option<bool>)> = t
                    .blocks
                    .iter()
                    .filter_map(|b| match b {
                        Block::ToolCall { state, result, .. } => {
                            Some((*state, result.as_ref().map(|r| r.ok)))
                        }
                        _ => None,
                    })
                    .collect();
                (t.id, t.role, t.state, states)
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(shape(&after), shape(&before));
    assert_eq!(after.turns.len(), 8);
    assert_eq!(
        shape(&after).iter().map(|s| s.1).collect::<Vec<_>>(),
        [Role::User, Role::Assistant].repeat(4)
    );
    assert_eq!(after.turns[7].state, TurnState::Running);
    assert_eq!(
        tool_result(tool(&after.turns, "tu-1"))
            .as_ref()
            .map(|r| r.ok),
        Some(true)
    );
}

#[test]
fn enrichment_never_sets_a_result_on_a_pending_call() {
    let mut draft = draft_of(&[
        prompt("run it"),
        pre("tu-1", "Bash", json!({"command": "make"})),
    ]);
    let changed = enrich(&mut draft, &[result("tu-1", "output", true)]);
    assert!(!changed, "nothing it may touch changed");
    match tool(&draft.turns, "tu-1") {
        Block::ToolCall { result, state, .. } => {
            assert_eq!(result, &None);
            assert_eq!(*state, ToolState::Pending);
        }
        _ => unreachable!(),
    }
}

#[test]
fn a_long_detail_is_capped_and_flagged() {
    let caps = Caps {
        max_result_bytes: 31,
        ..Caps::default()
    };
    let mut draft = build_draft(
        &[
            prompt("read it"),
            pre("tu-1", "Read", json!({"file_path": "/cjk.txt"})),
            post("tu-1", "short"),
            stop(),
        ],
        caps,
    );
    assert_eq!(
        tool_result(tool(&draft.turns, "tu-1"))
            .as_ref()
            .map(|r| r.truncated),
        Some(false),
        "fixture: the hook's own result is not truncated"
    );
    let detail = "日".repeat(50);
    assert_eq!(detail.len(), 150);
    assert!(apply(&mut draft, &[result("tu-1", &detail, true)], caps));
    let result = tool_result(tool(&draft.turns, "tu-1")).as_ref().unwrap();
    let stored = result.detail.as_deref().unwrap();
    assert_eq!(stored.len(), 30);
    assert_eq!(stored.chars().count(), 10);
    assert_eq!(stored, "日".repeat(10));
    assert!(result.truncated);
}

#[test]
fn a_detail_within_the_cap_is_not_flagged() {
    let caps = Caps {
        max_result_bytes: 31,
        ..Caps::default()
    };
    let mut draft = build_draft(
        &[
            prompt("read it"),
            pre("tu-1", "Read", json!({"file_path": "/cjk.txt"})),
            post("tu-1", "short"),
        ],
        caps,
    );
    // Exactly 30 bytes: under the cap, kept whole.
    assert!(apply(
        &mut draft,
        &[result("tu-1", &"日".repeat(10), true)],
        caps
    ));
    let result = tool_result(tool(&draft.turns, "tu-1")).as_ref().unwrap();
    assert_eq!(result.detail.as_deref(), Some("日日日日日日日日日日"));
    assert!(!result.truncated);
}

/// Re-applying without a reset appends prose a second time (visibly duplicated, never
/// misattributed); the only re-read path, task M6.5.10's restart, resets first.
#[test]
fn enrichment_is_idempotent() {
    let hooks = [
        prompt("check the build"),
        pre("tu-1", "Bash", json!({"command": "cargo build"})),
        post("tu-1", "Compiling"),
        stop(),
        prompt("now the tests"),
        stop(),
    ];
    let batch_a = [
        user(0, "check the build"),
        said(0, "Building."),
        call("tu-1", json!({"command": "cargo build", "timeout": 60})),
    ];
    let batch_b = [
        result("tu-1", "Compiling anthrex\nFinished", true),
        said(0, "It builds."),
        user(1, "now the tests"),
        said(1, "Running them."),
    ];
    let mut draft = draft_of(&hooks);
    let hook_only = conversation(&draft);

    assert!(enrich(&mut draft, &batch_a));
    assert!(enrich(&mut draft, &batch_b));
    let first = conversation(&draft);
    assert_ne!(first, hook_only, "fixture: enrichment changed something");
    assert_eq!(
        leading_text(&first.turns, 1).as_deref(),
        Some("Building.\n\nIt builds.")
    );

    assert!(reset(&mut draft));
    assert_eq!(
        conversation(&draft),
        hook_only,
        "reset restores the hooks' own values"
    );
    assert!(
        !reset(&mut draft),
        "a second reset has nothing left to drop"
    );

    assert!(enrich(&mut draft, &batch_a));
    assert!(enrich(&mut draft, &batch_b));
    assert_eq!(conversation(&draft), first);

    // A batch that names only turns and tools that do not exist changes nothing.
    assert!(!enrich(
        &mut draft,
        &[
            user(7, "no such prompt"),
            said(7, "no such reply"),
            call("tu-9", json!({})),
            result("tu-9", "no such output", true),
        ],
    ));
    assert_eq!(conversation(&draft), first);
}

#[test]
fn several_assistant_texts_in_one_turn_are_all_kept() {
    let mut draft = draft_of(&[prompt("check it"), stop()]);
    assert!(enrich(
        &mut draft,
        &[
            user(0, "check it"),
            said(0, "Starting the check."),
            said(0, "ready"),
        ],
    ));
    assert_eq!(
        leading_text(&draft.turns, 1).as_deref(),
        Some("Starting the check.\n\nready")
    );
    assert_eq!(
        draft.turns[1].blocks.len(),
        1,
        "one block, not one per record"
    );
}

#[test]
fn assistant_texts_split_across_batches_append_in_order() {
    let mut draft = draft_of(&[prompt("check it"), stop()]);
    assert!(enrich(
        &mut draft,
        &[user(0, "check it"), said(0, "Starting the check.")]
    ));
    assert!(enrich(&mut draft, &[said(0, "ready")]));
    assert_eq!(
        leading_text(&draft.turns, 1).as_deref(),
        Some("Starting the check.\n\nready")
    );
}
