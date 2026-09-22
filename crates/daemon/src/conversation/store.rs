//! `ConversationSet` (task M6.5.6): revisions, the delta ring and the caps. Every
//! conversation belonging to one window, keyed by `agent_id` (decision A1). Entirely
//! pure: no I/O, no clock of its own -- every entry point takes `now_unix_secs` and
//! `now: Instant`.
//!
//! One `Entry` per key holds the hook-built timeline (`Draft`, from `mod.rs`) plus the
//! bookkeeping `Draft` itself does not carry: the revision counter, the degrade reason,
//! the drop accounting and the ring of recent patch batches `delta_since` replays from.
//! That split mirrors task M6.5.5's own note on `Draft`: it is "minus the
//! revision/degrade/drop bookkeeping task M6.5.6's store adds on top" -- this file is
//! where that bookkeeping actually lives, one level above `Draft`, not inside it, so a
//! `Draft` on its own (as `build.rs`'s tests still use it) never has to fake a `rev`.

use super::{Caps, DELTA_HISTORY, Draft, MAX_CONVERSATIONS_PER_WINDOW};
use crate::hooks::{HookKind, ParsedHook};
use crate::subagents::SpawnOrigin;
use proto::{DegradeReason, DropCause, TurnPatch};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

/// One revision's patch batch, tagged with the revision it belongs to so
/// `delta_since` can select the range it needs out of the ring without also keeping a
/// separate index.
#[derive(Debug, Clone)]
struct PatchBatch {
    rev: u64,
    patches: Vec<TurnPatch>,
}

/// One conversation's full bookkeeping: `draft` is the hook-built timeline; everything
/// else is this store's own layer on top of it.
struct Entry {
    draft: Draft,
    rev: u64,
    degraded: Option<DegradeReason>,
    dropped_turns: u32,
    dropped_by: Option<DropCause>,
    /// Set once a single turn alone has exceeded `caps.max_bytes` and been kept anyway
    /// (decision A9's "never trimmed to zero"). Sticky: nothing in this task clears it,
    /// because the condition it records does not resolve itself -- the oversize turn
    /// stays oversize until `max_turns` finally drops it via a fresh prompt, or the
    /// whole conversation goes away. `ConversationSet::oversize` is the caller's read of
    /// this flag; the brief's own wording ("the set records `oversize_logged`") is
    /// followed literally, but the interfaces section (`docs/milestones/
    /// M6.5-conversation-view.md`, the `ConversationSet` sketch) does not list an
    /// accessor for it -- see the task report's "Implementation notes" for that gap.
    oversize_logged: bool,
    /// The last `DELTA_HISTORY` revisions' patch batches, oldest first.
    history: VecDeque<PatchBatch>,
}

impl Entry {
    fn new(draft: Draft) -> Self {
        Entry {
            draft,
            rev: 0,
            degraded: None,
            dropped_turns: 0,
            dropped_by: None,
            oversize_logged: false,
            history: VecDeque::new(),
        }
    }

    fn snapshot(&self) -> proto::Conversation {
        self.draft
            .to_conversation(self.rev, self.degraded, self.dropped_turns, self.dropped_by)
    }

    /// Increments `rev` by exactly 1 and records `patches` as that revision's batch,
    /// evicting the oldest batch once the ring holds `DELTA_HISTORY` of them.
    fn record_revision(&mut self, patches: Vec<TurnPatch>) {
        self.rev += 1;
        if self.history.len() == DELTA_HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(PatchBatch {
            rev: self.rev,
            patches,
        });
    }

    /// Enforces `caps` (the brief's exact rules) after a change that may have grown
    /// `turns` or their bytes: while `turns.len() > caps.max_turns || byte_size() >
    /// caps.max_bytes` and `turns.len() > 1`, drops `turns[0]`, counting it toward
    /// `dropped_turns` and recording which predicate was true (`max_turns` checked
    /// first). Returns the `Drop` patches for whatever it removed. If the loop stops
    /// with exactly one turn still over `max_bytes`, that turn is kept and
    /// `oversize_logged` is set.
    fn enforce_caps(&mut self, caps: Caps) -> Vec<TurnPatch> {
        let mut drops = Vec::new();
        loop {
            if self.draft.turns.len() <= 1 {
                break;
            }
            let over_turns = self.draft.turns.len() > caps.max_turns;
            let over_bytes = self.draft.byte_size() > caps.max_bytes;
            if !over_turns && !over_bytes {
                break;
            }
            let cause = if over_turns {
                DropCause::Turns
            } else {
                DropCause::Bytes
            };
            let removed = self.draft.turns.remove(0);
            self.dropped_turns = self.dropped_turns.saturating_add(1);
            self.dropped_by = Some(cause);
            drops.push(TurnPatch::Drop { id: removed.id });
        }
        if self.draft.turns.len() == 1 && self.draft.byte_size() > caps.max_bytes {
            self.oversize_logged = true;
        }
        drops
    }
}

