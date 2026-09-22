//! The pure transform from one hook into one conversation's `Draft` (task M6.5.5). Hooks
//! are authoritative (spec decision 1): this is the only place that creates, closes or
//! reorders a turn, or sets a `ToolCall`'s state. No I/O, no clock of its own -- every
//! entry point takes `now_unix_secs: u64` and `now: std::time::Instant`.

use super::{Caps, Draft, summary};
use crate::hooks::{HookKind, ParsedHook};
use crate::subagents::SpawnOrigin;
use proto::{Block, NoticeKind, Role, ToolResult, ToolState, TurnState};
use std::time::Instant;

/// Applies one hook to `draft`, per the table in the M6.5.5 brief. `spawn` is the
/// matched `SubagentTracker::spawn_origin` for `hook.agent_id`, resolved by the caller;
/// it is read only for a `SubagentStart` (task M6.5.6: it carries the `label` and
/// `model` a `SubagentSpawn` block needs, which `ParsedHook` itself never has). Returns
/// whether anything changed.
pub(super) fn apply(
    draft: &mut Draft,
    runtime: proto::Runtime,
    hook: &ParsedHook,
    spawn: Option<&SpawnOrigin>,
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
        HookKind::PreToolUse => pre_tool_use(draft, hook, now_unix_secs, now, caps),
        HookKind::PostToolUse => post_tool_use(draft, hook, now, caps),
        HookKind::PermissionRequest => permission_request(draft, hook, now_unix_secs),
        HookKind::Notification => notification(draft, hook, now_unix_secs),
        HookKind::Stop | HookKind::SessionEnd | HookKind::TurnComplete => draft.close_open_turn(),
        HookKind::SubagentStart => subagent_start(draft, hook, spawn, now_unix_secs),
        HookKind::SubagentStop => draft.close_open_turn(),
    }
}

