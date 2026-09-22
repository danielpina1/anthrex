//! Tests for `ConversationSet` (task M6.5.6): revisions, the delta ring and the caps.
//! Every fixture uses distinct prompt texts, tool ids and tool names within a test
//! (`"p1"`..`"p6"`, `"tu-1"`..`"tu-3"`, `"Bash"`/`"Read"`/`"Grep"`) for the same reason
//! `build_tests.rs`'s header gives: a bug that transposes two same-typed values, or a
//! fixture whose period divides the cap it is meant to exercise, must fail a test here
//! rather than pass by coincidence -- this file drives the cap-trimming and delta-replay
//! logic the milestone's own review history calls out both defect shapes for by name.

use super::*;
use crate::conversation::build;
use std::time::Duration;

fn hook(kind: HookKind) -> ParsedHook {
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
    }
}

fn prompt(text: &str) -> ParsedHook {
    let mut h = hook(HookKind::UserPromptSubmit);
    h.prompt = Some(text.to_owned());
    h
}

fn pre(id: &str, name: &str) -> ParsedHook {
    let mut h = hook(HookKind::PreToolUse);
    h.tool_use_id = Some(id.to_owned());
    h.tool_name = Some(name.to_owned());
    h
}

fn post(id: &str, response: serde_json::Value) -> ParsedHook {
    let mut h = hook(HookKind::PostToolUse);
    h.tool_use_id = Some(id.to_owned());
    h.tool_response = Some(response);
    h
}

fn start(id: &str) -> ParsedHook {
    let mut h = hook(HookKind::SubagentStart);
    h.agent_id = Some(id.to_owned());
    h.agent_type = Some("Explore".into());
    h
}

/// Drives one hook against `set` with no spawn origin and `Caps::default()`, since most
/// fixtures here don't care about either.
fn send(
    set: &mut ConversationSet,
    hook: &ParsedHook,
    ts: u64,
    now: Instant,
) -> Vec<Option<String>> {
    set.on_hook(proto::Runtime::Claude, hook, None, ts, now, Caps::default())
}

fn send_capped(
    set: &mut ConversationSet,
    hook: &ParsedHook,
    ts: u64,
    now: Instant,
    caps: Caps,
) -> Vec<Option<String>> {
    set.on_hook(proto::Runtime::Claude, hook, None, ts, now, caps)
}

