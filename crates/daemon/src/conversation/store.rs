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

use super::{Caps, DELTA_HISTORY, Draft, MAX_CONVERSATIONS_PER_WINDOW, enrich};
use crate::hooks::{HookKind, ParsedHook};
use crate::subagents::SpawnOrigin;
use crate::transcript::Record;
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
    /// Whether this conversation *currently* holds a single turn that alone exceeds
    /// `caps.max_bytes` and was kept anyway (decision A9's "never trimmed to zero").
    /// Fix round 1, finding F7: recomputed fresh on every `enforce_caps` call (not
    /// merely set-once-true), so it is a genuine current-state query -- `oversize()`'s
    /// own doc comment previously warned it could not be used that way.
    oversize_logged: bool,
    /// Whether the *current* `oversize_logged == true` streak has already been reported
    /// through `take_oversize`. Reset to `false` whenever `oversize_logged` goes back to
    /// `false`, so a later re-entry into oversize is reported again -- "exactly once per
    /// transition into oversize" (fix round 1, finding F7).
    oversize_reported: bool,
    /// The last `DELTA_HISTORY` revisions' patch batches, oldest first.
    history: VecDeque<PatchBatch>,
}

impl Entry {
    /// `degraded` seeds from `ConversationSet::current_degraded` (fix round 1, finding
    /// F6): a conversation created after the day's last `set_degraded` call used to
    /// start at `None` regardless and could only ever catch up if `set_degraded` was
    /// called *again* -- which never happens for a reason that stopped applying between
    /// calls. Seeding at creation means a late-created conversation is correct from its
    /// very first revision, with no extra revision bump needed (it starts at `rev 0`
    /// either way).
    fn new(draft: Draft, degraded: Option<DegradeReason>) -> Self {
        Entry {
            draft,
            rev: 0,
            degraded,
            dropped_turns: 0,
            dropped_by: None,
            oversize_logged: false,
            oversize_reported: false,
            history: VecDeque::new(),
        }
    }

    /// The reason a client sees. The reader's own reason (`degraded`, from
    /// `set_degraded`) wins when there is one: a file that cannot be read is the more
    /// fundamental problem. Otherwise a failed alignment check shows as `Misaligned`
    /// (task M6.5.8). Kept apart from `degraded` so the reader's routine
    /// `set_degraded(None)` after a clean pass cannot clear the enricher's reason; only
    /// `reset_enrichment` does.
    fn degraded(&self) -> Option<DegradeReason> {
        self.degraded.or(self
            .draft
            .enrichment
            .is_misaligned()
            .then_some(DegradeReason::Misaligned))
    }

    fn snapshot(&self) -> proto::Conversation {
        self.draft.to_conversation(
            self.rev,
            self.degraded(),
            self.dropped_turns,
            self.dropped_by,
        )
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
    /// first). Returns the `Drop` patches for whatever it removed. `oversize_logged` is
    /// then recomputed fresh (see its own doc comment) from whatever state the loop
    /// left behind.
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
            if removed.role == proto::Role::User {
                self.draft.dropped_user_turns = self.draft.dropped_user_turns.saturating_add(1);
            }
            self.draft.enrichment.forget_turn(removed.id);
            self.dropped_turns = self.dropped_turns.saturating_add(1);
            self.dropped_by = Some(cause);
            drops.push(TurnPatch::Drop { id: removed.id });
        }
        let is_oversize = self.draft.turns.len() == 1 && self.draft.byte_size() > caps.max_bytes;
        self.oversize_logged = is_oversize;
        if !is_oversize {
            self.oversize_reported = false;
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
    /// The reason last passed to `set_degraded`, used both to compute that call's `new`
    /// return value and to seed a conversation created afterwards (fix round 1, finding
    /// F6) -- distinct from any one entry's own `degraded` field, which can lag behind
    /// this until the entry is created or `set_degraded` runs again.
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
    /// `hook.agent_id` is `Some` and the set has room for one more conversation
    /// (`has_room_for_a_new_key`, decision A8) -- holding one empty `Running`
    /// `Assistant` turn and nothing else. Beyond the cap, no child conversation is
    /// created at all; the parent-side spawn block, which always lands regardless, is
    /// the hook's only visible effect. The child is deliberately *not* routed through
    /// `resolve_key`'s "redirect to the root" fallback the way the generic path is
    /// (below): pushing the child's seed turn into the *root's* draft instead would
    /// corrupt it with a stray empty turn that hook never meant for it.
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
            let parent_key = spawn.and_then(|origin| origin.parent_id.clone());
            if let Some(resolved) = self.touch(parent_key, caps, |draft| {
                super::build::apply(draft, runtime, hook, spawn, now_unix_secs, now, caps)
            }) {
                changed.push(resolved);
            }

            if let Some(child_id) = hook.agent_id.clone() {
                let child_key = Some(child_id);
                let already_exists = self.entries.contains_key(&child_key);
                if !already_exists
                    && self.has_room_for_a_new_key()
                    && let Some(resolved) = self.touch(child_key, caps, |draft| {
                        draft.push_turn(
                            proto::Role::Assistant,
                            proto::TurnState::Running,
                            Vec::new(),
                            now_unix_secs,
                        );
                        true
                    })
                {
                    changed.push(resolved);
                }
            }
            return changed;
        }

