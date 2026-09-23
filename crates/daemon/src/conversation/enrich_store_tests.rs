//! Tests for `ConversationSet::enrich` and `ConversationSet::reset_enrichment` (task
//! M6.5.8), the public entry points over `enrich::apply`/`enrich::reset`: routing, `rev`,
//! the degrade reason, and ordinals against a turn list the caps have trimmed.

use super::tests::{call, hook, leading_text, post, pre, prompt, result, said, stop, tool, user};
use crate::conversation::{Caps, ConversationSet};
use crate::hooks::{HookKind, ParsedHook};
use crate::transcript::Record;
use proto::{Block, DegradeReason, TurnPatch};
use serde_json::json;
use std::time::Instant;

fn feed(set: &mut ConversationSet, hooks: &[ParsedHook], caps: Caps) {
    let now = Instant::now();
    for (ts, h) in hooks.iter().enumerate() {
        set.on_hook(proto::Runtime::Claude, h, None, ts as u64, now, caps);
    }
}

fn root(set: &ConversationSet) -> proto::Conversation {
    set.snapshot(None).expect("the root conversation exists")
}

fn rev(set: &ConversationSet) -> u64 {
    root(set).rev
}

fn three_prompts() -> Vec<ParsedHook> {
    vec![
        prompt("p-zero"),
        stop(),
        prompt("p-one"),
        stop(),
        prompt("p-two"),
        stop(),
    ]
}

fn three_ordinals() -> Vec<Record> {
    vec![
        user(0, "p-zero"),
        said(0, "reply zero"),
        user(1, "p-one"),
        said(1, "reply one"),
        user(2, "p-two"),
        said(2, "reply two"),
    ]
}

fn every_text(conversation: &proto::Conversation) -> Vec<String> {
    conversation
        .turns
        .iter()
        .flat_map(|turn| &turn.blocks)
        .filter_map(|block| match block {
            Block::Text { text } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_store_enriches_the_root_and_advances_rev_once_per_changing_call() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    feed(&mut set, &three_prompts(), Caps::default());
    let before = rev(&set);

    assert_eq!(set.enrich(&three_ordinals(), Caps::default()), vec![None]);
    assert_eq!(rev(&set), before + 1);
    let conversation = root(&set);
    assert_eq!(
        leading_text(&conversation.turns, 1).as_deref(),
        Some("reply zero")
    );
    assert_eq!(
        leading_text(&conversation.turns, 5).as_deref(),
        Some("reply two")
    );

    // The revision's patches upsert exactly the three Assistant turns it changed.
    let (_, patches) = set.delta_since(None, before).expect("within the ring");
    let upserted: Vec<u64> = patches
        .iter()
        .map(|p| match p {
            TurnPatch::Upsert(turn) => turn.id,
            TurnPatch::Drop { id } => panic!("unexpected drop of {id}"),
        })
        .collect();
    assert_eq!(
        upserted,
        vec![
            conversation.turns[1].id,
            conversation.turns[3].id,
            conversation.turns[5].id
        ]
    );

    // A batch that changes nothing reports nothing and advances nothing.
    assert!(
        set.enrich(
            &[user(8, "nope"), result("tu-none", "x", true)],
            Caps::default()
        )
        .is_empty()
    );
    assert_eq!(rev(&set), before + 1);
}

#[test]
fn enrich_with_no_root_conversation_creates_none() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    assert!(set.enrich(&three_ordinals(), Caps::default()).is_empty());
    assert_eq!(set.snapshot(None), None);
    assert!(set.keys().is_empty());
}

/// Routing decision: a transcript file is the root conversation's
/// (`ConversationSet::transcript_path` is the root's), so its records enrich the root
/// only -- a `ToolDetail` naming a sub-agent's call does not reach into the sub-agent.
#[test]
fn records_enrich_the_root_not_a_subagent() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let mut spawn = hook(HookKind::SubagentStart);
    spawn.agent_id = Some("agent-1".into());
    let mut sub_pre = pre("tu-sub", "Read", json!({"file_path": "/sub.rs"}));
    sub_pre.agent_id = Some("agent-1".into());
    let mut sub_post = post("tu-sub", "fn sub()");
    sub_post.agent_id = Some("agent-1".into());
    feed(
        &mut set,
        &[
            prompt("delegate"),
            pre("tu-root", "Task", json!({"description": "look"})),
            spawn,
            sub_pre,
            sub_post,
        ],
        Caps::default(),
    );
    let sub_before = set.snapshot(Some("agent-1")).expect("sub-agent exists");

    let changed = set.enrich(
        &[
            call("tu-root", json!({"description": "look closely"})),
            result("tu-sub", "fn sub() { transcript }", true),
        ],
        Caps::default(),
    );
    assert_eq!(changed, vec![None]);
    assert_eq!(set.snapshot(Some("agent-1")), Some(sub_before));
    match tool(&root(&set).turns, "tu-root") {
        Block::ToolCall { input, .. } => {
            assert_eq!(input, &Some(json!({"description": "look closely"})))
        }
        _ => unreachable!(),
    }
}

#[test]
fn ordinals_survive_a_trimmed_turn_list() {
    let caps = Caps {
        max_turns: 4,
        ..Caps::default()
    };
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    feed(&mut set, &three_prompts(), caps);
    let trimmed = root(&set);
    assert_eq!(
        trimmed.dropped_turns, 2,
        "fixture: p-zero and its reply dropped"
    );
    assert_eq!(leading_text(&trimmed.turns, 0).as_deref(), Some("p-one"));

    assert_eq!(set.enrich(&three_ordinals(), caps), vec![None]);
    let conversation = root(&set);
    assert_eq!(conversation.turns.len(), 4);
    assert_eq!(
        leading_text(&conversation.turns, 1).as_deref(),
        Some("reply one")
    );
    assert_eq!(
        leading_text(&conversation.turns, 3).as_deref(),
        Some("reply two")
    );
    assert!(!every_text(&conversation).contains(&"reply zero".to_string()));
    assert_eq!(conversation.degraded, None);
}