/// Concatenates a delta window's patches with the compaction `delta_since` promises:
/// only the last `Upsert` per turn id, and no `Upsert` for a turn a later `Drop`
/// removed. Patches are folded in revision order, which is also the order `patches`
/// arrives in (each batch's own patches, batches already filtered and ordered by the
/// caller).
fn compact_patches(patches: impl Iterator<Item = TurnPatch>) -> Vec<TurnPatch> {
    let mut result: Vec<TurnPatch> = Vec::new();
    for patch in patches {
        let id = match &patch {
            TurnPatch::Upsert(turn) => turn.id,
            TurnPatch::Drop { id } => *id,
        };
        result.retain(|existing| !matches!(existing, TurnPatch::Upsert(t) if t.id == id));
        result.push(patch);
    }
    result
}

/// Every conversation belonging to one window, keyed by `agent_id` (decision A1).
pub struct ConversationSet {
    window_id: u32,
    runtime: proto::Runtime,
    entries: HashMap<Option<String>, Entry>,
    /// The reason last passed to `set_degraded`, used only to compute that call's `new`
    /// return value -- distinct from any one entry's own `degraded` field, since a
    /// conversation created after the last `set_degraded` call has not caught up yet.
    current_degraded: Option<DegradeReason>,
}

impl ConversationSet {
    pub fn new(window_id: u32, runtime: proto::Runtime) -> Self {
        ConversationSet {
            window_id,
            runtime,
            entries: HashMap::new(),
            current_degraded: None,
        }
    }

    /// Applies one hook. `spawn` is the caller-resolved `SubagentTracker::spawn_origin`
    /// for `hook.agent_id`, read only for a `SubagentStart` -- both for routing (its
    /// `parent_id`) and for filling in a `SubagentSpawn` block's `label`/`model`
    /// (`build::apply`, task M6.5.6). Returns every key whose `rev` advanced.
    ///
    /// A `SubagentStart` is the one hook that spans two conversations: `build::apply`'s
    /// own `SubagentStart` effect (appending a `SubagentSpawn` block to the open turn)
    /// is applied to the *parent's* draft unconditionally -- per the brief's table, the
    /// parent-side effect does not depend on `hook.agent_id` being present. The child's
    /// entry, which does need a real id, is created fresh here, directly, only when
    /// `hook.agent_id` is `Some` and the set is under `MAX_CONVERSATIONS_PER_WINDOW`
    /// (decision A8) -- holding one empty `Running` `Assistant` turn and nothing else.
    /// Beyond the cap, no child conversation is created at all; the parent-side spawn
    /// block, which always lands regardless, is the hook's only visible effect.
    pub fn on_hook(
        &mut self,
        runtime: proto::Runtime,
        hook: &ParsedHook,
        spawn: Option<&SpawnOrigin>,
        now_unix_secs: u64,
        now: Instant,
        caps: Caps,
    ) -> Vec<Option<String>> {
        if hook.kind == HookKind::SubagentStart {
            let mut changed = Vec::new();
            let parent_key = self.ensure_key(spawn.and_then(|origin| origin.parent_id.clone()));
            if self.touch(&parent_key, caps, |draft| {
                super::build::apply(draft, runtime, hook, spawn, now_unix_secs, now, caps)
            }) {
                changed.push(parent_key);
            }

            if let Some(child_id) = hook.agent_id.clone() {
                let child_key = Some(child_id);
                let already_exists = self.entries.contains_key(&child_key);
                if !already_exists && self.entries.len() < MAX_CONVERSATIONS_PER_WINDOW {
                    self.entries.insert(
                        child_key.clone(),
                        Entry::new(Draft::new(self.window_id, child_key.clone(), runtime)),
                    );
                    if self.touch(&child_key, caps, |draft| {
                        draft.push_turn(
                            proto::Role::Assistant,
                            proto::TurnState::Running,
                            Vec::new(),
                            now_unix_secs,
                        );
                        true
                    }) {
                        changed.push(child_key);
                    }
                }
            }
            return changed;
        }

        let key = self.ensure_key(hook.agent_id.clone());
        if self.touch(&key, caps, |draft| {
            super::build::apply(draft, runtime, hook, spawn, now_unix_secs, now, caps)
        }) {
            vec![key]
        } else {
            Vec::new()
        }
    }