/// Fix round 1, finding F1 (review reproduction): `Draft.tool_started` used to be keyed
/// by a `ToolCall` block's index within the open turn's `blocks`. Task M6.5.8's own
/// brief (rule 2) is specified to insert a `Text` block at the *front* of an open
/// `Assistant` turn that has none -- exactly what `enrich::apply` will do for
/// `Record::AssistantText` once task 8 lands. This reproduces that insert by hand
/// (`enrich.rs` does not exist yet) against two pending tools, and asserts the exact
/// durations the review measured: under the old index-keyed scheme, `Bash` reported
/// `Some(100)` ms against a true 1000 ms (picking up `Read`'s start time instead of its
/// own), because the insert shifted every block after it down by one index.
#[test]
fn review_front_insert_two_tools_keeps_correct_durations() {
    let t0 = Instant::now();
    let mut draft = Draft::new(4, None, proto::Runtime::Claude);
    let caps = Caps::default();

    assert!(build::apply(
        &mut draft,
        proto::Runtime::Claude,
        &pre("tu-A", "Bash"),
        None,
        1000,
        t0,
        caps
    ));
    assert!(build::apply(
        &mut draft,
        proto::Runtime::Claude,
        &pre("tu-B", "Read"),
        None,
        1000,
        t0 + Duration::from_millis(900),
        caps
    ));

    // The task-8-style front-insert (rule 2): a `Text` block lands at index 0 of the
    // open turn, shifting both `ToolCall` blocks after it down by one.
    let open = draft.open_turn_index().unwrap();
    draft.turns[open].blocks.insert(
        0,
        proto::Block::Text {
            text: "thinking".into(),
        },
    );

    assert!(build::apply(
        &mut draft,
        proto::Runtime::Claude,
        &post("tu-A", serde_json::json!("ok")),
        None,
        1001,
        t0 + Duration::from_millis(1000),
        caps
    ));
    assert!(build::apply(
        &mut draft,
        proto::Runtime::Claude,
        &post("tu-B", serde_json::json!("ok")),
        None,
        1002,
        t0 + Duration::from_millis(1200),
        caps
    ));

    let open = draft.open_turn_index().unwrap();
    let duration_of = |name: &str| {
        draft.turns[open]
            .blocks
            .iter()
            .find_map(|b| match b {
                proto::Block::ToolCall {
                    name: n,
                    duration_ms,
                    ..
                } if n == name => Some(*duration_ms),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no ToolCall block named {name}"))
    };
    assert_eq!(duration_of("Bash"), Some(1000), "Bash: t0+1000ms - t0");
    assert_eq!(duration_of("Read"), Some(300), "Read: t0+1200ms - t0+900ms");
}

/// The single-pending-tool half of the same reproduction: under the old index-keyed
/// scheme, the front-insert shifted the one `ToolCall` block from index 0 to index 1,
/// so `tool_started.get(&0)` missed entirely and `duration_ms` silently stayed `None`
/// while `state` still reported `Ok`.
#[test]
fn review_front_insert_single_tool_keeps_its_duration() {
    let t0 = Instant::now();
    let mut draft = Draft::new(4, None, proto::Runtime::Claude);
    let caps = Caps::default();

    assert!(build::apply(
        &mut draft,
        proto::Runtime::Claude,
        &pre("tu-A", "Bash"),
        None,
        1000,
        t0,
        caps
    ));

    let open = draft.open_turn_index().unwrap();
    draft.turns[open].blocks.insert(
        0,
        proto::Block::Text {
            text: "thinking".into(),
        },
    );

    assert!(build::apply(
        &mut draft,
        proto::Runtime::Claude,
        &post("tu-A", serde_json::json!("ok")),
        None,
        1001,
        t0 + Duration::from_millis(1000),
        caps
    ));

    let open = draft.open_turn_index().unwrap();
    let proto::Block::ToolCall {
        duration_ms, state, ..
    } = draft.turns[open]
        .blocks
        .iter()
        .find(|b| matches!(b, proto::Block::ToolCall { name, .. } if name == "Bash"))
        .unwrap()
    else {
        panic!("expected a ToolCall block");
    };
    assert_eq!(*state, proto::ToolState::Ok);
    assert_eq!(*duration_ms, Some(1000));
}

#[test]
fn rev_advances_only_on_a_real_change() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();

    send(&mut set, &prompt("p1"), 1000, now);
    assert_eq!(set.snapshot(None).unwrap().rev, 1);

    let mut idle = hook(HookKind::Notification);
    idle.notification_type = Some("idle_prompt".into());
    let changed = send(&mut set, &idle, 1001, now);
    assert_eq!(changed, Vec::<Option<String>>::new());
    assert_eq!(set.snapshot(None).unwrap().rev, 1);

    send(&mut set, &prompt("p2"), 1002, now);
    assert_eq!(set.snapshot(None).unwrap().rev, 2);
}

/// The test that makes the delta path meaningful (per the brief): applying
/// `delta_since`'s patches to a snapshot taken mid-stream must reconstruct exactly what
/// a fresh snapshot at the final revision shows, turn for turn, block for block. A delta
/// that omitted a modified turn -- or replayed stale content for one -- passes nothing
/// weaker than a full equality check here.
#[test]
fn a_delta_replays_to_the_same_state_as_a_snapshot() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();

    send(&mut set, &prompt("p1"), 1000, now); // rev 1
    send(&mut set, &pre("tu-1", "Bash"), 1001, now); // rev 2
    send(
        &mut set,
        &post("tu-1", serde_json::json!("out-1")),
        1002,
        now,
    ); // rev 3
    let at_rev_3 = set.snapshot(None).unwrap();
    assert_eq!(at_rev_3.rev, 3);

    send(&mut set, &hook(HookKind::Stop), 1003, now); // rev 4
    send(&mut set, &prompt("p2"), 1004, now); // rev 5
    send(&mut set, &pre("tu-2", "Read"), 1005, now); // rev 6
    send(
        &mut set,
        &post("tu-2", serde_json::json!("out-2")),
        1006,
        now,
    ); // rev 7
    send(&mut set, &hook(HookKind::Stop), 1007, now); // rev 8
    send(&mut set, &prompt("p3"), 1008, now); // rev 9
    send(&mut set, &pre("tu-3", "Grep"), 1009, now); // rev 10
    send(
        &mut set,
        &post("tu-3", serde_json::json!("out-3")),
        1010,
        now,
    ); // rev 11
    send(&mut set, &hook(HookKind::Stop), 1011, now); // rev 12

    let final_snapshot = set.snapshot(None).unwrap();
    assert_eq!(final_snapshot.rev, 12);

    let (to_rev, patches) = set.delta_since(None, 3).unwrap();
    assert_eq!(to_rev, 12);

    let mut replayed = at_rev_3;
    for patch in patches {
        match patch {
            TurnPatch::Upsert(turn) => {
                if let Some(existing) = replayed.turns.iter_mut().find(|t| t.id == turn.id) {
                    *existing = turn;
                } else {
                    replayed.turns.push(turn);
                }
            }
            TurnPatch::Drop { id } => replayed.turns.retain(|t| t.id != id),
        }
    }
    // `rev`/`degraded`/`dropped_turns`/`dropped_by` are not part of `TurnPatch` (decision
    // A4: they are carried directly on `ConversationDelta`, always at their current
    // value, never diffed) -- the client combines them with the replayed `turns` the
    // same way.
    replayed.rev = final_snapshot.rev;
    replayed.degraded = final_snapshot.degraded;
    replayed.dropped_turns = final_snapshot.dropped_turns;
    replayed.dropped_by = final_snapshot.dropped_by;

    assert_eq!(replayed, final_snapshot);
}

