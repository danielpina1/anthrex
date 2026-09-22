//! The pure transform from one hook into one conversation's `Draft` (task M6.5.5). Hooks
//! are authoritative (spec decision 1): this is the only place that creates, closes or
//! reorders a turn, or sets a `ToolCall`'s state. No I/O, no clock of its own -- every
//! entry point takes `now_unix_secs: u64` and `now: std::time::Instant`.

use super::{Caps, Draft, summary};
use crate::hooks::{HookKind, ParsedHook};
use proto::{Block, NoticeKind, Role, ToolResult, ToolState, TurnState};
use std::time::Instant;

/// Applies one hook to `draft`, per the table in the M6.5.5 brief. Returns whether
/// anything changed.
pub(super) fn apply(
    draft: &mut Draft,
    runtime: proto::Runtime,
    hook: &ParsedHook,
    now_unix_secs: u64,
    now: Instant,
    caps: Caps,
) -> bool {
    // Kept in sync on every hook rather than only at `Draft::new`: a freshly created
    // child conversation (`ConversationSet::on_hook`'s `SubagentStart` case) is seeded
    // before its first hook is known to be for this runtime, so this is the one place
    // that is guaranteed to see every runtime this draft's hooks actually arrive on.
    draft.runtime = runtime;

    match hook.kind {
        HookKind::SessionStart => session_start(draft, hook),
        HookKind::UserPromptSubmit => user_prompt_submit(draft, hook, now_unix_secs),
        HookKind::PreToolUse => pre_tool_use(draft, hook, now_unix_secs, now),
        HookKind::PostToolUse => post_tool_use(draft, hook, now, caps),
        HookKind::PermissionRequest => permission_request(draft, hook, now_unix_secs),
        HookKind::Notification => notification(draft, hook, now_unix_secs),
        HookKind::Stop | HookKind::SessionEnd | HookKind::TurnComplete => draft.close_open_turn(),
        HookKind::SubagentStart => subagent_start(draft, hook, now_unix_secs),
        HookKind::SubagentStop => draft.close_open_turn(),
    }
}

fn session_start(draft: &mut Draft, hook: &ParsedHook) -> bool {
    let mut changed = false;
    if draft.session_id != hook.session_id {
        draft.session_id.clone_from(&hook.session_id);
        changed = true;
    }
    if draft.transcript_path != hook.transcript_path {
        draft.transcript_path.clone_from(&hook.transcript_path);
        changed = true;
    }
    changed
}

fn user_prompt_submit(draft: &mut Draft, hook: &ParsedHook, now_unix_secs: u64) -> bool {
    draft.close_open_turn();
    let text = hook.prompt.clone().unwrap_or_default();
    draft.push_turn(
        Role::User,
        TurnState::Complete,
        vec![Block::Text { text }],
        now_unix_secs,
    );
    draft.push_turn(
        Role::Assistant,
        TurnState::Running,
        Vec::new(),
        now_unix_secs,
    );
    true
}

fn pre_tool_use(draft: &mut Draft, hook: &ParsedHook, now_unix_secs: u64, now: Instant) -> bool {
    let name = hook.tool_name.clone().unwrap_or_else(|| "tool".into());
    let summary = summary::for_tool(&name, hook.tool_input.as_ref());
    let block = Block::ToolCall {
        id: hook.tool_use_id.clone(),
        name,
        summary,
        input: hook.tool_input.clone(),
        result: None,
        state: ToolState::Pending,
        duration_ms: None,
    };
    let index = draft.append_to_open_turn(block, now_unix_secs);
    draft.tool_started.insert(index, now);
    true
}

/// Finds the target `ToolCall` in the open turn.
///
/// When `hook.tool_use_id` is `Some`, the id tier is the *only* tier tried: the last
/// still-`Pending` block whose `id` equals it, or `None` when there is no such block.
/// (Wave-1 review finding F1: an explicit id that matches nothing is evidence of a
/// mismatch, not absence of evidence, so this must never fall through to matching by
/// name -- that would let an unrelated `Post` for a stale or foreign id complete
/// whatever else happens to be `Pending` under the same tool name.) The `Pending` guard
/// (finding F7) also stops a redelivered `PostToolUse` for an id that already completed
/// from rewriting a finished call's result.
///
/// Only when `hook.tool_use_id` is `None` is the name tier tried: the **oldest**
/// still-`Pending` block whose `name` equals `hook.tool_name` (finding F2: results
/// arrive FIFO, so for two concurrent id-less calls to the same tool, the first `Post`
/// belongs to the first `Pre` -- searching newest-first would silently swap two
/// unrelated results between two different tool calls, both reporting `Ok`, with
/// nothing to signal it).
///
/// `None` when the applicable tier finds nothing -- a `PostToolUse` with no matching
/// `PreToolUse` (never arrived, already matched and completed by an earlier `Post`, or
/// its whole turn already closed) is a no-op, not an error.
fn find_target(blocks: &[Block], hook: &ParsedHook) -> Option<usize> {
    match hook.tool_use_id.as_deref() {
        Some(want) => blocks
            .iter()
            .enumerate()
            .rev()
            .find_map(|(index, block)| match block {
                Block::ToolCall {
                    id: Some(id),
                    state: ToolState::Pending,
                    ..
                } if id == want => Some(index),
                _ => None,
            }),
        None => hook.tool_name.as_deref().and_then(|want| {
            blocks
                .iter()
                .enumerate()
                .find_map(|(index, block)| match block {
                    Block::ToolCall {
                        name,
                        state: ToolState::Pending,
                        ..
                    } if name == want => Some(index),
                    _ => None,
                })
        }),
    }
}

