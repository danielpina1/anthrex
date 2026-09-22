//! Transcript records onto the hook-built timeline (task M6.5.8). Pure: no I/O, no
//! clock -- a function of the `Draft` and a slice of `Record`s.
//!
//! Hooks are authoritative (spec decision 1). A record may fill in a `User` turn's
//! prompt text, a leading prose `Text` block on an `Assistant` turn, and a `ToolCall`'s
//! `input` and `result.detail`, and nothing else: it never creates, removes, reorders or
//! re-states a turn, never changes a `ToolState` or `ToolResult.ok`, and never gives a
//! call a `result` the hooks did not.
//!
//! Two joins, with different failure modes:
//!
//! - **Positional** (`UserText`, `AssistantText`): transcript ordinal `k` is the `k`-th
//!   prompt since the session began, which is surviving `User` turn
//!   `k - draft.dropped_user_turns`. A miscounted prompt would shift every later ordinal
//!   onto the wrong turn, and the result would look plausible. So every ordinal's prompt
//!   is checked against the hook-built prompt before anything with that ordinal lands,
//!   and the first mismatch stops positional enrichment for good (until `reset`).
//! - **By id** (`ToolDetail`): `tool_use_id` names its call outright, so it applies
//!   regardless of alignment.

use super::{Caps, Draft, build};
use crate::transcript::Record;
use proto::{Block, Role};
use std::collections::{HashMap, HashSet};

/// What `apply` changed on a draft, and the alignment state it carries from one batch to
/// the next. `reset` reads this to undo exactly the enrichment: every entry here records
/// the hook-built value an enrichment overwrote (or that there was none), so nothing a
/// hook set is ever lost, and nothing a hook set *after* enrichment is undone.
///
/// Keyed by turn id and tool-use id, never by a block index: `apply` itself inserts
/// blocks at the front of a turn, which is exactly what makes an index go stale.
#[derive(Debug, Clone, Default, PartialEq)]
pub(super) struct Enrichment {
    /// `User` turn id -> the hook's own prompt text that a `UserText` replaced.
    prompts: HashMap<u64, String>,
    /// `Assistant` turn ids whose `blocks[0]` is the prose `Text` block `apply` inserted.
    /// `build.rs` only ever appends to a turn's blocks, so that block stays at the front.
    prose: HashSet<u64>,
    /// `(turn id, tool-use id)` -> the hook's own values that a `ToolDetail` replaced.
    tools: HashMap<(u64, String), ToolOriginal>,
    /// The ordinal of the first prompt that did not line up with the hooks. Once set, no
    /// `UserText`/`AssistantText` with an ordinal at or past it applies, in this batch or
    /// any later one: a later batch cannot resume positional enrichment past a mismatch.
    misaligned_at: Option<u32>,
    /// The ordinal whose prompt was most recently checked and found aligned. An
    /// `AssistantText` applies only to this ordinal: records arrive in file order, so a
    /// reply always follows its own prompt, and prose for a prompt that was never checked
    /// (out of range, dropped by the caps, or counted with no text) is not applied.
    checked: Option<u32>,
}

/// The hook-built values under one enriched `ToolCall`. Each field is `Some` only once a
/// record actually overwrote that value.
#[derive(Debug, Clone, Default, PartialEq)]
struct ToolOriginal {
    input: Option<Option<serde_json::Value>>,
    /// `(detail, truncated)` as the hook left them.
    detail: Option<(Option<String>, bool)>,
}

impl Enrichment {
    /// Whether a prompt failed the alignment check: the store reports
    /// `DegradeReason::Misaligned` while this holds.
    pub(super) fn is_misaligned(&self) -> bool {
        self.misaligned_at.is_some()
    }

    /// Forgets what was recorded for a turn the caps just dropped, so this stays bounded
    /// by the turns still held.
    pub(super) fn forget_turn(&mut self, turn_id: u64) {
        self.prompts.remove(&turn_id);
        self.prose.remove(&turn_id);
        self.tools.retain(|(turn, _), _| *turn != turn_id);
    }
}

