//! Tests for `build::apply`, the pure hook-to-turn transform (task M6.5.5), covering
//! every row of the per-`HookKind` table except `PostToolUse`'s matcher, which has its
//! own file (`match_tool_call_tests.rs`) -- see that file's header for why. Every
//! fixture's `session_id`, `agent_id` and `tool_use_id` use the brief's distinct values
//! (`"sess-a"`, `"agent-b"`, `"tu-c"`) where a test needs one at all, `window_id: 4`
//! throughout, and timestamps a second apart starting at `1000` -- so a bug that
//! transposes two same-typed fields (id for name, parent for child, ...) fails a test
//! instead of passing by coincidence.

use super::test_support::*;
use super::*;
use crate::conversation::ConversationSet;
use serde_json::json;
use std::time::Duration;

/// Wave-1 review finding F3, highest priority: `SessionStart` writes two same-typed
/// `Option<String>` fields (`session_id`, `transcript_path`) and nothing previously
/// asserted either by name -- a transposition between them was green, and so was
/// deleting the whole arm's body in favor of an unconditional `false`. Distinct values,
/// asserted by field name, close both: `transcript_path` is what task M6.5.12's watcher
/// opens and `session_id` is what `Conversation.session_id` reports, so a transposition
/// here would ship a watcher pointed at a session id.
#[test]
fn session_start_records_session_id_and_transcript_path_by_name() {
    let mut draft = draft();
    let now = Instant::now();
    let mut start = hook(HookKind::SessionStart);
    start.session_id = Some("sess-a".into());
    start.transcript_path = Some("/logs/agents/sess-a.jsonl".into());

    assert!(run(&mut draft, &start, 1000, now));
    assert_eq!(draft.session_id.as_deref(), Some("sess-a"));
    assert_eq!(
        draft.transcript_path.as_deref(),
        Some("/logs/agents/sess-a.jsonl")
    );

    // A second SessionStart with the same values is not a change.
    assert!(!run(&mut draft, &start, 1001, now));
    // No turn is ever created by SessionStart.
    assert!(draft.turns.is_empty());
}

#[test]
fn a_prompt_closes_the_previous_turn_and_opens_an_assistant_one() {
    let mut draft = draft();
    let now = Instant::now();

    assert!(run(&mut draft, &prompt("refactor the parser"), 1000, now));
    assert!(run(&mut draft, &prompt("now write the tests"), 1001, now));

    assert_eq!(draft.turns.len(), 4);
    let roles: Vec<_> = draft.turns.iter().map(|t| t.role).collect();
    assert_eq!(
        roles,
        vec![Role::User, Role::Assistant, Role::User, Role::Assistant]
    );
    assert_eq!(draft.turns[1].state, TurnState::Complete);
    assert_eq!(draft.turns[3].state, TurnState::Running);

    let text = |turn: &proto::Turn| match &turn.blocks[0] {
        Block::Text { text } => text.clone(),
        other => panic!("expected a Text block, got {other:?}"),
    };
    assert_eq!(text(&draft.turns[0]), "refactor the parser");
    assert_eq!(text(&draft.turns[2]), "now write the tests");
}

#[test]
fn a_tool_call_is_pending_until_its_post_hook() {
    let mut draft = draft();
    let now = Instant::now();

    let mut pre = pre(Some("tu-1"), "Grep");
    pre.tool_input = Some(json!({"pattern": "parse_"}));
    assert!(run(&mut draft, &pre, 1000, now));

    let open = draft.open_turn_index().unwrap();
    let (_, _, state, result) = tool_call(&draft.turns[open].blocks[0]);
    assert_eq!(*state, ToolState::Pending);
    assert!(result.is_none());
    let Block::ToolCall {
        duration_ms,
        summary,
        ..
    } = &draft.turns[open].blocks[0]
    else {
        unreachable!()
    };
    assert!(duration_ms.is_none());
    assert_eq!(summary, "\"parse_\"");

    let mut post = post(Some("tu-1"), None);
    post.tool_response = Some(json!("34 matches"));
    let later = now + Duration::from_secs(1);
    assert!(run(&mut draft, &post, 1001, later));

    let (_, _, state, result) = tool_call(&draft.turns[open].blocks[0]);
    assert_eq!(*state, ToolState::Ok);
    assert_eq!(result.as_ref().unwrap().summary, "34 matches");
    let Block::ToolCall { duration_ms, .. } = &draft.turns[open].blocks[0] else {
        unreachable!()
    };
    assert!(duration_ms.is_some());
}