#[test]
fn an_old_rev_gets_no_delta() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    for i in 0..70 {
        send(&mut set, &prompt(&format!("p{i}")), 1000 + i as u64, now);
    }
    assert_eq!(set.snapshot(None).unwrap().rev, 70);

    // Fix round 1, F8's related coverage gap: probe the exact 64/65 boundary rather
    // than 68/2, which leaves a 63-revision-wide window in which the condition could be
    // off by one and still pass.
    assert!(
        set.delta_since(None, 5).is_none(),
        "70 - 5 = 65 > DELTA_HISTORY (64)"
    );
    assert!(
        set.delta_since(None, 6).is_some(),
        "70 - 6 = 64 <= DELTA_HISTORY (64)"
    );
}

#[test]
fn a_future_rev_gets_no_delta() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    send(&mut set, &prompt("p1"), 1000, now);
    let rev = set.snapshot(None).unwrap().rev;

    assert!(set.delta_since(None, rev + 1).is_none());
}

/// `caps.max_turns = 4`. Six user-prompt/stop cycles with **distinct** prompt texts
/// `"p1"`..`"p6"`, each pushing exactly 2 turns (a `User` and an `Assistant`) -- plus
/// one deliberately odd-turn-contributing step wedged between cycles 3 and 4: a bare
/// `PreToolUse`/`Stop` pair with no preceding `UserPromptSubmit`, which opens and closes
/// exactly *one* fresh turn on its own.
///
/// Fix round 1, finding F8: without that extra step, every cycle contributes turns in
/// units of 2 against a bound of 4 -- 2 divides 4 -- so `turns.len()` only ever steps
/// `2, 4, 6, 4, 6, 4, ...` before each trim, never landing on an odd value above the
/// cap, and every trim removes exactly two turns. Two mutations (evicting from the back
/// instead of the front; transposing `DropCause::Turns`/`Bytes`) both still fail this
/// fixture even without the extra step, so the periodicity was not hiding a live
/// defect -- but a fixture should not rely on that staying true, so the extra step
/// breaks it: after cycle 3's trim (turns = `[3,4,5,6]`), the extra step pushes one more
/// turn (`turns.len()` reaches 5, not 6) and the trim that follows removes exactly one
/// turn, not two -- the assertion on that specific revision's `Drop` count pins the odd
/// path directly, not just the final counts.
#[test]
fn the_turn_cap_drops_from_the_front_and_counts() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    let caps = Caps {
        max_turns: 4,
        ..Caps::default()
    };

    let cycle = |set: &mut ConversationSet, text: &str, ts: u64| {
        send_capped(set, &prompt(text), ts, now, caps);
        send_capped(set, &hook(HookKind::Stop), ts + 1, now, caps);
    };
    let drops_in = |set: &ConversationSet, from_rev: u64| -> Vec<u64> {
        set.delta_since(None, from_rev)
            .unwrap()
            .1
            .iter()
            .filter_map(|p| match p {
                TurnPatch::Drop { id } => Some(*id),
                _ => None,
            })
            .collect()
    };

    cycle(&mut set, "p1", 1000);
    cycle(&mut set, "p2", 1002);

    let rev_before_p3 = set.snapshot(None).unwrap().rev;
    cycle(&mut set, "p3", 1004);
    assert_eq!(
        drops_in(&set, rev_before_p3),
        vec![1, 2],
        "the revision that trimmed carries a Drop for each removed id"
    );

    let rev_before_extra = set.snapshot(None).unwrap().rev;
    send_capped(&mut set, &pre("tu-extra", "Ping"), 1006, now, caps);
    assert_eq!(
        drops_in(&set, rev_before_extra),
        vec![3],
        "turns.len() reached 5 here, not 6, so exactly one turn -- not two -- was trimmed"
    );
    send_capped(&mut set, &hook(HookKind::Stop), 1007, now, caps);

    cycle(&mut set, "p4", 1008);
    cycle(&mut set, "p5", 1010);
    cycle(&mut set, "p6", 1012);

    let final_snapshot = set.snapshot(None).unwrap();
    assert_eq!(final_snapshot.turns.len(), 4, "the last 4 turns, in order");
    assert_eq!(final_snapshot.dropped_turns, 9, "13 pushed, 4 survive");
    assert_eq!(final_snapshot.dropped_by, Some(proto::DropCause::Turns));

    let text = |turn: &proto::Turn| match &turn.blocks[0] {
        proto::Block::Text { text } => text.clone(),
        other => panic!("expected a Text block, got {other:?}"),
    };
    assert_eq!(text(&final_snapshot.turns[0]), "p5");
    assert_eq!(text(&final_snapshot.turns[2]), "p6");
}