fn session_start(draft: &mut Draft, hook: &ParsedHook) -> bool {
    let mut changed = false;
    // A new session writing a new file (Claude's `/clear`): its prompts count from 0
    // again, starting after every `User` turn the window has so far (review F2). A new
    // file in the *same* session is the reader's restart instead, and a first
    // `SessionStart` (no path before it) starts at the base the draft already has.
    if draft.session_id != hook.session_id
        && draft.transcript_path.is_some()
        && hook.transcript_path.is_some()
        && draft.transcript_path != hook.transcript_path
    {
        let users = draft.turns.iter().filter(|t| t.role == Role::User).count() as u32;
        draft.session_base = draft.dropped_user_turns.saturating_add(users);
        draft.new_session = true;
        changed |= super::enrich::begin_session(draft);
    }
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

fn pre_tool_use(
    draft: &mut Draft,
    hook: &ParsedHook,
    now_unix_secs: u64,
    now: Instant,
    caps: Caps,
) -> bool {
    let name = hook.tool_name.clone().unwrap_or_else(|| "tool".into());
    let summary = summary::for_tool(&name, hook.tool_input.as_ref());
    let block = Block::ToolCall {
        id: hook.tool_use_id.clone(),
        name: name.clone(),
        summary,
        input: cap_input(hook.tool_input.as_ref(), caps.max_result_bytes),
        result: None,
        state: ToolState::Pending,
        duration_ms: None,
    };
    draft.append_to_open_turn(block, now_unix_secs);
    // Identity-keyed (fix round 1, F1), not by the block index just returned: the
    // index is a snapshot of a position that a later turn-8 front-insert can shift.
    draft
        .tool_started
        .push((hook.tool_use_id.clone(), name, now));
    true
}

/// The two-tier matching rule `find_target` (selecting a pending `ToolCall` block) and
/// `take_tool_start` (selecting a queued start time, fix round 1, finding F1) both need,
/// factored out once so the two cannot drift apart the way the review warned they would
/// if written twice: given `tool_use_id`, the id tier is the *only* tier tried -- the
/// **last** (by `index`) eligible candidate whose id equals it, or `None` when there is
/// no such candidate. (Wave-1 review finding F1: an explicit id that matches nothing is
/// evidence of a mismatch, not absence of evidence, so this must never fall through to
/// matching by name -- that would let an unrelated `Post` for a stale or foreign id
/// complete whatever else happens to share the same tool name.)
///
/// Only when `tool_use_id` is `None` is the name tier tried: the **oldest** (first, by
/// `index`) eligible candidate whose name equals `tool_name` (finding F2: results arrive
/// FIFO, so for two concurrent id-less calls to the same tool, the first `Post` belongs
/// to the first `Pre` -- searching newest-first would silently swap two unrelated
/// results between two different tool calls).
///
/// `candidates` must already be filtered to whatever "still open" means to the caller --
/// `find_target` filters to `Pending` blocks; `take_tool_start`'s candidates need no
/// such filter, since a `tool_started` entry is removed the moment it is consumed.
fn select_by_tier(
    candidates: &[(usize, Option<&str>, &str)],
    tool_use_id: Option<&str>,
    tool_name: Option<&str>,
) -> Option<usize> {
    match tool_use_id {
        Some(want) => candidates
            .iter()
            .rev()
            .find(|(_, id, _)| *id == Some(want))
            .map(|(index, _, _)| *index),
        None => tool_name.and_then(|want| {
            candidates
                .iter()
                .find(|(_, _, name)| *name == want)
                .map(|(index, _, _)| *index)
        }),
    }
}

/// Finds the target `ToolCall` in the open turn, via `select_by_tier`. The `Pending`
/// guard (wave-1 review finding F7) stops a redelivered `PostToolUse` for an id that
/// already completed from rewriting a finished call's result.
///
/// `None` when the applicable tier finds nothing -- a `PostToolUse` with no matching
/// `PreToolUse` (never arrived, already matched and completed by an earlier `Post`, or
/// its whole turn already closed) is a no-op, not an error.
fn find_target(blocks: &[Block], hook: &ParsedHook) -> Option<usize> {
    let candidates: Vec<(usize, Option<&str>, &str)> = blocks
        .iter()
        .enumerate()
        .filter_map(|(index, block)| match block {
            Block::ToolCall {
                id,
                name,
                state: ToolState::Pending,
                ..
            } => Some((index, id.as_deref(), name.as_str())),
            _ => None,
        })
        .collect();
    select_by_tier(
        &candidates,
        hook.tool_use_id.as_deref(),
        hook.tool_name.as_deref(),
    )
}

/// Finds and removes the `tool_started` entry matching `hook`, via the same
/// `select_by_tier` rule `find_target` uses, returning its recorded start time. Fix
/// round 1, finding F1: this used to be `draft.tool_started.get(&index)`, keyed by a
/// block index that a later task's front-insert (task M6.5.8 rule 2) can shift out from
/// under it; re-keying by tool identity needs its own selection over `tool_started`'s
/// own entries, done here rather than reusing `find_target`'s block-shaped one, but via
/// the one shared tier rule so the two selections cannot disagree about which tool call
/// a given hook means.
fn take_tool_start(draft: &mut Draft, hook: &ParsedHook) -> Option<Instant> {
    let candidates: Vec<(usize, Option<&str>, &str)> = draft
        .tool_started
        .iter()
        .enumerate()
        .map(|(index, (id, name, _))| (index, id.as_deref(), name.as_str()))
        .collect();
    let index = select_by_tier(
        &candidates,
        hook.tool_use_id.as_deref(),
        hook.tool_name.as_deref(),
    )?;
    Some(draft.tool_started.remove(index).2)
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
    let duration_ms = take_tool_start(draft, hook).map(|started| {
        let millis = now.saturating_duration_since(started).as_millis();
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
/// A tool `input` as it may be stored: whole, or not at all when its encoding could
/// exceed `max` (`conversation.max_result_bytes`, the cap a result's detail already
/// has). Task M6.5.10 review F1: `input` was capped nowhere, and one 6 MiB `Write` input
/// was enough to push a single turn past a frame. Truncating a JSON value would leave
/// something that is neither the input nor valid, so it is dropped whole; the summary,
/// derived from the full input before this, still says what the call did.
pub(super) fn cap_input(
    input: Option<&serde_json::Value>,
    max: usize,
) -> Option<serde_json::Value> {
    input
        .filter(|value| proto::conversation::value_byte_size(value) <= max)
        .cloned()
}

pub(super) fn cap_bytes(s: &str, max: usize) -> (String, bool) {
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
/// `label` and `model` come from `spawn` (task M6.5.6), the caller-resolved
/// `SubagentTracker::spawn_origin` for `hook.agent_id` -- `ParsedHook` itself carries
/// neither field at a `SubagentStart` event (only `agent_id` and `agent_type`, confirmed
/// by `hooks::tests::sub_agent_fields_are_read`). `kind` prefers `spawn.kind`, which the
/// tracker already derived from `hook.agent_type` with the same `"agent"` fallback this
/// function used to duplicate; `hook.agent_type` is read directly only when `spawn` is
/// `None` (an id beyond `MAX_CONVERSATIONS_PER_WINDOW`, or a test driving `apply`
/// without a tracker), so the fallback exists in exactly one place now.
fn subagent_start(
    draft: &mut Draft,
    hook: &ParsedHook,
    spawn: Option<&SpawnOrigin>,
    now_unix_secs: u64,
) -> bool {
    let agent_id = hook.agent_id.clone().unwrap_or_default();
    let (kind, label, model) = match spawn {
        Some(origin) => (
            origin.kind.clone(),
            origin.label.clone().unwrap_or_default(),
            origin.model.clone(),
        ),
        None => (
            hook.agent_type.clone().unwrap_or_else(|| "agent".into()),
            String::new(),
            None,
        ),
    };
    draft.append_to_open_turn(
        Block::SubagentSpawn {
            agent_id,
            kind,
            label,
            model,
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
    use crate::subagents::SpawnOrigin;
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

    /// `apply` against a fresh `Caps::default()` and no spawn origin, since most
    /// fixtures don't care about either.
    pub(super) fn run(draft: &mut Draft, hook: &ParsedHook, ts: u64, now: Instant) -> bool {
        apply(
            draft,
            proto::Runtime::Claude,
            hook,
            None,
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
        apply(draft, proto::Runtime::Claude, hook, None, ts, now, caps)
    }

    /// `apply` with an explicit spawn origin, for tests pinning `SubagentSpawn`'s
    /// `label`/`model` (task M6.5.6) rather than routing.
    pub(super) fn run_spawn(
        draft: &mut Draft,
        hook: &ParsedHook,
        spawn: &SpawnOrigin,
        ts: u64,
        now: Instant,
    ) -> bool {
        apply(
            draft,
            proto::Runtime::Claude,
            hook,
            Some(spawn),
            ts,
            now,
            Caps::default(),
        )
    }

    /// Drives `ConversationSet::on_hook` with a synthesized `SpawnOrigin` built from
    /// `parent` and `hook.agent_type` -- reproducing the routing and `kind` fallback
    /// `on_hook`'s pre-M6.5.6 `spawn_parent: Option<&str>` gave for free, since most of
    /// this file's `SubagentStart` fixtures only care about routing, not about `label`/
    /// `model`. Tests that care about the latter use `run_spawn` directly instead.
    pub(super) fn spawn_hook(
        set: &mut ConversationSet,
        hook: &ParsedHook,
        parent: Option<&str>,
        ts: u64,
        now: Instant,
    ) -> Vec<Option<String>> {
        let origin = SpawnOrigin {
            parent_id: parent.map(str::to_owned),
            kind: hook.agent_type.clone().unwrap_or_else(|| "agent".into()),
            label: None,
            model: None,
        };
        set.on_hook(
            proto::Runtime::Claude,
            hook,
            Some(&origin),
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

#[cfg(test)]
#[path = "spawn_origin_tests.rs"]
mod spawn_origin_tests;