/// Wave-1 review finding F8: `PreToolUse`'s `input` passthrough and its `"tool"` name
/// default (when `hook.tool_name` is absent) were read by no test -- deleting either
/// (`input: None` regardless of the hook, or `hook.tool_name.clone().unwrap_or_default()`
/// producing `""` instead of `"tool"`) was green.
#[test]
fn pre_tool_use_records_input_and_defaults_an_absent_name_to_tool() {
    let mut draft = draft();
    let now = Instant::now();
    let mut untitled = hook(HookKind::PreToolUse);
    untitled.tool_use_id = Some("tu-c".into());
    untitled.tool_input = Some(json!({"path": "/tmp/example.rs"}));
    // `tool_name` deliberately left `None`.
    run(&mut draft, &untitled, 1000, now);

    let open = draft.open_turn_index().unwrap();
    let Block::ToolCall { name, input, .. } = &draft.turns[open].blocks[0] else {
        panic!(
            "expected a ToolCall block, got {:?}",
            draft.turns[open].blocks[0]
        );
    };
    assert_eq!(name, "tool");
    assert_eq!(input, &Some(json!({"path": "/tmp/example.rs"})));
}

/// Wave-1 review finding F5: "then to the first line" was read by no test --
/// substituting the whole capped string for just its first line was green.
#[test]
fn only_the_first_line_of_a_multiline_result_is_kept() {
    let mut draft = draft();
    let now = Instant::now();
    run(&mut draft, &pre(Some("tu-c"), "Bash"), 1000, now);

    let mut post = post(Some("tu-c"), None);
    post.tool_response = Some(json!("first line\nsecond line\nthird line"));
    run(&mut draft, &post, 1001, now);

    let open = draft.open_turn_index().unwrap();
    let (_, _, _, result) = tool_call(&draft.turns[open].blocks[0]);
    assert_eq!(result.as_ref().unwrap().summary, "first line");
}

/// Wave-1 review finding F8: `detail: None` -- "only enrichment supplies detail" -- was
/// read by no test.
#[test]
fn a_completed_tool_calls_result_never_carries_a_detail() {
    let mut draft = draft();
    let now = Instant::now();
    run(&mut draft, &pre(Some("tu-c"), "Bash"), 1000, now);

    let mut post = post(Some("tu-c"), None);
    post.tool_response = Some(json!("output"));
    run(&mut draft, &post, 1001, now);

    let open = draft.open_turn_index().unwrap();
    let (_, _, _, result) = tool_call(&draft.turns[open].blocks[0]);
    assert_eq!(result.as_ref().unwrap().detail, None);
}

/// Wave-1 review finding F11: `apply`'s `runtime` parameter updates `Draft::runtime`,
/// but nothing asserted it was ever read -- deleting `draft.runtime = runtime;`
/// entirely was green. This `ConversationSet` is created with `Runtime::Claude`; a hook
/// applied with `Runtime::Codex` must still make the resulting `Conversation.runtime`
/// report `Codex`, proving the per-call parameter -- not the set's own stored value --
/// is what actually lands on the draft. (I kept the parameter rather than dropping it
/// as redundant: `build::apply`'s signature is the documented milestone interface, and
/// a freshly created child `Draft`, per `ConversationSet::on_hook`'s `SubagentStart`
/// case, is seeded before any hook has told it what runtime it belongs to -- so the
/// per-call value is the only source of truth available at that point, not a
/// duplicate of one already known.)
#[test]
fn apply_updates_the_drafts_runtime_from_the_per_call_argument() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    spawn_hook(&mut set, &hook(HookKind::SessionStart), None, 1000, now);
    assert_eq!(set.snapshot(None).unwrap().runtime, proto::Runtime::Claude);

    let codex_hook = hook(HookKind::SessionStart);
    set.on_hook(
        proto::Runtime::Codex,
        &codex_hook,
        None,
        1001,
        now,
        Caps::default(),
    );
    assert_eq!(set.snapshot(None).unwrap().runtime, proto::Runtime::Codex);
}