/// `caps.max_bytes` is measured directly off the first (real) turn's own `byte_size` --
/// not a guessed literal -- so the margin is exactly "fits one turn, not two", and
/// `caps.max_turns` is set far out of the way so only the byte predicate can be
/// responsible for the trim.
#[test]
fn the_byte_cap_drops_from_the_front_and_says_so() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();

    // Turn 1: one small `ToolCall`, its own turn (a `PreToolUse` on an empty draft opens
    // a fresh turn), closed so its size is final.
    send(&mut set, &pre("tu-1", "Bash"), 1000, now);
    send(&mut set, &hook(HookKind::Stop), 1001, now);
    let turn_1_size = set.snapshot(None).unwrap().turns[0].byte_size();

    let caps = Caps {
        max_bytes: turn_1_size + 1,
        max_turns: 1000,
        ..Caps::default()
    };

    // Turn 2: a deliberately much larger `ToolCall`, big enough on its own that turn 1 +
    // turn 2 together blow past `caps.max_bytes` even though turn 2 alone still fits.
    let mut big = pre(
        "tu-2",
        "VeryLongToolNameChosenSoThisTurnIsClearlyBiggerThanTurnOne",
    );
    big.tool_input = Some(serde_json::json!({
        "pattern": "x".repeat(200),
        "path": "y".repeat(200),
    }));
    send_capped(&mut set, &big, 1002, now, caps);

    let after_trim = set.snapshot(None).unwrap();
    assert_eq!(
        after_trim.turns.len(),
        1,
        "turn 1 was dropped the moment turn 2 pushed the total over max_bytes"
    );
    assert_eq!(after_trim.dropped_turns, 1);
    assert_eq!(after_trim.dropped_by, Some(proto::DropCause::Bytes));
    assert!(
        after_trim.turns[0].byte_size() > turn_1_size,
        "the surviving turn is turn 2, the bigger one"
    );
}