fn post_tool_use(draft: &mut Draft, hook: &ParsedHook, now: Instant, caps: Caps) -> bool {
    let Some(open) = draft.open_turn_index() else {
        return false;
    };
    let Some(index) = find_target(&draft.turns[open].blocks, hook) else {
        return false;
    };

    let ok = response_is_ok(hook.tool_response.as_ref());
    let raw = render_response(hook.tool_response.as_ref());
    let (capped, byte_capped) = cap_bytes(&raw, caps.max_result_bytes);
    let first_line = capped.lines().next().unwrap_or("");
    let text = summary::truncate_graphemes(first_line);
    let truncated = hook.tool_result_truncated == Some(true) || byte_capped;
    let duration_ms = draft.tool_started.get(&index).map(|started| {
        let millis = now.saturating_duration_since(*started).as_millis();
        u32::try_from(millis).unwrap_or(u32::MAX)
    });

    let Block::ToolCall {
        result,
        state,
        duration_ms: block_duration,
        ..
    } = &mut draft.turns[open].blocks[index]
    else {
        unreachable!("find_target only ever returns a ToolCall index");
    };
    *result = Some(ToolResult {
        ok,
        summary: text,
        detail: None,
        truncated,
    });
    *state = if ok { ToolState::Ok } else { ToolState::Failed };
    *block_duration = duration_ms;
    true
}

/// `false` when `tool_response` is a JSON object carrying a non-null `"error"` key, or
/// carrying `"success": false`; `true` otherwise, including when `tool_response` is
/// absent or is not an object. An object `tool_response` always arrives as an object
/// (`crates/cli`'s `bound_object` never stringifies one, even over `max_result_bytes`),
/// so this never needs to read `tool_result_stringified` -- that flag can only ever be
/// `true` for an array, number, bool or null, none of which this predicate inspects.
fn response_is_ok(value: Option<&serde_json::Value>) -> bool {
    let Some(serde_json::Value::Object(map)) = value else {
        return true;
    };
    let error_is_real = map.get("error").is_some_and(|v| !v.is_null());
    let success_is_false = matches!(map.get("success"), Some(serde_json::Value::Bool(false)));
    !(error_is_real || success_is_false)
}

/// `tool_response` rendered as text: the string itself when it is a JSON string, its
/// compact JSON encoding otherwise. `""` when absent.
fn render_response(value: Option<&serde_json::Value>) -> String {
    match value {
        None => String::new(),
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(other) => serde_json::to_string(other).unwrap_or_default(),
    }
}

/// The first `max` bytes of `s`, backed off to the nearest char boundary, and whether
/// that actually shortened it.
fn cap_bytes(s: &str, max: usize) -> (String, bool) {
    if s.len() <= max {
        return (s.to_string(), false);
    }
    let mut end = max.min(s.len());
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    (s[..end].to_string(), true)
}

fn permission_request(draft: &mut Draft, hook: &ParsedHook, now_unix_secs: u64) -> bool {
    let text = match hook.tool_name.as_deref() {
        Some(name) => format!("{name} needs permission"),
        None => "waiting for permission".to_string(),
    };
    draft.append_to_open_turn(
        Block::Notice {
            kind: NoticeKind::PermissionRequest,
            text,
        },
        now_unix_secs,
    );
    true
}

fn notification(draft: &mut Draft, hook: &ParsedHook, now_unix_secs: u64) -> bool {
    if hook.notification_type.as_deref() != Some("permission_prompt") {
        return false;
    }
    draft.append_to_open_turn(
        Block::Notice {
            kind: NoticeKind::PermissionRequest,
            text: "waiting for permission".to_string(),
        },
        now_unix_secs,
    );
    true
}