#[test]
fn a_failed_tool_is_marked_failed() {
    let cases = [
        (json!({"error": "no such file"}), false),
        (json!({"success": false}), false),
        (json!({"error": null}), true),
        (json!({"stdout": "fine"}), true),
    ];
    for (index, (response, expected_ok)) in cases.into_iter().enumerate() {
        let mut draft = draft();
        let now = Instant::now();
        let id = format!("tu-{index}");
        run(&mut draft, &pre(Some(&id), "Bash"), 1000, now);

        let mut post = post(Some(&id), None);
        post.tool_response = Some(response);
        run(&mut draft, &post, 1001, now);

        let open = draft.open_turn_index().unwrap();
        let (_, _, state, result) = tool_call(&draft.turns[open].blocks[0]);
        assert_eq!(result.as_ref().unwrap().ok, expected_ok, "case {index}");
        assert_eq!(
            *state,
            if expected_ok {
                ToolState::Ok
            } else {
                ToolState::Failed
            },
            "case {index}"
        );
    }
}

#[test]
fn an_over_limit_object_result_is_still_read_by_its_keys() {
    let mut draft = draft();
    let now = Instant::now();
    run(&mut draft, &pre(Some("tu-c"), "Bash"), 1000, now);

    let mut post = post(Some("tu-c"), None);
    post.tool_response = Some(json!({"error": "boom"}));
    post.tool_result_truncated = Some(true);
    run(&mut draft, &post, 1001, now);

    let open = draft.open_turn_index().unwrap();
    let (_, _, state, result) = tool_call(&draft.turns[open].blocks[0]);
    assert!(!result.as_ref().unwrap().ok);
    assert_eq!(*state, ToolState::Failed);
}

#[test]
fn a_stringified_non_object_result_is_unaffected_by_the_flag() {
    let mut draft = draft();
    let now = Instant::now();
    run(&mut draft, &pre(Some("tu-c"), "Bash"), 1000, now);

    let mut post = post(Some("tu-c"), None);
    post.tool_response = Some(json!("[0,1,2,...]"));
    post.tool_result_stringified = Some(true);
    run(&mut draft, &post, 1001, now);

    let open = draft.open_turn_index().unwrap();
    let (_, _, _, result) = tool_call(&draft.turns[open].blocks[0]);
    assert!(result.as_ref().unwrap().ok);
}

#[test]
fn stop_denies_every_pending_tool() {
    let mut draft = draft();
    let now = Instant::now();
    run(&mut draft, &pre(Some("tu-9"), "Bash"), 1000, now);
    run(&mut draft, &pre(Some("tu-10"), "Edit"), 1001, now);

    let mut done = post(Some("tu-9"), None);
    done.tool_response = Some(json!("ran"));
    run(&mut draft, &done, 1002, now);

    run(&mut draft, &hook(HookKind::Stop), 1003, now);

    let turn = draft.turns.last().unwrap();
    assert_eq!(turn.state, TurnState::Complete);
    let states: Vec<_> = turn.blocks.iter().map(|b| *tool_call(b).2).collect();
    assert_eq!(states, vec![ToolState::Ok, ToolState::Denied]);
}