#[test]
fn a_single_oversize_turn_is_kept() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    let caps = Caps {
        max_bytes: 10,
        ..Caps::default()
    };

    let mut big = pre("tu-1", "Bash");
    big.tool_input = Some(serde_json::json!({"pattern": "z".repeat(500)}));
    send_capped(&mut set, &big, 1000, now, caps);

    let snapshot = set.snapshot(None).unwrap();
    assert_eq!(snapshot.turns.len(), 1, "never trimmed to zero turns");
    assert_eq!(snapshot.dropped_turns, 0);
    assert!(set.oversize(None));
}

#[test]
fn degradation_changes_the_rev_and_is_reported_once() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    send(&mut set, &prompt("p1"), 1000, now);
    assert_eq!(set.snapshot(None).unwrap().rev, 1);

    let (changed, is_new) = set.set_degraded(Some(proto::DegradeReason::Unreadable));
    assert_eq!(changed, vec![None]);
    assert!(is_new);
    let after_first = set.snapshot(None).unwrap();
    assert_eq!(after_first.rev, 2);
    assert_eq!(after_first.degraded, Some(proto::DegradeReason::Unreadable));

    let (changed, is_new) = set.set_degraded(Some(proto::DegradeReason::Unreadable));
    assert!(changed.is_empty());
    assert!(!is_new);
    assert_eq!(set.snapshot(None).unwrap().rev, 2, "no change, no revision");

    let (changed, is_new) = set.set_degraded(Some(proto::DegradeReason::UnknownFormat));
    assert_eq!(changed, vec![None]);
    assert!(is_new);
    let after_third = set.snapshot(None).unwrap();
    assert_eq!(after_third.rev, 3);
    assert_eq!(
        after_third.degraded,
        Some(proto::DegradeReason::UnknownFormat)
    );

    let (changed, is_new) = set.set_degraded(None);
    assert_eq!(changed, vec![None]);
    assert!(is_new);
    let after_fourth = set.snapshot(None).unwrap();
    assert_eq!(after_fourth.rev, 4);
    assert_eq!(after_fourth.degraded, None);
}

#[test]
fn conversations_are_capped_by_count() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    let total = MAX_CONVERSATIONS_PER_WINDOW + 5;

    for i in 0..total {
        let id = format!("agent-{i}");
        set.on_hook(
            proto::Runtime::Claude,
            &start(&id),
            None,
            1000 + i as u64,
            now,
            Caps::default(),
        );
    }

    assert_eq!(set.keys().len(), MAX_CONVERSATIONS_PER_WINDOW);

    let root = set.snapshot(None).unwrap();
    let spawned_ids: Vec<String> = root
        .turns
        .iter()
        .flat_map(|t| t.blocks.iter())
        .filter_map(|b| match b {
            proto::Block::SubagentSpawn { agent_id, .. } => Some(agent_id.clone()),
            _ => None,
        })
        .collect();

    // The root got a spawn block for every one of the `total` hooks (the parent-side
    // effect always runs), but only the first `MAX_CONVERSATIONS_PER_WINDOW - 1` (the
    // cap minus the root's own slot) got their own conversation.
    assert_eq!(spawned_ids.len(), total);
    for i in 0..(MAX_CONVERSATIONS_PER_WINDOW - 1) {
        let id = format!("agent-{i}");
        assert!(
            set.snapshot(Some(&id)).is_some(),
            "{id} should have its own conversation"
        );
    }
    for i in (MAX_CONVERSATIONS_PER_WINDOW - 1)..total {
        let id = format!("agent-{i}");
        assert!(
            set.snapshot(Some(&id)).is_none(),
            "{id} is beyond the cap and has no conversation of its own"
        );
        assert!(
            spawned_ids.contains(&id),
            "{id}'s spawn block still landed in the root conversation"
        );
    }
}

