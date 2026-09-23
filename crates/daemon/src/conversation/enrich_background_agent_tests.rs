//! Enrichment of a Claude 2.1.278 conversation in which a background sub-agent finished
//! (the milestone's manual check). Claude injects a peer hand-back and a task
//! notification as user records, each of which fires `UserPromptSubmit`, so the hooks
//! build a `User` turn for each. The transcript is the committed fixture, parsed through
//! the Claude parser the reader uses, and applied through `ConversationSet::enrich`.

use super::tests::{leading_text, prompt, said, stop};
use crate::conversation::{Caps, ConversationSet};
use crate::hooks::ParsedHook;
use crate::transcript::{Cursor, Record, detect_head, parser_for};
use proto::{Block, DegradeReason};
use std::time::Instant;

const FIXTURE: &str =
    include_str!("../../tests/fixtures/transcripts/claude-2.1.278-background-agent.jsonl");

const HUMAN_PROMPT: &str = "use one Explore sub-agent to count the files under crates/ and wait \
    for its answer before replying";
const HUMAN_REPLY: &str = "Explore agent launched to count files under `crates/`. Holding my \
    answer until it reports back.";
const PEER_REPLY: &str = "**2 files** under `crates/`:\n\n1. `crates/a/lib.rs`\n2. \
    `crates/b/lib.rs`\n\nBoth are 0 bytes. No hidden files, no symlinks, nothing deeper — \
    consistent with the earlier sweep.";
const NOTIFICATION_REPLY: &str = "That's the completion event for the count agent — its \
    answer (2 files) is already reported above. Nothing new to add.";

/// The `message.content` of the fixture's user line whose `origin.kind` is `kind`.
fn content(kind: &str) -> String {
    FIXTURE
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .find(|v| v["type"] == "user" && v["origin"]["kind"] == kind)
        .and_then(|v| v["message"]["content"].as_str().map(str::to_owned))
        .unwrap_or_else(|| panic!("the fixture has a {kind} user line"))
}

/// The peer turn's hook prompt, as the view showed it: the record's inner
/// `<agent-message …>…</agent-message>` block, without the harness text around it.
fn peer_hook_prompt() -> String {
    let wrapped = content("peer");
    let start = wrapped
        .find("<agent-message")
        .expect("an agent-message block");
    let end = wrapped.find("</agent-message>").expect("its end") + "</agent-message>".len();
    let inner = wrapped[start..end].to_owned();
    assert_ne!(
        inner.trim(),
        wrapped.trim(),
        "fixture: the record wraps its hook prompt"
    );
    inner
}

/// The task notification's hook prompt: the record's whole content.
fn notification_hook_prompt() -> String {
    content("task-notification")
}

/// Every record of the fixture, the way the reader produces them.
fn records() -> Vec<Record> {
    let parser = parser_for(proto::Runtime::Claude).expect("claude has a parser");
    let version = detect_head(parser, FIXTURE.lines()).expect("the fixture is detected");
    let mut cursor = Cursor::default();
    FIXTURE
        .lines()
        .flat_map(|line| parser.record(version, line, &mut cursor))
        .collect()
}

fn store(prompts: &[String]) -> ConversationSet {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let hooks: Vec<ParsedHook> = prompts.iter().flat_map(|p| [prompt(p), stop()]).collect();
    let now = Instant::now();
    for (ts, h) in hooks.iter().enumerate() {
        set.on_hook(
            proto::Runtime::Claude,
            h,
            None,
            ts as u64,
            now,
            Caps::default(),
        );
    }
    set
}

fn root(set: &ConversationSet) -> proto::Conversation {
    set.snapshot(None).expect("the root conversation exists")
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
fn each_reply_lands_on_its_own_injected_turn() {
    let mut set = store(&[
        HUMAN_PROMPT.to_owned(),
        peer_hook_prompt(),
        notification_hook_prompt(),
    ]);
    set.enrich(&records(), Caps::default());
    let conversation = root(&set);

    assert_eq!(
        conversation.turns.len(),
        6,
        "hooks: three prompts, three replies"
    );
    assert_eq!(
        leading_text(&conversation.turns, 1).as_deref(),
        Some(HUMAN_REPLY)
    );
    assert_eq!(
        leading_text(&conversation.turns, 3).as_deref(),
        Some(PEER_REPLY)
    );
    assert_eq!(
        leading_text(&conversation.turns, 5).as_deref(),
        Some(NOTIFICATION_REPLY)
    );
    // The human turn holds its own reply and nothing that came after it.
    let human_texts: usize = conversation.turns[1]
        .blocks
        .iter()
        .filter(|b| matches!(b, Block::Text { .. }))
        .count();
    assert_eq!(human_texts, 1);
    // A contained match keeps the hook's prompt: the harness text is not the prompt.
    assert_eq!(
        leading_text(&conversation.turns, 2),
        Some(peer_hook_prompt())
    );
    assert_eq!(conversation.degraded, None);
}

/// The peer hand-back's hook never fired, so ordinal 1 (the peer record) meets the task
/// notification's `User` turn. That is Misaligned, never the peer's reply on the wrong
/// turn.
#[test]
fn an_injected_record_whose_hook_never_fired_is_misaligned() {
    let mut set = store(&[HUMAN_PROMPT.to_owned(), notification_hook_prompt()]);
    set.enrich(&records(), Caps::default());
    let conversation = root(&set);

    assert_eq!(conversation.degraded, Some(DegradeReason::Misaligned));
    let texts = every_text(&conversation);
    assert!(
        !texts.iter().any(|t| t.contains("**2 files**")),
        "{texts:?}"
    );
    assert!(
        !texts.iter().any(|t| t.contains("completion event")),
        "{texts:?}"
    );
    assert_eq!(
        leading_text(&conversation.turns, 1).as_deref(),
        Some(HUMAN_REPLY)
    );
    assert_eq!(leading_text(&conversation.turns, 3), None);
}

fn injected(ordinal: u32, text: &str, human: bool) -> Record {
    Record::UserText {
        session_id: None,
        ordinal,
        text: text.to_owned(),
        human,
    }
}

/// Containment is allowed for an injected prompt only, and only around a non-empty hook
/// prompt: a typed prompt still needs the exact text, and an empty hook prompt is
/// contained in anything.
#[test]
fn containment_aligns_only_an_injected_record_around_a_non_empty_hook_prompt() {
    for (hook_prompt, record, human, aligned) in [
        (
            "<m>report</m>",
            "wrapper <m>report</m> wrapper",
            false,
            true,
        ),
        (
            "<m>report</m>",
            "wrapper <m>report</m> wrapper",
            true,
            false,
        ),
        ("", "wrapper", false, false),
        ("  ", "wrapper", false, false),
        (
            "<m>other</m>",
            "wrapper <m>report</m> wrapper",
            false,
            false,
        ),
    ] {
        let mut set = store(&[hook_prompt.to_owned()]);
        set.enrich(
            &[injected(0, record, human), said(0, "the reply")],
            Caps::default(),
        );
        let conversation = root(&set);
        let case = format!("{hook_prompt:?} in {record:?}, human {human}");
        if aligned {
            assert_eq!(
                leading_text(&conversation.turns, 1).as_deref(),
                Some("the reply"),
                "{case}"
            );
            assert_eq!(conversation.degraded, None, "{case}");
        } else {
            assert_eq!(leading_text(&conversation.turns, 1), None, "{case}");
            assert_eq!(
                conversation.degraded,
                Some(DegradeReason::Misaligned),
                "{case}"
            );
        }
    }
}