#[test]
fn a_truncated_hook_result_is_flagged() {
    let cases = [(Some(true), true), (Some(false), false), (None, false)];
    for (index, (flag, expected)) in cases.into_iter().enumerate() {
        let mut draft = draft();
        let now = Instant::now();
        let id = format!("tu-{index}");
        run(&mut draft, &pre(Some(&id), "Bash"), 1000, now);

        let mut post = post(Some(&id), None);
        post.tool_response = Some(json!("short"));
        post.tool_result_truncated = flag;
        run(&mut draft, &post, 1001, now);

        let open = draft.open_turn_index().unwrap();
        let (_, _, _, result) = tool_call(&draft.turns[open].blocks[0]);
        assert_eq!(result.as_ref().unwrap().truncated, expected, "case {index}");
    }
}

/// Wave-1 review finding F4: the original fixture (64 identical `'z'`s, capped to 16
/// bytes) was green even for two real bugs -- truncating to a single byte, and deleting
/// the char-boundary backoff in `cap_bytes` entirely -- because a run of one repeated
/// byte carries no positional information (its unit size, 1, divides every candidate
/// cap). This is the fourth instance of "the fixture's period divides the bound" on
/// this project. The fixed fixture is 15 ASCII bytes followed by three 3-byte "€"
/// characters (24 bytes, 18 chars total): `max_result_bytes: 16` falls *inside* the
/// first "€"'s encoding, so the exact expected output -- the 15-byte ASCII prefix, not
/// 16 bytes and not 1 -- can only be produced by backing off to the nearest earlier
/// char boundary. Deleting that backoff does not silently pass a weaker assertion here;
/// it panics (`s[..16]` on a non-char-boundary), which this test's own passing is proof
/// against.
#[test]
fn a_long_hook_result_is_capped_by_max_result_bytes() {
    let mut draft = draft();
    let now = Instant::now();
    run(&mut draft, &pre(Some("tu-c"), "Bash"), 1000, now);

    let mut post = post(Some("tu-c"), None);
    post.tool_response = Some(json!(format!("{}€€€", "z".repeat(15))));
    let caps = Caps {
        max_result_bytes: 16,
        ..Caps::default()
    };
    run_capped(&mut draft, &post, 1001, now, caps);

    let open = draft.open_turn_index().unwrap();
    let (_, _, _, result) = tool_call(&draft.turns[open].blocks[0]);
    let result = result.as_ref().unwrap();
    assert_eq!(result.summary, "z".repeat(15));
    assert!(result.truncated);
}

/// Simulates `hook.tool_response`'s string content already having its *last* grapheme
/// cluster split by `anthrex hook`'s own char-boundary (not grapheme-boundary)
/// truncation: a family-emoji base character (`U+1F468`, "man") followed by the lone
/// ZWJ that would have joined it to the next member, with that next member itself
/// removed (as if it had been cut away) -- placed right at grapheme 80 (79 plain "z"
/// graphemes before it, 20 more after), so it lands exactly on `truncate_graphemes`'s
/// cut boundary rather than being dropped entirely by it.
///
/// `summary.rs`'s own `long_summaries_truncate_by_grapheme` already establishes the
/// correct total for an ordinary over-the-cap string: `SUMMARY_MAX_GRAPHEMES` content
/// graphemes plus one appended `"…"` marker, i.e. 81. This test's claim is that a
/// malformed trailing cluster right at that boundary does not push the total *past*
/// 81 -- it must not panic, and it must not silently count as more than one grapheme
/// of its own just because it can no longer merge back into a base character whose
/// continuation was already cut away.
#[test]
fn a_split_grapheme_cluster_in_the_result_does_not_panic_or_overrun_the_cap() {
    let split = format!("{}\u{1F468}\u{200D}{}", "z".repeat(79), "z".repeat(20));
    let mut draft = draft();
    let now = Instant::now();
    run(&mut draft, &pre(Some("tu-c"), "Bash"), 1000, now);

    let mut post = post(Some("tu-c"), None);
    post.tool_response = Some(json!(split));
    run(&mut draft, &post, 1001, now);

    let open = draft.open_turn_index().unwrap();
    let (_, _, _, result) = tool_call(&draft.turns[open].blocks[0]);
    let summary = &result.as_ref().unwrap().summary;
    let grapheme_count =
        unicode_segmentation::UnicodeSegmentation::graphemes(summary.as_str(), true).count();
    assert!(
        grapheme_count <= summary::SUMMARY_MAX_GRAPHEMES + 1,
        "grapheme_count = {grapheme_count}"
    );
}

