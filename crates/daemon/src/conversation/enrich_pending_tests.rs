//! Tests for the pending buffer (task M6.5.8 fix round 1, F1): a transcript record read
//! before its hook is parked and lands when the hook does, whichever of the two arrives
//! first. Each race is driven through the public `ConversationSet` entry points, since
//! the retry after a hook lives in `on_hook`, and `rev` is asserted to advance exactly
//! once for the hook that brings the parked record in.

use super::tests::{
    call, draft_of, enrich, leading_text, post, pre, prompt, result, said, stop, tool, user,
};
use super::{PENDING_POSITIONAL_MAX, PENDING_TOOLS_MAX, apply, retry};
use crate::conversation::{Caps, ConversationSet, build};
use crate::hooks::ParsedHook;
use crate::transcript::Record;
use proto::{Block, DegradeReason, ToolState};
use serde_json::json;
use std::time::Instant;

fn set_of(hooks: &[ParsedHook]) -> ConversationSet {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    for h in hooks {
        hook(&mut set, h);
    }
    set
}

fn hook(set: &mut ConversationSet, h: &ParsedHook) -> Vec<Option<String>> {
    set.on_hook(
        proto::Runtime::Claude,
        h,
        None,
        1,
        Instant::now(),
        Caps::default(),
    )
}

fn feed(set: &mut ConversationSet, records: &[Record]) -> Vec<Option<String>> {
    set.enrich(records, Caps::default())
}

fn root(set: &ConversationSet) -> proto::Conversation {
    set.snapshot(None).expect("root exists")
}

fn detail_of(conversation: &proto::Conversation, id: &str) -> Option<String> {
    match tool(&conversation.turns, id) {
        Block::ToolCall { result, .. } => result.as_ref().and_then(|r| r.detail.clone()),
        _ => unreachable!(),
    }
}

fn input_of(conversation: &proto::Conversation, id: &str) -> Option<serde_json::Value> {
    match tool(&conversation.turns, id) {
        Block::ToolCall { input, .. } => input.clone(),
        _ => unreachable!(),
    }
}