#[test]
fn each_conversation_has_its_own_rev() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    send(&mut set, &prompt("root only line"), 1000, now);
    set.on_hook(
        proto::Runtime::Claude,
        &start("agent-b"),
        None,
        1001,
        now,
        Caps::default(),
    );

    let root = set.snapshot(None).unwrap();
    let child = set.snapshot(Some("agent-b")).unwrap();
    assert_ne!(root.rev, child.rev);
    assert_eq!(
        child.rev, 1,
        "a fresh child's first hook is its own first revision"
    );

    // `delta_since` is scoped strictly by key: agent-b's own delta never carries root's
    // content (its `SubagentSpawn` block), and root's own delta never carries agent-b's
    // seed turn in its place.
    let (_, child_patches) = set.delta_since(Some("agent-b"), 0).unwrap();
    for patch in &child_patches {
        if let TurnPatch::Upsert(turn) = patch {
            assert!(
                turn.blocks.is_empty(),
                "agent-b's only turn is its own empty seed turn"
            );
        }
    }
    let (_, root_patches) = set.delta_since(None, 0).unwrap();
    assert!(
        root_patches.iter().any(|p| matches!(
            p,
            TurnPatch::Upsert(t) if t.blocks.iter().any(|b| matches!(b, proto::Block::SubagentSpawn { .. }))
        )),
        "root's own delta carries the spawn block"
    );
}

#[test]
fn caps_come_from_the_config_defaults() {
    let caps = Caps::from_config(&config::Conversation::default());
    assert_eq!(
        caps,
        Caps {
            max_turns: 500,
            max_bytes: 2_097_152,
            max_result_bytes: 16384,
        }
    );
}

/// Task M6.5.6, item 2: `Draft::to_conversation` used to hard-code `rev: 0, degraded:
/// None, dropped_turns: 0, dropped_by: None` with no test asserting any of them. Every
/// value here is distinct from every other of the same type where the type itself
/// doesn't already forbid a transposition (`rev: u64` vs `dropped_turns: u32` cannot be
/// swapped without a compile error; `degraded: Option<DegradeReason>` vs `dropped_by:
/// Option<DropCause>` likewise), so this pins every field by name against a real,
/// non-default value.
#[test]
fn draft_to_conversation_wires_rev_degraded_dropped_turns_and_dropped_by_by_name() {
    let mut draft = Draft::new(7, Some("agent-x".into()), proto::Runtime::Codex);
    draft.push_turn(
        proto::Role::User,
        proto::TurnState::Complete,
        vec![proto::Block::Text { text: "hi".into() }],
        1000,
    );

    let conversation = draft.to_conversation(
        9,
        Some(proto::DegradeReason::TooLarge),
        3,
        Some(proto::DropCause::Bytes),
    );
    assert_eq!(conversation.window_id, 7);
    assert_eq!(conversation.agent_id.as_deref(), Some("agent-x"));
    assert_eq!(conversation.rev, 9);
    assert_eq!(conversation.degraded, Some(proto::DegradeReason::TooLarge));
    assert_eq!(conversation.dropped_turns, 3);
    assert_eq!(conversation.dropped_by, Some(proto::DropCause::Bytes));
}

/// Fix round 1, finding F2 (review reproduction): `conversations_are_capped_by_count`
/// only ever drives `SubagentStart`, whose own `on_hook` arm always resolves the root's
/// key (`None`) first -- creating it as entry #1 before any child -- so that test cannot
/// see a root created *after* `MAX_CONVERSATIONS_PER_WINDOW - 1` named keys already
/// exist. This drives the same number of distinct agent ids on the generic path instead
/// (a `PreToolUse` naming a fresh `agent_id` each time, as a restarted daemon or a
/// dropped `SubagentStart` could deliver in production), which is exactly the path that
/// let the set grow to `MAX_CONVERSATIONS_PER_WINDOW + 1` (52, when the cap is 51)
/// before this fix reserved the root's own slot.
#[test]
fn conversations_are_capped_by_count_via_the_generic_path() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    let total = MAX_CONVERSATIONS_PER_WINDOW + 5;

    for i in 0..total {
        let mut h = pre(&format!("tu-{i}"), "Bash");
        h.agent_id = Some(format!("agent-{i}"));
        send(&mut set, &h, 1000 + i as u64, now);
    }

    assert_eq!(
        set.keys().len(),
        MAX_CONVERSATIONS_PER_WINDOW,
        "the root's own slot must be reserved, never leaving room for a 52nd conversation"
    );
    assert!(
        set.keys().contains(&None),
        "the root must exist within the cap once overflow hooks start redirecting to it"
    );
}