#[test]
fn a_permission_request_is_a_notice_not_a_tool() {
    let mut draft = draft();
    let now = Instant::now();
    let mut permission = hook(HookKind::PermissionRequest);
    permission.tool_name = Some("Bash".into());
    run(&mut draft, &permission, 1000, now);

    let open = draft.open_turn_index().unwrap();
    assert_eq!(draft.turns[open].blocks.len(), 1);
    let Block::Notice { kind, text } = &draft.turns[open].blocks[0] else {
        panic!(
            "expected a Notice block, got {:?}",
            draft.turns[open].blocks[0]
        );
    };
    assert_eq!(*kind, NoticeKind::PermissionRequest);
    assert_eq!(text, "Bash needs permission");
}

#[test]
fn a_notification_that_is_not_a_permission_prompt_does_nothing() {
    let mut draft = draft();
    let now = Instant::now();
    run(&mut draft, &prompt("looks good"), 1000, now);
    let before = draft.to_conversation(0, None, 0, None);

    let mut notification = hook(HookKind::Notification);
    notification.notification_type = Some("idle_prompt".into());
    let changed = run(&mut draft, &notification, 1001, now);

    assert!(!changed);
    assert_eq!(draft.to_conversation(0, None, 0, None), before);
}

#[test]
fn a_subagent_spawn_lands_in_the_parent_and_opens_the_child() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();

    spawn_hook(&mut set, &prompt("investigate the crash"), None, 1000, now);

    let mut start = hook(HookKind::SubagentStart);
    start.agent_id = Some("agent-b".into());
    start.agent_type = Some("Explore".into());
    let changed = spawn_hook(&mut set, &start, None, 1001, now);
    assert_eq!(changed, vec![None, Some("agent-b".to_string())]);

    let parent = set.snapshot(None).unwrap();
    assert_eq!(parent.agent_id, None);
    assert_eq!(
        parent.turns.len(),
        2,
        "the User turn is still there, untouched"
    );
    assert_eq!(parent.turns[1].role, Role::Assistant);
    assert_eq!(parent.turns[1].state, TurnState::Running);
    assert_eq!(parent.turns[1].blocks.len(), 1);
    let Block::SubagentSpawn { agent_id, kind, .. } = &parent.turns[1].blocks[0] else {
        panic!(
            "expected a SubagentSpawn block, got {:?}",
            parent.turns[1].blocks[0]
        );
    };
    assert_eq!(agent_id, "agent-b");
    assert_eq!(kind, "Explore");

    let child = set.snapshot(Some("agent-b")).unwrap();
    assert_eq!(child.agent_id.as_deref(), Some("agent-b"));
    assert_eq!(child.turns.len(), 1);
    assert_eq!(child.turns[0].role, Role::Assistant);
    assert_eq!(child.turns[0].state, TurnState::Running);
    assert!(child.turns[0].blocks.is_empty());
}

