//! Task M6.5.10 fix round 1, review F2: a new session in the same window (Claude's
//! `/clear`) starts a new transcript whose prompts count from 0 again. Its ordinals map
//! onto the new session's own turns, the old session's enrichment stays, and nothing
//! from one session lands on the other's turns.

use super::tests::{prompt, said, session, stop, user};
use crate::conversation::{Caps, ConversationSet};
use crate::hooks::ParsedHook;
use crate::transcript::Record;
use proto::{Block, Role};
use std::time::Instant;

fn apply(set: &mut ConversationSet, hooks: &[ParsedHook]) {
    for h in hooks {
        set.on_hook(
            proto::Runtime::Claude,
            h,
            None,
            0,
            Instant::now(),
            Caps::default(),
        );
    }
}

/// A `SessionStart` onto `path` whose source promises a fresh file (`startup`; `clear`
/// is the same to this code).
fn session_at(id: &str, path: &str) -> ParsedHook {
    sourced(id, path, Some("startup"))
}

fn sourced(id: &str, path: &str, source: Option<&str>) -> ParsedHook {
    let mut h = session(id);
    h.transcript_path = Some(path.into());
    h.session_source = source.map(str::to_owned);
    h
}

fn enrich(set: &mut ConversationSet, records: &[Record]) {
    set.enrich(records, Caps::default());
}

/// `(role, texts)` per turn, in order.
fn shape(set: &ConversationSet) -> Vec<(Role, Vec<String>)> {
    set.snapshot(None)
        .unwrap()
        .turns
        .iter()
        .map(|turn| {
            let texts = turn
                .blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Text { text } => Some(text.clone()),
                    _ => None,
                })
                .collect();
            (turn.role, texts)
        })
        .collect()
}