/// Fix round 1, finding F2's folded minor (review reproduction): a hook that changes
/// nothing must not permanently occupy a conversation slot. Before this fix, `ensure_key`
/// created and inserted an entry unconditionally, before the hook's own effect ever ran
/// -- so a stream of `Notification`s the daemon never acts on (anything but
/// `"permission_prompt"`) could each still claim a slot for a distinct `agent_id`,
/// eventually locking out real sub-agent conversations once the cap was reached.
#[test]
fn ignored_hooks_do_not_consume_a_conversation_slot() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    let total = MAX_CONVERSATIONS_PER_WINDOW + 5;

    for i in 0..total {
        let mut h = hook(HookKind::Notification);
        h.notification_type = Some("idle_prompt".into());
        h.agent_id = Some(format!("agent-{i}"));
        let changed = send(&mut set, &h, 1000 + i as u64, now);
        assert!(
            changed.is_empty(),
            "an ignored notification reports no change"
        );
    }

    assert_eq!(
        set.keys().len(),
        0,
        "no conversation should exist for a stream of hooks that changed nothing"
    );
}

/// Fix round 1, finding F3 (review reproduction): deleting `compact_patches`'s one
/// `retain` line left `cargo test -p anthrex-daemon --lib conversation::` at 51/51
/// green, because the client fold applies patches in order and redundant `Upsert`s for
/// the same id are idempotent. This asserts directly on the *compacted patch list*
/// itself, not on replayed state: three `PreToolUse`/`PostToolUse` pairs all land on the
/// same open `Assistant` turn, producing six revisions that each `Upsert` that one turn
/// -- the compacted delta must carry exactly one.
#[test]
fn a_delta_carries_only_the_last_upsert_per_turn() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    send(&mut set, &prompt("p1"), 1000, now);
    let rev_after_prompt = set.snapshot(None).unwrap().rev;

    for i in 0..3u64 {
        let id = format!("tu-{i}");
        send(&mut set, &pre(&id, "Bash"), 1001 + i * 2, now);
        send(
            &mut set,
            &post(&id, serde_json::json!("ok")),
            1002 + i * 2,
            now,
        );
    }

    let (_, patches) = set.delta_since(None, rev_after_prompt).unwrap();
    let ids: Vec<u64> = patches
        .iter()
        .map(|p| match p {
            TurnPatch::Upsert(t) => t.id,
            TurnPatch::Drop { id } => *id,
        })
        .collect();
    let mut sorted = ids.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(
        ids.len(),
        sorted.len(),
        "no turn id appears twice in a compacted delta"
    );
    assert_eq!(
        patches.len(),
        1,
        "all three pre/post pairs land on the same open turn"
    );
}

/// The other half of the compaction rule (review's suggested second case): a turn
/// `Upsert`ed and then `Drop`ped within the same delta window must not carry a stale
/// `Upsert` for it. `max_turns = 2` makes the second `UserPromptSubmit` both modify
/// turn 2 (closing it) and immediately evict turns 1 and 2 in that same revision, while
/// turns 3 and 4 (the new prompt's own pair) survive.
#[test]
fn a_delta_drops_an_upsert_for_a_turn_a_later_drop_removed() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    let caps = Caps {
        max_turns: 2,
        ..Caps::default()
    };
    send_capped(&mut set, &prompt("p1"), 1000, now, caps);
    send_capped(&mut set, &prompt("p2"), 1001, now, caps);

    let (_, patches) = set.delta_since(None, 0).unwrap();
    let upserted_ids: Vec<u64> = patches
        .iter()
        .filter_map(|p| match p {
            TurnPatch::Upsert(t) => Some(t.id),
            _ => None,
        })
        .collect();
    let dropped_ids: Vec<u64> = patches
        .iter()
        .filter_map(|p| match p {
            TurnPatch::Drop { id } => Some(*id),
            _ => None,
        })
        .collect();

    assert_eq!(dropped_ids, vec![1, 2], "turns 1 and 2 were evicted");
    assert!(
        !upserted_ids.contains(&1),
        "turn 1's Upsert must not survive its own Drop"
    );
    assert!(
        !upserted_ids.contains(&2),
        "turn 2's Upsert must not survive its own Drop, even though it was re-upserted \
         (closed) first"
    );
    assert_eq!(upserted_ids, vec![3, 4]);
}

