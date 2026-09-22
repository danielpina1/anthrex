//! Tests for `subagent_start`'s use of `SpawnOrigin` (task M6.5.6, item 1 of the task's
//! "three things from task 5" list): before this task, `Block::SubagentSpawn`'s `label`
//! and `model` shipped as `""`/`None` unconditionally, because nothing threaded
//! `SubagentTracker::spawn_origin`'s matched fields into `build::apply`. Split out of
//! `build_tests.rs` (which grew past the repository's ~600-line guideline once these
//! were added) the same way `match_tool_call_tests.rs` was: a narrow, self-contained
//! concern earning its own file rather than being folded into the general per-`HookKind`
//! table.

use super::test_support::*;
use super::*;
use crate::subagents::SpawnOrigin;

/// `kind` deliberately disagrees between `spawn.kind` ("Explore") and `hook.agent_type`
/// ("Plan") so a version that still reads `hook.agent_type` for `kind` -- the
/// pre-task-6 behaviour -- fails this test instead of passing by coincidence; `label`
/// and `model` are distinct non-empty strings so a transposition between the two
/// `Option<String>` fields also fails it.
#[test]
fn subagent_start_fills_label_and_model_from_the_spawn_origin() {
    let mut draft = draft();
    let now = Instant::now();

    let mut start = hook(HookKind::SubagentStart);
    start.agent_id = Some("agent-b".into());
    start.agent_type = Some("Plan".into());
    let origin = SpawnOrigin {
        parent_id: Some("agent-root".into()),
        kind: "Explore".into(),
        label: Some("scout the crash".into()),
        model: Some("claude-opus-4".into()),
    };
    assert!(run_spawn(&mut draft, &start, &origin, 1000, now));

    let open = draft.open_turn_index().unwrap();
    let Block::SubagentSpawn {
        agent_id,
        kind,
        label,
        model,
    } = &draft.turns[open].blocks[0]
    else {
        panic!(
            "expected a SubagentSpawn block, got {:?}",
            draft.turns[open].blocks[0]
        );
    };
    assert_eq!(agent_id, "agent-b");
    assert_eq!(kind, "Explore");
    assert_eq!(label, "scout the crash");
    assert_eq!(model.as_deref(), Some("claude-opus-4"));
}

/// The complementary case: no matched spawn (an id beyond
/// `MAX_CONVERSATIONS_PER_WINDOW`, or a degraded hook the tracker never saw) falls back
/// to `hook.agent_type` for `kind` and to an empty label with no model, exactly as
/// before this task -- so the fallback path is pinned too, not just the happy path.
#[test]
fn subagent_start_falls_back_to_agent_type_when_there_is_no_spawn_origin() {
    let mut draft = draft();
    let now = Instant::now();

    let mut start = hook(HookKind::SubagentStart);
    start.agent_id = Some("agent-b".into());
    start.agent_type = Some("Plan".into());
    assert!(run(&mut draft, &start, 1000, now));

    let open = draft.open_turn_index().unwrap();
    let Block::SubagentSpawn {
        kind, label, model, ..
    } = &draft.turns[open].blocks[0]
    else {
        panic!(
            "expected a SubagentSpawn block, got {:?}",
            draft.turns[open].blocks[0]
        );
    };
    assert_eq!(kind, "Plan");
    assert_eq!(label, "");
    assert_eq!(model, &None);
}