/// The parent-side effect only: appends a `SubagentSpawn` block to the open turn,
/// opening one if needed. The child-side effect -- a fresh conversation holding one
/// empty `Running` `Assistant` turn -- is `ConversationSet::on_hook`'s job, not
/// `apply`'s: applying this same function to the child's own draft would append a
/// second, spurious spawn block there instead of the parent's.
///
/// `label` and `model` are not populated here: `ParsedHook` carries no field for
/// either at a `SubagentStart` event (only `agent_id` and `agent_type`, confirmed by
/// `hooks::tests::sub_agent_fields_are_read`), and the value `SubagentTracker` computed
/// from the matching `PreToolUse`'s spawn request is not threaded through `on_hook`'s
/// documented signature (only `spawn_parent` is). See the task report's "Implementation
/// notes" for this brief gap.
fn subagent_start(draft: &mut Draft, hook: &ParsedHook, now_unix_secs: u64) -> bool {
    let agent_id = hook.agent_id.clone().unwrap_or_default();
    let kind = hook.agent_type.clone().unwrap_or_else(|| "agent".into());
    draft.append_to_open_turn(
        Block::SubagentSpawn {
            agent_id,
            kind,
            label: String::new(),
            model: None,
        },
        now_unix_secs,
    );
    true
}

/// Shared fixture and driver helpers for `build_tests.rs` and
/// `match_tool_call_tests.rs` -- both test files are `apply`'s tests, split by
/// responsibility (the general per-`HookKind` transform vs. `find_target`'s matcher
/// specifically) rather than duplicated, since `build_tests.rs` alone grew well past
/// the repository's ~600-line guideline once the matcher's own coverage gaps were
/// closed. `pub(super)` so both sibling test modules (`build`'s descendants) can use
/// them without re-declaring anything.
#[cfg(test)]
mod test_support {
    use super::*;
    use crate::conversation::ConversationSet;
    use proto::HookSource;

    pub(super) fn hook(kind: HookKind) -> ParsedHook {
        ParsedHook {
            source: HookSource::Claude,
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

    pub(super) fn prompt(text: &str) -> ParsedHook {
        let mut h = hook(HookKind::UserPromptSubmit);
        h.prompt = Some(text.to_owned());
        h
    }

    pub(super) fn pre(id: Option<&str>, name: &str) -> ParsedHook {
        let mut h = hook(HookKind::PreToolUse);
        h.tool_use_id = id.map(str::to_owned);
        h.tool_name = Some(name.to_owned());
        h
    }

    pub(super) fn post(id: Option<&str>, name: Option<&str>) -> ParsedHook {
        let mut h = hook(HookKind::PostToolUse);
        h.tool_use_id = id.map(str::to_owned);
        h.tool_name = name.map(str::to_owned);
        h
    }

    pub(super) fn draft() -> Draft {
        Draft::new(4, None, proto::Runtime::Claude)
    }

    /// `apply` against a fresh `Caps::default()`, since most fixtures don't care.
    pub(super) fn run(draft: &mut Draft, hook: &ParsedHook, ts: u64, now: Instant) -> bool {
        apply(
            draft,
            proto::Runtime::Claude,
            hook,
            ts,
            now,
            Caps::default(),
        )
    }

    pub(super) fn run_capped(
        draft: &mut Draft,
        hook: &ParsedHook,
        ts: u64,
        now: Instant,
        caps: Caps,
    ) -> bool {
        apply(draft, proto::Runtime::Claude, hook, ts, now, caps)
    }

    pub(super) fn spawn_hook(
        set: &mut ConversationSet,
        hook: &ParsedHook,
        parent: Option<&str>,
        ts: u64,
        now: Instant,
    ) -> Vec<Option<String>> {
        set.on_hook(
            proto::Runtime::Claude,
            hook,
            parent,
            ts,
            now,
            Caps::default(),
        )
    }

    pub(super) fn tool_call(
        block: &Block,
    ) -> (&Option<String>, &String, &ToolState, &Option<ToolResult>) {
        match block {
            Block::ToolCall {
                id,
                name,
                state,
                result,
                ..
            } => (id, name, state, result),
            _ => panic!("expected a ToolCall block, got {block:?}"),
        }
    }

    /// The open turn's blocks' `ToolState`s, in order. Panics if any block is not a
    /// `ToolCall` -- every caller already knows it should be.
    pub(super) fn tool_states(draft: &Draft, open: usize) -> Vec<ToolState> {
        draft.turns[open]
            .blocks
            .iter()
            .map(|b| *tool_call(b).2)
            .collect()
    }
}

#[cfg(test)]
#[path = "build_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "match_tool_call_tests.rs"]
mod match_tool_call_tests;
