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
//!
//! Nothing orders a transcript line after its hook (fix round 1, F1): the hook sender
//! gives up after `HOOK_DEADLINE` and the reader polls every `TRANSCRIPT_POLL`, so either
//! can come first. A record whose hook has not landed yet is *parked*, not discarded, and
//! retried at the start of every `apply` and after every hook (`retry`):
//!
//! - a `ToolDetail` whose call does not exist yet, or whose detail meets a call with no
//!   `result` yet, waits in `pending_tools`;
//! - the first `UserText` whose `User` turn does not exist yet starts `pending_positional`,
//!   a FIFO that also takes every positional record after it, so nothing positional
//!   overtakes a parked prompt. A parked prompt is checked against its turn only once
//!   that turn exists, so alignment stays sound: if its hook was really lost, the next
//!   prompt's hook fills the slot, the texts differ, and the result is `Misaligned`.

use super::{Caps, Draft, build};
use crate::transcript::Record;
use proto::{Block, Role};
use std::collections::{HashMap, HashSet, VecDeque};

/// The most `ToolDetail`s `pending_tools` holds; beyond it the oldest is evicted.
///
/// A hook reaches the daemon within `HOOK_DEADLINE` (1 s, `crates/cli/src/hook.rs`) or
/// never, so a record legitimately waits for at most about a second of transcript: a
/// handful of calls, even with parallel tool use. Everything else in the buffer is a
/// record whose hook is never coming (a call from a sub-agent or from a tool the runtime
/// fires no hook for) and nothing on the timeline is missing it, so eviction is not
/// reported. 64 is an order of magnitude above a second's worth of calls.
pub(super) const PENDING_TOOLS_MAX: usize = 64;

/// The most positional records `pending_positional` holds behind a parked prompt.
///
/// The same one-second argument as `PENDING_TOOLS_MAX`: a late prompt hook lands within
/// `HOOK_DEADLINE`, and in one second a transcript writes a few lines, not 64. Filling
/// the queue means the prompt's hook is not coming while the transcript keeps going, so
/// overflow stops buffering and reports `Misaligned` from the parked prompt's ordinal:
/// the transcript no longer lines up with a timeline the hooks are not delivering.
pub(super) const PENDING_POSITIONAL_MAX: usize = 64;

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
    /// `ToolDetail`s (or the detail half of one) waiting for their hook: see the module
    /// comment. Bounded by `PENDING_TOOLS_MAX`.
    pending_tools: VecDeque<PendingTool>,
    /// A parked `UserText` whose `User` turn does not exist yet, at the front, and every
    /// positional record read after it, in file order. When non-empty its front is always
    /// a `UserText`. Bounded by `PENDING_POSITIONAL_MAX`.
    pending_positional: VecDeque<Record>,
}

/// A `ToolDetail` waiting for its call or its result. `detail` is already capped to
/// `max_result_bytes` (with whether that cut it), so a parked record holds no more than
/// an applied one would.
#[derive(Debug, Clone, PartialEq)]
struct PendingTool {
    tool_use_id: String,
    input: Option<serde_json::Value>,
    detail: Option<(String, bool)>,
}

/// Where transcript ordinal `k`'s `User` turn is.
enum Slot {
    /// Below `dropped_user_turns`: the caps dropped it.
    Dropped,
    /// Past the last `User` turn: its hook has not landed (yet).
    NotYet,
    /// At this index in `draft.turns`.
    At(usize),
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

    #[cfg(test)]
    pub(super) fn pending(&self) -> (usize, usize) {
        (self.pending_tools.len(), self.pending_positional.len())
    }
}