/// A trim can drop a `User` turn and keep its reply: `turns[0]` is then an `Assistant`
/// turn whose prompt is gone. Its prose is dropped with the prompt (ordinal 0 is below
/// `dropped_user_turns`), and ordinal 1 still finds `p-one`'s reply, not the orphan.
#[test]
fn a_trim_between_a_prompt_and_its_reply_does_not_shift_the_ordinals() {
    let caps = Caps {
        max_turns: 5,
        ..Caps::default()
    };
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    feed(&mut set, &three_prompts(), caps);
    let trimmed = root(&set);
    assert_eq!(trimmed.dropped_turns, 1, "fixture: only p-zero dropped");
    assert_eq!(trimmed.turns[0].role, proto::Role::Assistant);

    assert_eq!(set.enrich(&three_ordinals(), caps), vec![None]);
    let conversation = root(&set);
    assert_eq!(
        leading_text(&conversation.turns, 0),
        None,
        "orphan stays bare"
    );
    assert_eq!(
        leading_text(&conversation.turns, 2).as_deref(),
        Some("reply one")
    );
    assert_eq!(
        leading_text(&conversation.turns, 4).as_deref(),
        Some("reply two")
    );
    assert_eq!(conversation.degraded, None);
}

#[test]
fn misalignment_is_reported_and_survives_the_readers_own_reason() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    feed(&mut set, &three_prompts(), Caps::default());
    let before = rev(&set);

    let changed = set.enrich(
        &[user(0, "p-zero"), user(1, "p-two"), said(1, "wrong turn")],
        Caps::default(),
    );
    assert_eq!(changed, vec![None]);
    assert_eq!(rev(&set), before + 1);
    assert_eq!(root(&set).degraded, Some(DegradeReason::Misaligned));
    assert!(!every_text(&root(&set)).contains(&"wrong turn".to_string()));

    // The reader clearing its own reason does not clear the enricher's.
    set.set_degraded(None);
    assert_eq!(root(&set).degraded, Some(DegradeReason::Misaligned));
    // A reader reason, when there is one, is the one shown.
    set.set_degraded(Some(DegradeReason::BadRecord));
    assert_eq!(root(&set).degraded, Some(DegradeReason::BadRecord));
    set.set_degraded(None);

    let at = rev(&set);
    assert_eq!(set.reset_enrichment(), vec![None]);
    assert_eq!(rev(&set), at + 1);
    assert_eq!(root(&set).degraded, None);
}

/// The brief's idempotence property at the public entry point. `reset_enrichment` is a
/// change (prose disappears) and re-applying is a change back, so each advances `rev`
/// by exactly one; the re-read's end state equals the first read's turn for turn. See
/// the task report for why "the second application reports no change" is not
/// achievable with separate reset and enrich calls.
#[test]
fn a_reset_and_reread_ends_where_the_first_read_did() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    feed(&mut set, &three_prompts(), Caps::default());
    let hook_only = root(&set);
    let records = three_ordinals();
    let (a, b) = records.split_at(3);

    set.enrich(a, Caps::default());
    set.enrich(b, Caps::default());
    let first = root(&set);

    assert_eq!(set.reset_enrichment(), vec![None]);
    assert_eq!(root(&set).turns, hook_only.turns);
    assert_eq!(rev(&set), first.rev + 1);
    assert!(set.reset_enrichment().is_empty(), "nothing left to drop");
    assert_eq!(rev(&set), first.rev + 1);

    assert_eq!(set.enrich(a, Caps::default()), vec![None]);
    assert_eq!(set.enrich(b, Caps::default()), vec![None]);
    let second = root(&set);
    assert_eq!(second.turns, first.turns);
    assert_eq!(second.degraded, first.degraded);
    assert_eq!(second.rev, first.rev + 3);
}

#[test]
fn reset_enrichment_touches_only_the_conversations_it_changes() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let mut spawn = hook(HookKind::SubagentStart);
    spawn.agent_id = Some("agent-1".into());
    feed(&mut set, &[prompt("p-zero"), spawn], Caps::default());
    let sub_rev = set.snapshot(Some("agent-1")).unwrap().rev;

    assert!(set.reset_enrichment().is_empty(), "nothing enriched yet");
    set.enrich(&[user(0, "p-zero"), said(0, "reply zero")], Caps::default());
    assert_eq!(set.reset_enrichment(), vec![None]);
    assert_eq!(set.snapshot(Some("agent-1")).unwrap().rev, sub_rev);
}

/// Prose adds bytes, so enrichment enforces `max_bytes` like a hook does: the oldest
/// turns go, with `Drop` patches in the same revision.
#[test]
fn enrichment_growth_is_capped() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    feed(&mut set, &three_prompts(), Caps::default());
    let hook_bytes = root(&set).byte_size();
    let caps = Caps {
        max_bytes: hook_bytes + 100,
        ..Caps::default()
    };
    let before = rev(&set);
    let long = "x".repeat(400);
    assert_eq!(
        set.enrich(&[user(0, "p-zero"), said(0, &long)], caps),
        vec![None]
    );
    let conversation = root(&set);
    assert!(conversation.dropped_turns >= 1);
    assert!(conversation.byte_size() <= caps.max_bytes);
    let (_, patches) = set.delta_since(None, before).unwrap();
    assert!(patches.iter().any(|p| matches!(p, TurnPatch::Drop { .. })));
}