fn every_text(conversation: &proto::Conversation) -> Vec<String> {
    conversation
        .turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter_map(|b| match b {
            Block::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Race 1: the transcript's `tool_result` line is read before `PostToolUse` lands.
#[test]
fn a_result_read_before_its_post_tool_use_lands_with_the_hook() {
    let mut set = set_of(&[
        prompt("p-one"),
        pre("tu-1", "Bash", json!({"command": "ls"})),
    ]);
    let before = root(&set).rev;
    assert!(feed(&mut set, &[result("tu-1", "FULL DETAIL", true)]).is_empty());
    assert_eq!(root(&set).rev, before, "parking is not a visible change");

    assert_eq!(hook(&mut set, &post("tu-1", "hook summary")), vec![None]);
    let conversation = root(&set);
    assert_eq!(
        conversation.rev,
        before + 1,
        "hook and retry share one revision"
    );
    assert_eq!(
        detail_of(&conversation, "tu-1").as_deref(),
        Some("FULL DETAIL")
    );
}

/// Race 2: the transcript's `tool_use` line is read before `PreToolUse` lands.
#[test]
fn a_call_read_before_its_pre_tool_use_lands_with_the_hook() {
    let mut set = set_of(&[prompt("p-one")]);
    assert!(
        feed(
            &mut set,
            &[call("tu-1", json!({"command": "TRANSCRIPT INPUT"}))]
        )
        .is_empty()
    );
    let before = root(&set).rev;
    hook(
        &mut set,
        &pre("tu-1", "Bash", json!({"command": "hook input"})),
    );
    let conversation = root(&set);
    assert_eq!(conversation.rev, before + 1);
    assert_eq!(
        input_of(&conversation, "tu-1"),
        Some(json!({"command": "TRANSCRIPT INPUT"}))
    );
    hook(&mut set, &post("tu-1", "summary"));
    feed(&mut set, &[result("tu-1", "FULL DETAIL", true)]);
    assert_eq!(
        detail_of(&root(&set), "tu-1").as_deref(),
        Some("FULL DETAIL")
    );
}

/// Race 3: a prompt is read before its `UserPromptSubmit`. The prompt, and the prose
/// behind it (in this batch and a later one), wait for the hook's turn, then land in
/// order, in the hook's revision.
#[test]
fn a_prompt_read_before_its_hook_keeps_the_whole_turns_prose() {
    let mut set = set_of(&[prompt("p-zero"), stop()]);
    assert_eq!(
        feed(
            &mut set,
            &[
                user(0, "p-zero"),
                said(0, "reply zero"),
                user(1, "p-one"),
                said(1, "reply one"),
            ],
        ),
        vec![None]
    );
    assert!(feed(&mut set, &[said(1, "more one")]).is_empty(), "queued");
    let before = root(&set).rev;

    assert_eq!(hook(&mut set, &prompt("p-one")), vec![None]);
    let conversation = root(&set);
    assert_eq!(conversation.rev, before + 1);
    assert_eq!(
        leading_text(&conversation.turns, 1).as_deref(),
        Some("reply zero")
    );
    assert_eq!(
        leading_text(&conversation.turns, 3).as_deref(),
        Some("reply one\n\nmore one")
    );
    assert_eq!(conversation.degraded, None);
}

/// Nothing positional overtakes a parked prompt: prose read behind it waits, and a later
/// prompt that also has no turn yet parks again with its prose behind it.
#[test]
fn positional_records_keep_file_order_behind_a_parked_prompt() {
    let mut set = set_of(&[prompt("p-zero"), stop()]);
    feed(&mut set, &[user(0, "p-zero"), user(1, "p-one")]);
    feed(
        &mut set,
        &[
            said(1, "one-a"),
            said(1, "one-b"),
            user(2, "p-two"),
            said(2, "two-a"),
        ],
    );
    hook(&mut set, &prompt("p-one"));
    let conversation = root(&set);
    assert_eq!(
        leading_text(&conversation.turns, 3).as_deref(),
        Some("one-a\n\none-b")
    );
    assert!(!every_text(&conversation).contains(&"two-a".to_string()));

    hook(&mut set, &stop());
    hook(&mut set, &prompt("p-two"));
    let conversation = root(&set);
    assert_eq!(
        leading_text(&conversation.turns, 5).as_deref(),
        Some("two-a")
    );
    assert_eq!(conversation.degraded, None);
}

/// A hook that is really lost: the next prompt's hook fills the parked prompt's slot,
/// the texts differ, and the result is `Misaligned` with the prose applied nowhere.
#[test]
fn a_lost_prompt_hook_still_reports_misaligned() {
    let mut set = set_of(&[prompt("p-zero"), stop()]);
    feed(
        &mut set,
        &[user(0, "p-zero"), user(1, "p-one"), said(1, "reply one")],
    );
    assert_eq!(root(&set).degraded, None, "waiting is not a mismatch");
    hook(&mut set, &prompt("p-two"));
    let conversation = root(&set);
    assert_eq!(conversation.degraded, Some(DegradeReason::Misaligned));
    assert!(!every_text(&conversation).contains(&"reply one".to_string()));
    assert_eq!(leading_text(&conversation.turns, 3), None);
}

fn parked_prose(count: usize) -> Vec<Record> {
    let mut records = vec![user(0, "p-zero"), user(1, "p-one")];
    records.extend((0..count).map(|i| said(1, &format!("line {i}"))));
    records
}

/// The queue holds `PENDING_POSITIONAL_MAX` records: the parked prompt plus
/// `MAX - 1` behind it all land once the hook comes.
#[test]
fn a_full_positional_queue_still_lands() {
    let mut set = set_of(&[prompt("p-zero"), stop()]);
    feed(&mut set, &parked_prose(PENDING_POSITIONAL_MAX - 1));
    assert_eq!(root(&set).degraded, None);
    hook(&mut set, &prompt("p-one"));
    let prose = leading_text(&root(&set).turns, 3).expect("prose landed");
    assert_eq!(prose.split("\n\n").count(), PENDING_POSITIONAL_MAX - 1);
    assert!(prose.ends_with(&format!("line {}", PENDING_POSITIONAL_MAX - 2)));
}

/// One record more overflows: buffering stops, the queue is dropped, and positional
/// enrichment stops at the parked prompt's ordinal, reported as `Misaligned`. The hook
/// arriving afterwards brings nothing in.
#[test]
fn an_overflowing_positional_queue_reports_misaligned() {
    let mut set = set_of(&[prompt("p-zero"), stop()]);
    let before = root(&set).rev;
    assert_eq!(
        feed(&mut set, &parked_prose(PENDING_POSITIONAL_MAX)),
        vec![None]
    );
    assert_eq!(root(&set).rev, before + 1);
    assert_eq!(root(&set).degraded, Some(DegradeReason::Misaligned));
    hook(&mut set, &prompt("p-one"));
    assert_eq!(leading_text(&root(&set).turns, 3), None);

    let mut draft = draft_of(&[prompt("p-zero"), stop()]);
    enrich(&mut draft, &parked_prose(PENDING_POSITIONAL_MAX));
    assert_eq!(draft.enrichment.pending(), (0, 0), "stopped buffering");
}

/// Unmatched tool records are bounded too: past `PENDING_TOOLS_MAX` the oldest is
/// evicted, and only the ones still held land when their hooks come.
#[test]
fn the_tool_buffer_evicts_its_oldest_record() {
    let mut draft = draft_of(&[prompt("p-zero")]);
    let records: Vec<Record> = (0..=PENDING_TOOLS_MAX)
        .map(|i| result(&format!("tu-{i}"), &format!("detail {i}"), true))
        .collect();
    assert!(!enrich(&mut draft, &records));
    assert_eq!(draft.enrichment.pending(), (PENDING_TOOLS_MAX, 0));

    let now = Instant::now();
    let last = format!("tu-{PENDING_TOOLS_MAX}");
    for id in ["tu-0", "tu-1", last.as_str()] {
        for h in [pre(id, "Bash", json!({"command": id})), post(id, "summary")] {
            build::apply(
                &mut draft,
                proto::Runtime::Claude,
                &h,
                None,
                1,
                now,
                Caps::default(),
            );
        }
    }
    assert!(retry(&mut draft));
    let detail = |id: &str| match tool(&draft.turns, id) {
        Block::ToolCall { result, .. } => result.as_ref().and_then(|r| r.detail.clone()),
        _ => unreachable!(),
    };
    assert_eq!(detail("tu-0"), None, "evicted");
    assert_eq!(detail("tu-1").as_deref(), Some("detail 1"));
    assert_eq!(detail(&last), Some(format!("detail {PENDING_TOOLS_MAX}")));
}

/// A `Denied` call never gets a result, so a detail waiting for one is dropped rather
/// than held until eviction.
#[test]
fn a_detail_waiting_on_a_denied_call_is_dropped() {
    let mut draft = draft_of(&[prompt("go"), pre("tu-1", "Bash", json!({"command": "rm"}))]);
    assert!(!enrich(&mut draft, &[result("tu-1", "never shown", true)]));
    assert_eq!(draft.enrichment.pending(), (1, 0));
    build::apply(
        &mut draft,
        proto::Runtime::Claude,
        &stop(),
        None,
        1,
        Instant::now(),
        Caps::default(),
    );
    assert!(!retry(&mut draft));
    assert_eq!(draft.enrichment.pending(), (0, 0));
    match tool(&draft.turns, "tu-1") {
        Block::ToolCall { result, state, .. } => {
            assert_eq!(result, &None);
            assert_eq!(*state, ToolState::Denied);
        }
        _ => unreachable!(),
    }
}

/// `reset_enrichment` clears the buffer: nothing parked before a reset lands after it.
/// Parked records are not visible, so clearing only them is not a revision.
#[test]
fn reset_clears_the_pending_buffer() {
    let mut set = set_of(&[
        prompt("p-zero"),
        stop(),
        prompt("p-one"),
        pre("tu-1", "Bash", json!({})),
    ]);
    feed(
        &mut set,
        &[
            user(0, "p-zero"),
            user(1, "p-one"),
            result("tu-1", "parked detail", true),
            user(2, "p-two"),
            said(2, "parked prose"),
        ],
    );
    let before = root(&set).rev;
    assert!(
        set.reset_enrichment().is_empty(),
        "only invisible state held"
    );
    assert_eq!(root(&set).rev, before);

    hook(&mut set, &post("tu-1", "summary"));
    hook(&mut set, &stop());
    hook(&mut set, &prompt("p-two"));
    let conversation = root(&set);
    assert_eq!(detail_of(&conversation, "tu-1"), None);
    assert!(!every_text(&conversation).contains(&"parked prose".to_string()));

    let mut draft = draft_of(&[prompt("p-zero")]);
    apply(
        &mut draft,
        &[user(1, "later"), result("tu-9", "x", true)],
        Caps::default(),
    );
    assert_eq!(draft.enrichment.pending(), (1, 1));
    super::reset(&mut draft);
    assert_eq!(draft.enrichment.pending(), (0, 0));
}

/// `apply` retries the buffer first, so a record parked before a hook that reached the
/// draft without `retry` (here `build::apply` alone) still lands with the next batch,
/// ahead of that batch's own records.
#[test]
fn apply_retries_the_buffer_before_its_own_records() {
    let mut draft = draft_of(&[prompt("p-zero"), stop()]);
    enrich(
        &mut draft,
        &[user(0, "p-zero"), user(1, "p-one"), said(1, "first")],
    );
    build::apply(
        &mut draft,
        proto::Runtime::Claude,
        &prompt("p-one"),
        None,
        1,
        Instant::now(),
        Caps::default(),
    );
    assert!(enrich(&mut draft, &[said(1, "second")]));
    assert_eq!(
        leading_text(&draft.turns, 3).as_deref(),
        Some("first\n\nsecond")
    );
    assert_eq!(draft.enrichment.pending(), (0, 0));
}