/// Applies `records`, in order. Returns whether the draft changed, including its
/// alignment state (a first mismatch is a change: the store then reports `Misaligned`).
///
/// Not idempotent on its own: `AssistantText` *appends*, so applying the same records
/// twice without a `reset` in between shows that prose twice -- visibly duplicated, never
/// misattributed. The only re-read path (task M6.5.10's restart) calls `reset` first.
pub(super) fn apply(draft: &mut Draft, records: &[Record], caps: Caps) -> bool {
    let mut changed = retry(draft);
    for record in records {
        changed |= match record {
            Record::ToolDetail {
                tool_use_id,
                input,
                detail,
                ok: _,
            } => {
                let pending = PendingTool {
                    tool_use_id: tool_use_id.clone(),
                    input: build::cap_input(input.as_ref(), caps.max_result_bytes),
                    detail: detail
                        .as_deref()
                        .map(|d| build::cap_bytes(d, caps.max_result_bytes)),
                };
                tool_detail_or_park(draft, pending)
            }
            positional => {
                if draft.enrichment.pending_positional.is_empty() {
                    positional_record(draft, positional.clone())
                } else {
                    park_positional(draft, positional.clone())
                }
            }
        };
    }
    changed
}

/// Retries every parked record (see the module comment). The store calls this after
/// every hook, inside the hook's own revision, and `apply` calls it first. Returns
/// whether anything changed.
pub(super) fn retry(draft: &mut Draft) -> bool {
    let mut changed = false;
    for pending in std::mem::take(&mut draft.enrichment.pending_tools) {
        changed |= tool_detail_or_park(draft, pending);
    }
    while let Some(front) = draft.enrichment.pending_positional.front() {
        if let Record::UserText { ordinal, .. } = front
            && in_aligned_range(draft, *ordinal)
            && matches!(user_slot(draft, *ordinal), Slot::NotYet)
        {
            break;
        }
        let record = draft
            .enrichment
            .pending_positional
            .pop_front()
            .expect("front just seen");
        changed |= positional_record(draft, record);
    }
    changed
}

/// One positional record with no queue ahead of it. A `UserText` whose turn does not
/// exist yet starts the queue instead of being judged.
fn positional_record(draft: &mut Draft, record: Record) -> bool {
    match &record {
        Record::UserText {
            session_id,
            ordinal,
            text,
        } => {
            if !in_aligned_range(draft, *ordinal) {
                return false;
            }
            match user_slot(draft, *ordinal) {
                Slot::Dropped => false,
                Slot::NotYet => park_positional(draft, record),
                Slot::At(index) => {
                    let (session_id, ordinal, text) = (session_id.clone(), *ordinal, text.clone());
                    user_text(draft, session_id.as_deref(), ordinal, &text, index)
                }
            }
        }
        Record::AssistantText { ordinal, text, .. } => assistant_text(draft, *ordinal, text),
        Record::ToolDetail { .. } => unreachable!("positional records only"),
    }
}

/// Queues a positional record behind the parked prompt. On overflow, stops buffering:
/// the queue is dropped and positional enrichment stops at the parked prompt's ordinal,
/// reported as `Misaligned` (a change, so it returns `true`).
fn park_positional(draft: &mut Draft, record: Record) -> bool {
    let enrichment = &mut draft.enrichment;
    if enrichment.pending_positional.len() < PENDING_POSITIONAL_MAX {
        enrichment.pending_positional.push_back(record);
        return false;
    }
    let first = match enrichment.pending_positional.front() {
        Some(Record::UserText { ordinal, .. }) => *ordinal,
        _ => unreachable!("a non-empty queue starts with its parked UserText"),
    };
    enrichment.pending_positional.clear();
    enrichment.checked = None;
    enrichment.misaligned_at = Some(enrichment.misaligned_at.map_or(first, |m| m.min(first)));
    true
}

