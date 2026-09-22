//! Task M6.5.10's one rule for revisions: no observable change, no new `rev`. One test
//! per case the brief names, each paired with the visible change that must still count,
//! so a rule that simply stopped recording revisions fails too.

use super::tests::{hook, prompt, said, session, stop, user};
use crate::conversation::{Caps, ConversationSet};
use crate::hooks::{HookKind, ParsedHook};
use proto::DegradeReason;
use std::time::Instant;

fn apply(set: &mut ConversationSet, h: &ParsedHook) -> Vec<Option<String>> {
    set.on_hook(
        proto::Runtime::Claude,
        h,
        None,
        0,
        Instant::now(),
        Caps::default(),
    )
}

fn rev(set: &ConversationSet) -> u64 {
    set.rev(None).expect("the root conversation exists")
}

fn session_at(id: &str, path: &str) -> ParsedHook {
    let mut h = session(id);
    h.transcript_path = Some(path.into());
    h
}

/// Task M6.5.6's F4, first half: a `SessionStart` that changes only `transcript_path`
/// is invisible to a client, so it is no revision; the path is still recorded for the
/// reader. A changed `session_id` is visible (it travels on the delta) and still counts.
#[test]
fn a_session_start_that_changes_only_the_transcript_path_is_no_revision() {
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    apply(&mut set, &prompt("hello"));
    let before = rev(&set);

    assert_eq!(
        apply(&mut set, &session_at("sess-a", "/t/a.jsonl")),
        vec![None]
    );
    assert_eq!(rev(&set), before + 1, "a new session id is visible");
    assert_eq!(
        set.visible(None).unwrap().session_id.as_deref(),
        Some("sess-a")
    );

    assert!(apply(&mut set, &session_at("sess-a", "/t/b.jsonl")).is_empty());
    assert_eq!(rev(&set), before + 1, "only the path moved");
    assert_eq!(set.transcript_path(), Some("/t/b.jsonl"));
}

/// The same rule where the root does not exist yet: the entry is kept, because the
/// reader needs its path, but at revision 0 and without reporting a changed key.
#[test]
fn a_path_only_session_start_creates_the_root_without_a_revision() {
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    let mut start = hook(HookKind::SessionStart);
    start.transcript_path = Some("/t/a.jsonl".into());
    assert!(apply(&mut set, &start).is_empty());
    assert_eq!(set.transcript_path(), Some("/t/a.jsonl"));
    assert_eq!(set.rev(None), Some(0));
}

/// Task M6.5.8's concern 2: a restart that re-applies the records the conversation
/// already shows is one step with nothing to see. Different records are a revision.
#[test]
fn a_restart_that_reapplies_the_same_records_is_no_revision() {
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    for h in [prompt("p-zero"), stop(), prompt("p-one"), stop()] {
        apply(&mut set, &h);
    }
    let records = [
        user(0, "p-zero"),
        said(0, "reply zero"),
        user(1, "p-one"),
        said(1, "reply one"),
    ];
    assert_eq!(set.enrich(&records, Caps::default()), vec![None]);
    let enriched = set.snapshot(None).unwrap();

    assert!(set.restart_enrichment(&records, Caps::default()).is_empty());
    assert_eq!(
        set.snapshot(None).unwrap(),
        enriched,
        "same turns, same rev"
    );

    let shorter = &records[..2];
    assert_eq!(
        set.restart_enrichment(shorter, Caps::default()),
        vec![None],
        "the second reply is gone: visible"
    );
    assert_eq!(rev(&set), enriched.rev + 1);
}

/// Task M6.5.8's F4: `Misaligned` set while the reader's own reason is shown changes
/// nothing a client sees. It surfaces, as a revision, once the reader's reason clears.
#[test]
fn a_misalignment_under_the_readers_reason_is_no_revision() {
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    for h in [prompt("p-zero"), stop(), prompt("p-one"), stop()] {
        apply(&mut set, &h);
    }
    set.set_degraded(Some(DegradeReason::BadRecord));
    let before = rev(&set);

    let changed = set.enrich(&[user(0, "not what the hook saw")], Caps::default());
    assert!(changed.is_empty());
    assert_eq!(rev(&set), before);
    assert_eq!(
        set.snapshot(None).unwrap().degraded,
        Some(DegradeReason::BadRecord)
    );

    let (changed, _) = set.set_degraded(None);
    assert_eq!(changed, vec![None]);
    assert_eq!(rev(&set), before + 1);
    assert_eq!(
        set.snapshot(None).unwrap().degraded,
        Some(DegradeReason::Misaligned)
    );
}

/// `set_degraded` follows the same rule: a reader reason that changes underneath an
/// identical visible one (here, the reader naming `Misaligned` while the enricher
/// already shows it) is no revision.
#[test]
fn set_degraded_is_no_revision_when_the_shown_reason_is_unchanged() {
    let mut set = ConversationSet::new(1, proto::Runtime::Claude);
    for h in [prompt("p-zero"), stop()] {
        apply(&mut set, &h);
    }
    set.enrich(&[user(0, "not what the hook saw")], Caps::default());
    assert_eq!(
        set.snapshot(None).unwrap().degraded,
        Some(DegradeReason::Misaligned)
    );
    let before = rev(&set);
    let (changed, _) = set.set_degraded(Some(DegradeReason::Misaligned));
    assert!(changed.is_empty());
    assert_eq!(rev(&set), before);
}

/// A subscriber to a conversation no hook has touched still gets one: revision 0, no
/// turns, the set's current reason, and the same shape a first hook would start from.
#[test]
fn snapshot_or_empty_answers_for_an_untouched_key() {
    let mut set = ConversationSet::new(9, proto::Runtime::Shell);
    set.set_degraded(Some(DegradeReason::NoTranscriptPath));
    let empty = set.snapshot_or_empty(None);
    assert_eq!(empty.window_id, 9);
    assert_eq!(empty.agent_id, None);
    assert_eq!(empty.rev, 0);
    assert!(empty.turns.is_empty());
    assert_eq!(empty.degraded, Some(DegradeReason::NoTranscriptPath));
    assert!(set.snapshot(None).is_none(), "answering creates nothing");
}