/// Applies `records`, in order. Returns whether the draft changed, including its
/// alignment state (a first mismatch is a change: the store then reports `Misaligned`).
///
/// Not idempotent on its own: `AssistantText` *appends*, so applying the same records
/// twice without a `reset` in between shows that prose twice -- visibly duplicated, never
/// misattributed. The only re-read path (task M6.5.10's restart) calls `reset` first.
pub(super) fn apply(draft: &mut Draft, records: &[Record], caps: Caps) -> bool {
    let mut changed = false;
    for record in records {
        changed |= match record {
            Record::UserText {
                session_id,
                ordinal,
                text,
            } => user_text(draft, session_id.as_deref(), *ordinal, text),
            Record::AssistantText { ordinal, text, .. } => assistant_text(draft, *ordinal, text),
            Record::ToolDetail {
                tool_use_id,
                input,
                detail,
                ok: _,
            } => tool_detail(draft, tool_use_id, input.as_ref(), detail.as_deref(), caps),
        };
    }
    changed
}

/// Whether positional enrichment may still act on `ordinal`.
fn in_aligned_range(draft: &Draft, ordinal: u32) -> bool {
    draft
        .enrichment
        .misaligned_at
        .is_none_or(|first_bad| ordinal < first_bad)
}

/// The index in `draft.turns` of the `User` turn transcript ordinal `ordinal` maps to, or
/// `None` when it belongs to a dropped turn or to one the hooks have not built (yet).
fn user_turn_index(draft: &Draft, ordinal: u32) -> Option<usize> {
    let surviving = ordinal.checked_sub(draft.dropped_user_turns)? as usize;
    draft
        .turns
        .iter()
        .enumerate()
        .filter(|(_, turn)| turn.role == Role::User)
        .nth(surviving)
        .map(|(index, _)| index)
}

/// The hook's own prompt text for the `User` turn at `index`: the value it had before any
/// enrichment replaced it.
fn hook_prompt(draft: &Draft, index: usize) -> Option<&str> {
    let turn = &draft.turns[index];
    if let Some(original) = draft.enrichment.prompts.get(&turn.id) {
        return Some(original);
    }
    turn.blocks.iter().find_map(|block| match block {
        Block::Text { text } => Some(text.as_str()),
        _ => None,
    })
}

fn user_text(draft: &mut Draft, session_id: Option<&str>, ordinal: u32, text: &str) -> bool {
    if !in_aligned_range(draft, ordinal) {
        return false;
    }
    let Some(index) = user_turn_index(draft, ordinal) else {
        return false;
    };
    // `build.rs` stores `hook.prompt` verbatim, and nothing upstream truncates it, so
    // the two must be equal up to surrounding whitespace. A prefix is not accepted: with
    // no truncation to allow for, "yes" against "yes please" is a different prompt. A
    // record from another session is a different prompt too, whatever its words.
    let same_session = match (session_id, draft.session_id.as_deref()) {
        (Some(record), Some(hook)) => record == hook,
        _ => true,
    };
    let aligned = same_session && hook_prompt(draft, index).map(str::trim) == Some(text.trim());
    if !aligned {
        draft.enrichment.misaligned_at = Some(ordinal);
        draft.enrichment.checked = None;
        return true;
    }
    draft.enrichment.checked = Some(ordinal);

    let turn = &mut draft.turns[index];
    let Some(Block::Text { text: current }) = turn
        .blocks
        .iter_mut()
        .find(|block| matches!(block, Block::Text { .. }))
    else {
        return false;
    };
    if current == text {
        return false;
    }
    draft
        .enrichment
        .prompts
        .entry(turn.id)
        .or_insert_with(|| current.clone());
    *current = text.to_owned();
    true
}

fn assistant_text(draft: &mut Draft, ordinal: u32, text: &str) -> bool {
    if !in_aligned_range(draft, ordinal) || draft.enrichment.checked != Some(ordinal) {
        return false;
    }
    let Some(user_index) = user_turn_index(draft, ordinal) else {
        return false;
    };
    // The reply to a prompt is the turn `build.rs` pushed right after it.
    let Some(turn) = draft
        .turns
        .get_mut(user_index + 1)
        .filter(|turn| turn.role == Role::Assistant)
    else {
        return false;
    };
    if draft.enrichment.prose.contains(&turn.id) {
        let Some(Block::Text { text: prose }) = turn.blocks.first_mut() else {
            unreachable!("an enriched turn's first block is its prose Text block");
        };
        prose.push_str("\n\n");
        prose.push_str(text);
    } else {
        turn.blocks.insert(
            0,
            Block::Text {
                text: text.to_owned(),
            },
        );
        draft.enrichment.prose.insert(turn.id);
    }
    true
}