/// Fix round 1, finding F6 (review reproduction): `Entry::new` used to hard-code
/// `degraded: None` and never seed from `ConversationSet::current_degraded`, so a
/// conversation created after `set_degraded` had already run started (and stayed)
/// `None` regardless -- the old code comment's "has not caught up yet" overstated it,
/// since it only ever catches up if `set_degraded` is called *again* for a reason that
/// no longer applies.
#[test]
fn set_degraded_reaches_a_conversation_created_afterward() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    set.set_degraded(Some(proto::DegradeReason::Unreadable));

    let now = Instant::now();
    send(&mut set, &prompt("late prompt"), 1000, now);

    let late = set.snapshot(None).unwrap();
    assert_eq!(late.degraded, Some(proto::DegradeReason::Unreadable));
}

/// Fix round 1, finding F7 (review reproduction): `oversize()` alone gives a caller no
/// way to log "once at `warn`" -- polling it after every hook and logging when true
/// would log on every hook, not once. `take_oversize` reports a transition into
/// oversize exactly once; `oversize()` itself stays a sticky current-state query in
/// between. Also exercises the companion fix (F7's second paragraph): `oversize_logged`
/// is now recomputed fresh on every `enforce_caps` call, so it genuinely resolves once
/// the oversized turn leaves, and a later re-entry into oversize is reported again.
#[test]
fn take_oversize_reports_the_transition_exactly_once() {
    // Measure a normal small turn's size under generous caps first.
    let mut scratch = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    send(&mut scratch, &pre("tu-small", "Bash"), 1000, now);
    send(&mut scratch, &hook(HookKind::Stop), 1001, now);
    let small_size = scratch.snapshot(None).unwrap().turns[0].byte_size();

    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let caps = Caps {
        max_bytes: small_size + 1,
        max_turns: 1000,
        ..Caps::default()
    };

    // Turn 1: oversized alone.
    let mut big = pre("tu-big", "Bash");
    big.tool_input = Some(serde_json::json!({"pattern": "z".repeat(2000)}));
    send_capped(&mut set, &big, 1000, now, caps);
    assert!(set.oversize(None));
    assert!(
        set.take_oversize(None),
        "the first observation after the transition reports true"
    );
    assert!(
        !set.take_oversize(None),
        "a second call before anything changes reports false"
    );
    assert!(
        set.oversize(None),
        "oversize() itself is unaffected by take_oversize -- still a sticky current-state query"
    );

    // Turn 2: a small tool call, identical to the one used to measure `small_size`, so
    // it fits under `caps.max_bytes` once turn 1 is evicted.
    send_capped(&mut set, &hook(HookKind::Stop), 1001, now, caps);
    send_capped(&mut set, &pre("tu-small", "Bash"), 1002, now, caps);

    let snap = set.snapshot(None).unwrap();
    assert_eq!(
        snap.turns.len(),
        1,
        "the oversized turn 1 was evicted, leaving only turn 2"
    );
    assert!(
        !set.oversize(None),
        "oversize resolved once the oversized turn left"
    );

    // Re-enter oversize with a third, big turn.
    send_capped(&mut set, &hook(HookKind::Stop), 1003, now, caps);
    let mut big2 = pre("tu-big2", "Bash");
    big2.tool_input = Some(serde_json::json!({"pattern": "z".repeat(2000)}));
    send_capped(&mut set, &big2, 1004, now, caps);
    assert!(set.oversize(None));
    assert!(
        set.take_oversize(None),
        "a fresh transition into oversize is reported again"
    );
}

/// Fix round 1, finding F9: `transcript_path` needs no `crate::transcript::Record` (the
/// review pointed out the deferral was unnecessary) -- it only reads
/// `Draft.transcript_path`, already populated by `build::session_start`.
#[test]
fn transcript_path_reads_the_root_drafts_own_field() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    assert_eq!(
        set.transcript_path(),
        None,
        "no root conversation exists yet"
    );

    let now = Instant::now();
    let mut start = hook(HookKind::SessionStart);
    start.session_id = Some("sess-a".into());
    start.transcript_path = Some("/logs/agents/sess-a.jsonl".into());
    send(&mut set, &start, 1000, now);

    assert_eq!(set.transcript_path(), Some("/logs/agents/sess-a.jsonl"));
}