/// Applies what it can of `pending` and parks the rest (evicting the oldest parked
/// record at `PENDING_TOOLS_MAX`). Returns whether the draft changed.
fn tool_detail_or_park(draft: &mut Draft, pending: PendingTool) -> bool {
    let (changed, rest) = tool_detail(draft, pending);
    if let Some(rest) = rest {
        let queue = &mut draft.enrichment.pending_tools;
        if queue.len() >= PENDING_TOOLS_MAX {
            queue.pop_front();
        }
        queue.push_back(rest);
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

/// Where transcript ordinal `ordinal`'s `User` turn is: surviving `User` turn
/// `ordinal - dropped_user_turns`.
fn user_slot(draft: &Draft, ordinal: u32) -> Slot {
    let Some(surviving) = ordinal.checked_sub(draft.dropped_user_turns) else {
        return Slot::Dropped;
    };
    draft
        .turns
        .iter()
        .enumerate()
        .filter(|(_, turn)| turn.role == Role::User)
        .nth(surviving as usize)
        .map_or(Slot::NotYet, |(index, _)| Slot::At(index))
}

/// The index of ordinal `ordinal`'s `User` turn, when it exists.
fn user_turn_index(draft: &Draft, ordinal: u32) -> Option<usize> {
    match user_slot(draft, ordinal) {
        Slot::At(index) => Some(index),
        Slot::Dropped | Slot::NotYet => None,
    }
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

/// Checks ordinal `ordinal`'s prompt against the hook-built `User` turn at `index` and,
/// when they line up, replaces the hook's text with the transcript's.
fn user_text(
    draft: &mut Draft,
    session_id: Option<&str>,
    ordinal: u32,
    text: &str,
    index: usize,
) -> bool {
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

/// Applies what it can of one `ToolDetail` and returns what must wait: the whole record
/// when no call has its id yet, or its detail when the call has no `result` yet and is
/// still `Pending` (a `Denied` call never gets one, so its detail is dropped).
fn tool_detail(draft: &mut Draft, pending: PendingTool) -> (bool, Option<PendingTool>) {
    let PendingTool {
        tool_use_id,
        input,
        detail,
    } = pending;
    // The last call with this id, the same tier `build.rs` uses to match a result.
    let found = draft.turns.iter_mut().rev().find_map(|turn| {
        let turn_id = turn.id;
        turn.blocks
            .iter_mut()
            .rev()
            .find(
                |block| matches!(block, Block::ToolCall { id: Some(id), .. } if *id == tool_use_id),
            )
            .map(|block| (turn_id, block))
    });
    let Some((turn_id, block)) = found else {
        let waiting = (input.is_some() || detail.is_some()).then_some(PendingTool {
            tool_use_id,
            input,
            detail,
        });
        return (false, waiting);
    };
    let Block::ToolCall {
        input: block_input,
        result,
        state,
        ..
    } = block
    else {
        unreachable!("matched a ToolCall");
    };

    let key = (turn_id, tool_use_id.clone());
    let mut changed = false;
    if let Some(input) = input
        && block_input.as_ref() != Some(&input)
    {
        let original = draft.enrichment.tools.entry(key.clone()).or_default();
        original.input.get_or_insert_with(|| block_input.clone());
        *block_input = Some(input);
        changed = true;
    }
    let Some((capped, byte_capped)) = detail else {
        return (changed, None);
    };
    // Detail only on a call the hooks already gave a result: a `Pending` call stays
    // exactly as the hooks left it, and its detail waits for the `PostToolUse`.
    let Some(result) = result.as_mut() else {
        let waiting = (*state == proto::ToolState::Pending).then_some(PendingTool {
            tool_use_id,
            input: None,
            detail: Some((capped, byte_capped)),
        });
        return (changed, waiting);
    };
    // The hook's flag says the CLI cut `tool_response`, the summary's source. Once the
    // transcript's detail replaces `detail`, the flag describes that detail: truncated
    // exactly when it was over the cap (fix round 1, F2). `reset` restores the hook's.
    let truncated = byte_capped;
    if result.detail.as_deref() != Some(capped.as_str()) || result.truncated != truncated {
        let original = draft.enrichment.tools.entry(key).or_default();
        original
            .detail
            .get_or_insert_with(|| (result.detail.clone(), result.truncated));
        result.detail = Some(capped);
        result.truncated = truncated;
        changed = true;
    }
    (changed, None)
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
#[path = "enrich_pending_tests.rs"]
mod pending_tests;

#[cfg(test)]
#[path = "enrich_store_tests.rs"]
mod store_tests;

#[cfg(test)]
#[path = "revision_rule_tests.rs"]
mod revision_rule_tests;

#[cfg(test)]
#[path = "input_cap_tests.rs"]
mod input_cap_tests;
