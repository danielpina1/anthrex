//! Tests for `find_target`, `PostToolUse`'s matcher (task M6.5.5, wave-1 review). Split
//! out of `build_tests.rs` (which covers the rest of `apply`'s per-`HookKind` table) by
//! responsibility: the matcher is the one piece of this transform that reads a
//! runtime-controlled correlation key (`tool_use_id`) across a sequence of hooks that
//! can arrive missing, repeated, out of order or redelivered, and it earned its own
//! file once its coverage gaps (F1, F2, F7 below) were closed -- `build_tests.rs` alone
//! had grown past the repository's ~600-line guideline.
//!
//! Every fixture's ids and tool names are distinct within a test (`tu-1`/`tu-2`/`tu-3`,
//! `Grep`/`Read`/`Bash`, ...) for the same reason as `build_tests.rs`: a matcher bug
//! that reads the wrong field, or picks the wrong end of a search, must fail a test
//! rather than pass by coincidence.

use super::test_support::*;
use super::*;
use serde_json::json;

#[test]
fn post_tool_use_matches_by_id_not_by_position() {
    let mut draft = draft();
    let now = Instant::now();
    for (id, name) in [("tu-1", "Grep"), ("tu-2", "Read"), ("tu-3", "Bash")] {
        run(&mut draft, &pre(Some(id), name), 1000, now);
    }

    let mut post = post(Some("tu-2"), None);
    post.tool_response = Some(json!("done"));
    run(&mut draft, &post, 1001, now);

    let open = draft.open_turn_index().unwrap();
    assert_eq!(
        tool_states(&draft, open),
        vec![ToolState::Pending, ToolState::Ok, ToolState::Pending]
    );
}

/// Two `PreToolUse`s can share a `tool_use_id` (a runtime bug, a retried call, or two
/// distinct tool invocations the runtime happened to number the same). The id tier
/// takes the **last** still-`Pending` match -- so the earlier one is never completed by
/// this `PostToolUse` and stays `Pending` until the turn closes and denies it.
///
/// This is *not* the same rule as the name tier (see `name_fallback_matches_the_
/// oldest_pending_call_first` below): wave-1 review finding F2 established that the
/// **name** tier must search oldest-first, not newest-first -- results arrive FIFO, so
/// for two id-less calls to the same tool the first `Post` belongs to the first `Pre`.
/// The **id** tier keeps "last", because a repeated id is a data anomaly the brief
/// resolves by picking the most recently opened call with that id, not a normal
/// concurrent-calls case the way two id-less same-named calls are.
#[test]
fn a_repeated_tool_use_id_completes_only_the_most_recent_call() {
    let mut draft = draft();
    let now = Instant::now();
    run(&mut draft, &pre(Some("tu-dup"), "Bash"), 1000, now);
    run(&mut draft, &pre(Some("tu-dup"), "Edit"), 1001, now);

    let mut post = post(Some("tu-dup"), None);
    post.tool_response = Some(json!("done"));
    run(&mut draft, &post, 1002, now);

    let open = draft.open_turn_index().unwrap();
    assert_eq!(
        tool_states(&draft, open),
        vec![ToolState::Pending, ToolState::Ok]
    );
}

#[test]
fn post_tool_use_falls_back_to_the_name_when_there_is_no_id() {
    let mut draft = draft();
    let now = Instant::now();
    run(&mut draft, &pre(None, "Read"), 1000, now);
    run(&mut draft, &pre(None, "Bash"), 1001, now);

    let mut post = post(None, Some("Read"));
    post.tool_response = Some(json!("contents"));
    run(&mut draft, &post, 1002, now);

    let open = draft.open_turn_index().unwrap();
    let states: Vec<_> = draft.turns[open]
        .blocks
        .iter()
        .map(|b| {
            let (_, name, state, _) = tool_call(b);
            (name.to_string(), *state)
        })
        .collect();
    assert_eq!(
        states,
        vec![
            ("Read".to_string(), ToolState::Ok),
            ("Bash".to_string(), ToolState::Pending),
        ]
    );
}