/// Wave-1 review finding F9: the parent-side effect of a `SubagentStart` -- appending
/// the `SubagentSpawn` block -- must happen even when `hook.agent_id` is absent (a
/// malformed or degraded hook). Only the *child* side, which genuinely needs a real id
/// to key a new conversation, is conditional on it. Before this fix, a missing
/// `agent_id` skipped the parent notification entirely, so the parent's conversation
/// never learned a sub-agent had started at all.
#[test]
fn a_subagent_start_without_an_agent_id_still_notifies_the_parent() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();

    let mut start = hook(HookKind::SubagentStart);
    start.agent_type = Some("Explore".into());
    // `agent_id` deliberately left `None`.
    let changed = spawn_hook(&mut set, &start, None, 1000, now);
    assert_eq!(changed, vec![None]);

    let parent = set.snapshot(None).unwrap();
    assert_eq!(parent.turns.len(), 1);
    let Block::SubagentSpawn { agent_id, kind, .. } = &parent.turns[0].blocks[0] else {
        panic!(
            "expected a SubagentSpawn block, got {:?}",
            parent.turns[0].blocks[0]
        );
    };
    assert_eq!(agent_id, "");
    assert_eq!(kind, "Explore");

    // No child conversation was created: there was no id to key one by.
    assert_eq!(set.keys(), vec![None]);
}

#[test]
fn a_grandchild_spawn_lands_in_its_own_parent() {
    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();

    let mut start_b = hook(HookKind::SubagentStart);
    start_b.agent_id = Some("agent-b".into());
    start_b.agent_type = Some("Explore".into());
    spawn_hook(&mut set, &start_b, None, 1000, now);

    let mut start_c = hook(HookKind::SubagentStart);
    start_c.agent_id = Some("agent-c".into());
    start_c.agent_type = Some("Plan".into());
    let changed = spawn_hook(&mut set, &start_c, Some("agent-b"), 1001, now);
    assert_eq!(
        changed,
        vec![Some("agent-b".to_string()), Some("agent-c".to_string())]
    );

    // The root conversation only ever received agent-b's own spawn block.
    let root = set.snapshot(None).unwrap();
    assert_eq!(root.turns.len(), 1);
    assert_eq!(root.turns[0].blocks.len(), 1);

    let parent_b = set.snapshot(Some("agent-b")).unwrap();
    assert_eq!(parent_b.turns.len(), 1);
    assert_eq!(parent_b.turns[0].blocks.len(), 1);
    let Block::SubagentSpawn { agent_id, kind, .. } = &parent_b.turns[0].blocks[0] else {
        panic!(
            "expected a SubagentSpawn block, got {:?}",
            parent_b.turns[0].blocks[0]
        );
    };
    assert_eq!(agent_id, "agent-c");
    assert_eq!(kind, "Plan");

    let child_c = set.snapshot(Some("agent-c")).unwrap();
    assert_eq!(child_c.turns.len(), 1);
    assert!(child_c.turns[0].blocks.is_empty());
}

#[test]
fn every_hook_kind_is_handled() {
    let kinds = [
        HookKind::SessionStart,
        HookKind::UserPromptSubmit,
        HookKind::PreToolUse,
        HookKind::PostToolUse,
        HookKind::PermissionRequest,
        HookKind::Notification,
        HookKind::Stop,
        HookKind::SubagentStart,
        HookKind::SubagentStop,
        HookKind::SessionEnd,
        HookKind::TurnComplete,
    ];
    assert_eq!(
        kinds.len(),
        11,
        "every HookKind variant must be covered here"
    );

    let mut set = ConversationSet::new(4, proto::Runtime::Claude);
    let now = Instant::now();
    for (index, kind) in kinds.into_iter().enumerate() {
        let mut h = hook(kind);
        h.session_id = Some("sess-a".into());
        h.tool_name = Some("Bash".into());
        h.tool_use_id = Some(format!("tu-{index}"));
        h.notification_type = Some("permission_prompt".into());
        h.prompt = Some("go".into());
        if matches!(kind, HookKind::SubagentStart | HookKind::SubagentStop) {
            h.agent_id = Some("agent-loop".into());
            h.agent_type = Some("Explore".into());
        }
        spawn_hook(&mut set, &h, None, 1000 + index as u64, now);
    }

    for key in set.keys() {
        let conversation = set.snapshot(key.as_deref()).unwrap();
        for turn in &conversation.turns {
            assert_ne!(
                turn.role,
                Role::System,
                "no HookKind in this milestone should ever produce a System turn"
            );
        }
    }
}