    /// Sets or clears the degrade reason on every conversation in this set. Returns the
    /// keys whose `rev` advanced, and whether `reason` differs from the reason the
    /// previous call (or, on the first call, the set's own starting state, `None`) left
    /// in place -- the caller's cue to log once (decision 2).
    pub fn set_degraded(&mut self, reason: Option<DegradeReason>) -> (Vec<Option<String>>, bool) {
        let is_new = self.current_degraded != reason;
        self.current_degraded = reason;
        let mut changed = Vec::new();
        let keys: Vec<Option<String>> = self.entries.keys().cloned().collect();
        for key in keys {
            let entry = self.entries.get_mut(&key).expect("key just listed");
            if entry.degraded != reason {
                entry.degraded = reason;
                entry.record_revision(Vec::new());
                changed.push(key);
            }
        }
        (changed, is_new)
    }

    /// Whether `agent_id`'s conversation currently holds a single turn that alone
    /// exceeds `caps.max_bytes` and was kept anyway (decision A9). `false` for an
    /// unknown key.
    pub fn oversize(&self, agent_id: Option<&str>) -> bool {
        self.entries
            .get(&agent_id.map(str::to_owned))
            .is_some_and(|entry| entry.oversize_logged)
    }

    /// A snapshot of the conversation at `agent_id`, or `None` when no hook has ever
    /// touched that key.
    pub fn snapshot(&self, agent_id: Option<&str>) -> Option<proto::Conversation> {
        self.entries
            .get(&agent_id.map(str::to_owned))
            .map(Entry::snapshot)
    }

    /// The patches for every revision after `from_rev` up to the conversation's current
    /// `rev`, compacted (`compact_patches`). `None` when `from_rev` is ahead of `rev`,
    /// when the gap is wider than `DELTA_HISTORY`, or when the key is unknown -- in
    /// every case the caller falls back to a full `ConversationSnapshot` (decision A3).
    pub fn delta_since(
        &self,
        agent_id: Option<&str>,
        from_rev: u64,
    ) -> Option<(u64, Vec<TurnPatch>)> {
        let entry = self.entries.get(&agent_id.map(str::to_owned))?;
        if from_rev > entry.rev || entry.rev - from_rev > DELTA_HISTORY as u64 {
            return None;
        }
        let patches = compact_patches(
            entry
                .history
                .iter()
                .filter(|batch| batch.rev > from_rev && batch.rev <= entry.rev)
                .flat_map(|batch| batch.patches.iter().cloned()),
        );
        Some((entry.rev, patches))
    }

    /// Every key this set currently holds a conversation for.
    pub fn keys(&self) -> Vec<Option<String>> {
        self.entries.keys().cloned().collect()
    }

    /// Resolves `key` to the key a *new* entry should actually be created under
    /// (decision A8): an already-known key, or `None` itself, is always honored; a
    /// brand-new key beyond `MAX_CONVERSATIONS_PER_WINDOW` is redirected to the root
    /// (`None`) key instead of creating one more conversation. Creates the resolved
    /// entry if it does not exist yet, and returns the resolved key.
    fn ensure_key(&mut self, key: Option<String>) -> Option<String> {
        let resolved = if key.is_none()
            || self.entries.contains_key(&key)
            || self.entries.len() < MAX_CONVERSATIONS_PER_WINDOW
        {
            key
        } else {
            None
        };
        let window_id = self.window_id;
        let runtime = self.runtime;
        self.entries
            .entry(resolved.clone())
            .or_insert_with(|| Entry::new(Draft::new(window_id, resolved.clone(), runtime)));
        resolved
    }

    /// Applies `f` to `key`'s draft, then -- only if `f` reports a change -- records
    /// exactly one revision: an `Upsert` for every turn `f` created or modified (found
    /// by comparing `turns` before and after, since `build::apply`'s own per-`HookKind`
    /// effects vary too widely to track surgically) plus a `Drop` for every turn
    /// `enforce_caps` trims in the same call. `key`'s entry must already exist
    /// (`ensure_key` or the `SubagentStart` child-creation path is always called
    /// first).
    fn touch<F>(&mut self, key: &Option<String>, caps: Caps, f: F) -> bool
    where
        F: FnOnce(&mut Draft) -> bool,
    {
        let entry = self
            .entries
            .get_mut(key)
            .expect("touch is only ever called against a key ensure_key just created");
        let before: HashMap<u64, proto::Turn> = entry
            .draft
            .turns
            .iter()
            .map(|turn| (turn.id, turn.clone()))
            .collect();
        if !f(&mut entry.draft) {
            return false;
        }
        let mut patches: Vec<TurnPatch> = entry
            .draft
            .turns
            .iter()
            .filter(|turn| before.get(&turn.id) != Some(turn))
            .map(|turn| TurnPatch::Upsert(turn.clone()))
            .collect();
        patches.extend(entry.enforce_caps(caps));
        entry.record_revision(patches);
        true
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