/// Wave-1 review finding F1: a `Post` that carries a `tool_use_id` which matches
/// nothing must not fall through to matching by name. `tu-1`'s `Bash` call is the only
/// `Pending` block; a `Post` for the unrelated `tu-999` (also `Bash`) must not complete
/// it -- an explicit id that matches nothing is evidence of a mismatch (a stale retry,
/// a foreign id, a `Post` for a call already denied out of a previous turn), not
/// absence of evidence that would license a name-based guess.
#[test]
fn a_post_with_an_unmatched_id_does_not_fall_back_to_name_matching() {
    let mut draft = draft();
    let now = Instant::now();
    run(&mut draft, &pre(Some("tu-1"), "Bash"), 1000, now);

    let mut post = post(Some("tu-999"), Some("Bash"));
    post.tool_response = Some(json!("output of a different call"));
    let changed = run(&mut draft, &post, 1001, now);

    assert!(!changed);
    let open = draft.open_turn_index().unwrap();
    let (_, _, state, result) = tool_call(&draft.turns[open].blocks[0]);
    assert_eq!(*state, ToolState::Pending);
    assert!(result.is_none());
}

/// Wave-1 review finding F2: two concurrent, id-less calls to the same tool must have
/// their results attached in the order they were opened (FIFO), not newest-first. `A`
/// then `B` are both `Pending` `Bash` calls; `Post("RESULT-OF-A")` must complete `A`,
/// and the following `Post("RESULT-OF-B")` must then complete `B` -- not swap them.
#[test]
fn name_fallback_matches_the_oldest_pending_call_first() {
    let mut draft = draft();
    let now = Instant::now();
    run(&mut draft, &pre(None, "Bash"), 1000, now);
    run(&mut draft, &pre(None, "Bash"), 1001, now);

    let mut post_a = post(None, Some("Bash"));
    post_a.tool_response = Some(json!("RESULT-OF-A"));
    run(&mut draft, &post_a, 1002, now);

    let mut post_b = post(None, Some("Bash"));
    post_b.tool_response = Some(json!("RESULT-OF-B"));
    run(&mut draft, &post_b, 1003, now);

    let open = draft.open_turn_index().unwrap();
    let summaries: Vec<_> = draft.turns[open]
        .blocks
        .iter()
        .map(|b| tool_call(b).3.as_ref().unwrap().summary.clone())
        .collect();
    assert_eq!(summaries, vec!["RESULT-OF-A", "RESULT-OF-B"]);
}

/// Wave-1 review finding F7: a tool completes once. A redelivered `PostToolUse` for an
/// id that already completed (`Ok`) must not rewrite it -- not flip it to `Failed`, and
/// not recompute `duration_ms` -- because hook delivery is not guaranteed unique.
#[test]
fn a_redelivered_post_tool_use_does_not_rewrite_a_completed_call() {
    let mut draft = draft();
    let now = Instant::now();
    run(&mut draft, &pre(Some("tu-c"), "Bash"), 1000, now);

    let mut first = post(Some("tu-c"), None);
    first.tool_response = Some(json!("first result"));
    run(&mut draft, &first, 1001, now);

    let open = draft.open_turn_index().unwrap();
    let (_, _, state, result) = tool_call(&draft.turns[open].blocks[0]);
    assert_eq!(*state, ToolState::Ok);
    assert_eq!(result.as_ref().unwrap().summary, "first result");

    let mut redelivered = post(Some("tu-c"), None);
    redelivered.tool_response = Some(json!({"error": "should never be applied"}));
    let changed = run(&mut draft, &redelivered, 1002, now);

    assert!(!changed);
    let (_, _, state, result) = tool_call(&draft.turns[open].blocks[0]);
    assert_eq!(*state, ToolState::Ok);
    assert_eq!(result.as_ref().unwrap().summary, "first result");
}