fn texts(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

/// Session A with one turn, enriched; `/clear`; session B with two turns, whose file
/// counts from 0. All three replies land on their own turns.
fn a_then_b(a_prompt: &str, b_prompts: [&str; 2]) -> ConversationSet {
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    apply(
        &mut set,
        &[session_at("sess-A", "/t/a.jsonl"), prompt(a_prompt), stop()],
    );
    enrich(&mut set, &[user(0, a_prompt), said(0, "A reply 1")]);

    apply(&mut set, &[session_at("sess-B", "/t/b.jsonl")]);
    assert!(set.take_new_session(), "the reader is told to switch files");
    assert!(!set.take_new_session(), "once");
    apply(
        &mut set,
        &[prompt(b_prompts[0]), stop(), prompt(b_prompts[1]), stop()],
    );
    enrich(
        &mut set,
        &[
            user(0, b_prompts[0]),
            said(0, "B reply 1"),
            user(1, b_prompts[1]),
            said(1, "B reply 2"),
        ],
    );
    set
}

#[test]
fn a_new_session_enriches_its_own_turns_and_keeps_the_old_ones() {
    let set = a_then_b("fix the bug", ["hello B", "second B"]);
    assert_eq!(
        shape(&set),
        vec![
            (Role::User, texts(&["fix the bug"])),
            (Role::Assistant, texts(&["A reply 1"])),
            (Role::User, texts(&["hello B"])),
            (Role::Assistant, texts(&["B reply 1"])),
            (Role::User, texts(&["second B"])),
            (Role::Assistant, texts(&["B reply 2"])),
        ]
    );
    assert_eq!(set.snapshot(None).unwrap().degraded, None);
}

/// The misattribution the alignment check exists to prevent: B's first prompt repeats
/// A's, so without the session base B's ordinal 0 would line up with A's turn and B's
/// reply would land on A's.
#[test]
fn a_repeated_first_prompt_does_not_put_the_new_sessions_reply_on_the_old_turn() {
    let set = a_then_b("fix the bug", ["fix the bug", "second B"]);
    assert_eq!(
        shape(&set),
        vec![
            (Role::User, texts(&["fix the bug"])),
            (Role::Assistant, texts(&["A reply 1"])),
            (Role::User, texts(&["fix the bug"])),
            (Role::Assistant, texts(&["B reply 1"])),
            (Role::User, texts(&["second B"])),
            (Role::Assistant, texts(&["B reply 2"])),
        ]
    );
    assert_eq!(set.snapshot(None).unwrap().degraded, None);
}

/// Task M6.5.8's pending buffer across the switch. A's second prompt was read but its
/// hook never came, so it and its reply are parked when B starts; they are dropped, not
/// laid onto B's turns. B's own first prompt, read before its hook, parks and then lands.
#[test]
fn records_parked_across_a_session_switch_land_only_in_their_own_session() {
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    apply(
        &mut set,
        &[session_at("sess-A", "/t/a.jsonl"), prompt("a-one"), stop()],
    );
    enrich(
        &mut set,
        &[
            user(0, "a-one"),
            said(0, "A reply 1"),
            user(1, "a-two"),
            said(1, "A reply 2"),
        ],
    );

    apply(&mut set, &[session_at("sess-B", "/t/b.jsonl")]);
    enrich(&mut set, &[user(0, "b-one"), said(0, "B reply 1")]);
    apply(&mut set, &[prompt("b-one"), stop()]);

    assert_eq!(
        shape(&set),
        vec![
            (Role::User, texts(&["a-one"])),
            (Role::Assistant, texts(&["A reply 1"])),
            (Role::User, texts(&["b-one"])),
            (Role::Assistant, texts(&["B reply 1"])),
        ]
    );
    assert_eq!(set.snapshot(None).unwrap().degraded, None);
}

/// Only a new session is a switch: the same session moving files is the reader's
/// restart (the revision rule's case 1 depends on it), and a first `SessionStart` with no
/// path before it is neither.
#[test]
fn only_a_new_session_with_a_new_file_is_a_switch() {
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    apply(&mut set, &[session_at("sess-A", "/t/a.jsonl")]);
    assert!(!set.take_new_session(), "the first SessionStart");
    apply(&mut set, &[session_at("sess-A", "/t/a2.jsonl")]);
    assert!(!set.take_new_session(), "same session, another file");
    apply(&mut set, &[session("sess-B")]);
    assert!(!set.take_new_session(), "a new session with no path");
}

fn resumed_at(id: &str, path: &str) -> ParsedHook {
    let mut h = session_at(id, path);
    h.session_source = Some("resume".into());
    h
}

/// Fix round 2 (N1/N2) and re-review 2 (M1): only `startup` and `clear` promise a fresh
/// file. Any other source that switches session or file asks for an at-end open, a
/// window's first `SessionStart` included; the same session and file again asks for
/// nothing, whatever the source.
#[test]
fn every_switch_but_startup_and_clear_asks_for_an_at_end_open() {
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    apply(&mut set, &[resumed_at("sess-A", "/t/a.jsonl")]);
    assert!(set.take_at_end(), "a window's first SessionStart, resumed");
    assert!(!set.take_at_end(), "once");
    for source in [
        Some("resume"),
        Some("compact"),
        Some("clear"),
        Some("fork"),
        None,
    ] {
        apply(&mut set, &[sourced("sess-A", "/t/a.jsonl", source)]);
        assert!(!set.take_at_end(), "the same session and file, {source:?}");
    }
    let switches = [
        ("sess-B", Some("clear"), false),
        ("sess-C", Some("startup"), false),
        ("sess-D", Some("fork"), true),
        ("sess-E", Some("compact"), true),
        ("sess-F", Some("resume"), true),
        ("sess-G", Some("something-new"), true),
        ("sess-H", None, true),
    ];
    for (id, source, at_end) in switches {
        apply(&mut set, &[sourced(id, &format!("/t/{id}.jsonl"), source)]);
        assert_eq!(set.take_at_end(), at_end, "{source:?}");
    }
    apply(&mut set, &[resumed_at("sess-A", "/t/a.jsonl")]);
    apply(&mut set, &[session_at("sess-Z", "/t/z.jsonl")]);
    assert!(
        !set.take_at_end(),
        "a later fresh switch replaces an at-end one the reader never saw"
    );
}

/// The ordinal base is taken when the file is opened: the session's ordinal 0 is the
/// window's next `User` turn, even when the resumed prompt repeats an old one.
#[test]
fn an_at_end_open_starts_the_sessions_ordinals_at_the_next_turn() {
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    apply(
        &mut set,
        &[
            session_at("sess-A", "/t/a.jsonl"),
            prompt("continue"),
            stop(),
        ],
    );
    enrich(&mut set, &[user(0, "continue"), said(0, "A reply 1")]);
    apply(&mut set, &[resumed_at("sess-B", "/t/b.jsonl")]);
    set.open_at_end(&[], false, Caps::default());
    apply(&mut set, &[prompt("continue"), stop()]);
    enrich(&mut set, &[user(0, "continue"), said(0, "B reply 1")]);
    assert_eq!(
        shape(&set),
        vec![
            (Role::User, texts(&["continue"])),
            (Role::Assistant, texts(&["A reply 1"])),
            (Role::User, texts(&["continue"])),
            (Role::Assistant, texts(&["B reply 1"])),
        ]
    );
    assert_eq!(set.snapshot(None).unwrap().degraded, None);
}

/// A prompt that arrived after the resume's `SessionStart` and before the open may have
/// its record on either side of the measured end: the session degrades rather than guess.
#[test]
fn an_ambiguous_at_end_open_degrades_instead_of_enriching() {
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    apply(
        &mut set,
        &[
            resumed_at("sess-A", "/t/a.jsonl"),
            prompt("continue"),
            stop(),
        ],
    );
    let changed = set.open_at_end(&[], false, Caps::default());
    assert_eq!(changed, vec![None]);
    enrich(&mut set, &[user(0, "continue"), said(0, "maybe mine")]);
    assert_eq!(
        shape(&set),
        vec![
            (Role::User, texts(&["continue"])),
            (Role::Assistant, texts(&[])),
        ]
    );
    assert_eq!(
        set.snapshot(None).unwrap().degraded,
        Some(proto::DegradeReason::Misaligned)
    );
}