        self.touch(hook.agent_id.clone(), caps, |draft| {
            super::build::apply(draft, runtime, hook, spawn, now_unix_secs, now, caps)
        })
        .map(|resolved| vec![resolved])
        .unwrap_or_default()
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
    /// exceeds `caps.max_bytes` and was kept anyway (decision A9). A genuine
    /// current-state query (fix round 1, finding F7): recomputed on every cap
    /// enforcement, not merely latched the first time it was seen. `false` for an
    /// unknown key.
    pub fn oversize(&self, agent_id: Option<&str>) -> bool {
        self.entries
            .get(&agent_id.map(str::to_owned))
            .is_some_and(|entry| entry.oversize_logged)
    }

    /// Reports a transition into `oversize` exactly once: `true` the first time
    /// `agent_id`'s conversation is found oversize after not having been (or having
    /// never been checked), `false` on every subsequent call until the condition
    /// resolves (the oversize turn leaves via `max_turns`, or the conversation goes
    /// away) and recurs. Fix round 1, finding F7: `oversize()` alone gives a caller no
    /// way to log "once at `warn`" the way the brief's own wording asks for -- polling
    /// `oversize()` after every hook and logging when true would log on every hook,
    /// not once. Preferred over mirroring `set_degraded`'s `(changed, new)` shape
    /// because `oversize` is inherently per-key, not a whole-call, whole-set action the
    /// way `set_degraded` is; a bolted-on tuple return here would mean every caller of
    /// `on_hook`/`enrich` has to thread an extra value through for a condition most
    /// hooks never trigger, where `take_oversize` costs nothing until a caller actually
    /// wants to know.
    pub fn take_oversize(&mut self, agent_id: Option<&str>) -> bool {
        let Some(entry) = self.entries.get_mut(&agent_id.map(str::to_owned)) else {
            return false;
        };
        if entry.oversize_logged && !entry.oversize_reported {
            entry.oversize_reported = true;
            true
        } else {
            false
        }
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

    /// The root window's own `transcript_path`, as last set by a `SessionStart` hook --
    /// which file `watch.rs` (task M6.5.10) needs to know to start tailing. `None` when
    /// no `SessionStart` has landed for the root conversation yet, or when the root
    /// conversation does not exist yet at all. Fix round 1, finding F9: this needs no
    /// `crate::transcript::Record` (unlike `enrich`/`reset_enrichment`, still deferred
    /// to tasks M6.5.7/8) -- it only reads `Draft.transcript_path`, which this task
    /// already carries.
    pub fn transcript_path(&self) -> Option<&str> {
        self.entries.get(&None)?.draft.transcript_path.as_deref()
    }

    /// Applies transcript records (decision 1: enrichment only) to the window's root
    /// conversation, returning `[None]` when its `rev` advanced and nothing otherwise.
    ///
    /// Routing (task M6.5.8): the records come from one transcript file, and the file
    /// the reader tails is the root's (`transcript_path` above). A sub-agent's transcript
    /// is a separate file in practice (the Claude parser also skips `isSidechain` lines),
    /// so every record -- text and `ToolDetail` alike -- belongs to the root, and none
    /// reaches into a sub-agent's conversation. Never creates a conversation: records
    /// with no hook-built root to land on change nothing.
    pub fn enrich(&mut self, records: &[Record], caps: Caps) -> Vec<Option<String>> {
        let Some(entry) = self.entries.get_mut(&None) else {
            return Vec::new();
        };
        if Self::mutate(entry, Some(caps), |draft| {
            enrich::apply(draft, records, caps)
        }) {
            vec![None]
        } else {
            Vec::new()
        }
    }

    /// Drops every enrichment-sourced value on every conversation, keeping the
    /// hook-built timeline (decision A7), and returns the keys whose `rev` advanced.
    /// Only a conversation that actually held enrichment changes.
    pub fn reset_enrichment(&mut self) -> Vec<Option<String>> {
        let mut changed = Vec::new();
        for (key, entry) in &mut self.entries {
            // Undoing enrichment only shrinks a conversation, so no cap can newly bite,
            // and running `enforce_caps` without the caller's real caps would recompute
            // `oversize_logged` against the wrong bound.
            if Self::mutate(entry, None, enrich::reset) {
                changed.push(key.clone());
            }
        }
        changed
    }

    /// Whether the set has room for one more brand-new key without exceeding
    /// `MAX_CONVERSATIONS_PER_WINDOW`, reserving the root's own slot when it does not
    /// exist yet. Fix round 1, finding F2: the pre-fix version admitted a brand-new
    /// named key whenever `entries.len() < MAX_CONVERSATIONS_PER_WINDOW`, with no
    /// reservation for the root -- if 51 named keys arrived before the root's own first
    /// hook, the root's creation (via `resolve_key`'s own `None` fallback) became a
    /// 52nd conversation. Reserving one slot for the root here, in the one place both
    /// `resolve_key` and the `SubagentStart` child-creation path consult, makes that
    /// arithmetically unreachable rather than merely untested.
    fn has_room_for_a_new_key(&self) -> bool {
        let root_reserved = usize::from(!self.entries.contains_key(&None));
        self.entries.len() + root_reserved < MAX_CONVERSATIONS_PER_WINDOW
    }

    /// Resolves `key` to the key a hook naming it should actually be applied to
    /// (decision A8): an already-known key, or `None` itself, is always honored as-is;
    /// a brand-new named key is redirected to the root (`None`) key once the set has no
    /// room left for it (`has_room_for_a_new_key`). Pure -- creates nothing.
    fn resolve_key(&self, key: &Option<String>) -> Option<String> {
        if key.is_none() || self.entries.contains_key(key) || self.has_room_for_a_new_key() {
            key.clone()
        } else {
            None
        }
    }

    /// Applies `f` to the draft resolved from `key` (via `resolve_key`), returning the
    /// resolved key when `f` reports a real change, `None` otherwise. Records exactly
    /// one revision when it does: an `Upsert` for every turn `f` created or modified
    /// (found by comparing `turns` before and after, since `build::apply`'s own
    /// per-`HookKind` effects vary too widely to track surgically) plus a `Drop` for
    /// every turn `enforce_caps` trims in the same call.
    ///
    /// Fix round 1, finding F2's folded minor: a brand-new entry is no longer inserted
    /// unconditionally before `f` runs. It is built off to the side and only inserted
    /// into `entries` if `f` actually changed something -- otherwise a stream of hooks
    /// that change nothing (an unmatched `Notification`, for instance) could
    /// permanently occupy a conversation slot for an agent id that never did anything,
    /// crowding out real sub-agent conversations once `MAX_CONVERSATIONS_PER_WINDOW` is
    /// reached.
    fn touch<F>(&mut self, key: Option<String>, caps: Caps, f: F) -> Option<Option<String>>
    where
        F: FnOnce(&mut Draft) -> bool,
    {
        let resolved = self.resolve_key(&key);
        let changed = if let Some(entry) = self.entries.get_mut(&resolved) {
            Self::mutate(entry, Some(caps), f)
        } else {
            let mut entry = Entry::new(
                Draft::new(self.window_id, resolved.clone(), self.runtime),
                self.current_degraded,
            );
            let changed = Self::mutate(&mut entry, Some(caps), f);
            if changed {
                self.entries.insert(resolved.clone(), entry);
            }
            changed
        };
        changed.then_some(resolved)
    }

    /// The "diff before/after, enforce caps, record one revision" logic `touch` runs
    /// against an `Entry`, whether that entry is already in `entries` or is being
    /// evaluated off to the side before its first insertion. Returns whether `f`
    /// reported a change. `caps: None` skips cap enforcement, for a change that can only
    /// shrink the conversation (`reset_enrichment`).
    fn mutate<F>(entry: &mut Entry, caps: Option<Caps>, f: F) -> bool
    where
        F: FnOnce(&mut Draft) -> bool,
    {
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
        if let Some(caps) = caps {
            patches.extend(entry.enforce_caps(caps));
        }
        entry.record_revision(patches);
        true
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