fn tool_detail(
    draft: &mut Draft,
    tool_use_id: &str,
    input: Option<&serde_json::Value>,
    detail: Option<&str>,
    caps: Caps,
) -> bool {
    // The last call with this id, the same tier `build.rs` uses to match a result.
    let Some((turn_id, block)) = draft.turns.iter_mut().rev().find_map(|turn| {
        let turn_id = turn.id;
        turn.blocks
            .iter_mut()
            .rev()
            .find(
                |block| matches!(block, Block::ToolCall { id: Some(id), .. } if id == tool_use_id),
            )
            .map(|block| (turn_id, block))
    }) else {
        return false;
    };
    let Block::ToolCall {
        input: block_input,
        result,
        ..
    } = block
    else {
        unreachable!("matched a ToolCall");
    };

    let key = (turn_id, tool_use_id.to_owned());
    let mut changed = false;
    if let Some(input) = input
        && block_input.as_ref() != Some(input)
    {
        let original = draft.enrichment.tools.entry(key.clone()).or_default();
        original.input.get_or_insert_with(|| block_input.clone());
        *block_input = Some(input.clone());
        changed = true;
    }
    // Detail only on a call the hooks already gave a result: a `Pending` call stays
    // exactly as the hooks left it.
    if let (Some(detail), Some(result)) = (detail, result.as_mut()) {
        let (capped, byte_capped) = build::cap_bytes(detail, caps.max_result_bytes);
        let truncated = result.truncated || byte_capped;
        if result.detail.as_deref() != Some(capped.as_str()) || result.truncated != truncated {
            let original = draft.enrichment.tools.entry(key).or_default();
            original
                .detail
                .get_or_insert_with(|| (result.detail.clone(), result.truncated));
            result.detail = Some(capped);
            result.truncated = truncated;
            changed = true;
        }
    }
    changed
}

/// Undoes every enrichment-sourced value -- replaced prompts, inserted prose, tool
/// `input`/`detail`, and the alignment state -- leaving the hook-built timeline exactly
/// as the hooks made it, including anything a hook changed after enrichment. Returns
/// whether anything changed.
pub(super) fn reset(draft: &mut Draft) -> bool {
    let enrichment = std::mem::take(&mut draft.enrichment);
    // `checked` alone is bookkeeping, not a visible value: a reset that clears only it
    // changes nothing a client can see, so it does not count as a change.
    let visible = !enrichment.prompts.is_empty()
        || !enrichment.prose.is_empty()
        || !enrichment.tools.is_empty()
        || enrichment.misaligned_at.is_some();
    for turn in &mut draft.turns {
        if let Some(original) = enrichment.prompts.get(&turn.id)
            && let Some(Block::Text { text }) = turn
                .blocks
                .iter_mut()
                .find(|block| matches!(block, Block::Text { .. }))
        {
            text.clone_from(original);
        }
        if enrichment.prose.contains(&turn.id)
            && matches!(turn.blocks.first(), Some(Block::Text { .. }))
        {
            turn.blocks.remove(0);
        }
        let turn_id = turn.id;
        let mut restored: HashSet<&str> = HashSet::new();
        // Newest first and once per id: `tool_detail` enriched only the last call with a
        // given id, so only that one is restored.
        for block in turn.blocks.iter_mut().rev() {
            let Block::ToolCall {
                id: Some(id),
                input,
                result,
                ..
            } = block
            else {
                continue;
            };
            let Some(((_, key), original)) = enrichment.tools.get_key_value(&(turn_id, id.clone()))
            else {
                continue;
            };
            if !restored.insert(key.as_str()) {
                continue;
            }
            if let Some(hook_input) = &original.input {
                input.clone_from(hook_input);
            }
            if let (Some((detail, truncated)), Some(result)) = (&original.detail, result.as_mut()) {
                result.detail.clone_from(detail);
                result.truncated = *truncated;
            }
        }
    }
    visible
}

#[cfg(test)]
#[path = "enrich_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "enrich_alignment_tests.rs"]
mod alignment_tests;

#[cfg(test)]
#[path = "enrich_reset_tests.rs"]
mod reset_tests;

#[cfg(test)]
#[path = "enrich_store_tests.rs"]
mod store_tests;
