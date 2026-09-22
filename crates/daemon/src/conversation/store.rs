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

pub use super::entry::Visible;
use super::entry::{Entry, compact_patches};
use super::{Caps, DELTA_HISTORY, Draft, MAX_CONVERSATIONS_PER_WINDOW, enrich};
use crate::hooks::{HookKind, ParsedHook};
use crate::subagents::SpawnOrigin;
use crate::transcript::Record;
use proto::{DegradeReason, TurnPatch};
use std::collections::HashMap;
use std::time::Instant;

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
    /// Every hook also retries the draft's parked transcript records
    /// (`enrich::retry`, task M6.5.8 fix round 1, F1) inside the hook's own revision, so
    /// a record read before its hook lands the moment the hook does, without waiting for
    /// another transcript line, and costs no extra revision.
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
                    | super::enrich::retry(draft)
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
                | super::enrich::retry(draft)
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
            // The stored reason always follows the reader; a revision only when what a
            // client sees moved with it (task M6.5.10's one rule, `mutate`).
            let before = entry.visible();
            entry.degraded = reason;
            if entry.visible() != before {
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

    /// The conversation's current revision, without building a snapshot. `None` for an
    /// unknown key.
    pub fn rev(&self, agent_id: Option<&str>) -> Option<u64> {
        self.entries
            .get(&agent_id.map(str::to_owned))
            .map(|e| e.rev)
    }

    /// The fields a delta carries beside its patches, at their current values. `None`
    /// for an unknown key.
    pub fn visible(&self, agent_id: Option<&str>) -> Option<Visible> {
        self.entries
            .get(&agent_id.map(str::to_owned))
            .map(Entry::visible)
    }

    /// A snapshot of `agent_id`'s conversation, or of the empty conversation it would
    /// start as (rev 0, no turns, the set's current degrade reason) when no hook has
    /// touched it yet. What a subscriber is sent for a window whose agent has not said
    /// anything, so that a subscription always answers with a conversation (task
    /// M6.5.10's `a_shell_window_has_an_empty_degraded_conversation`). Built by the
    /// same `Entry::new` a first hook would use, so the two agree on revision 0.
    pub fn snapshot_or_empty(&self, agent_id: Option<&str>) -> proto::Conversation {
        let key = agent_id.map(str::to_owned);
        match self.entries.get(&key) {
            Some(entry) => entry.snapshot(),
            None => Entry::new(
                Draft::new(self.window_id, key, self.runtime),
                self.current_degraded,
            )
            .snapshot(),
        }
    }

    /// Every key this set currently holds a conversation for.
    pub fn keys(&self) -> Vec<Option<String>> {
        self.entries.keys().cloned().collect()
    }

    /// The root window's own `transcript_path`, as last set by a `SessionStart` hook --
    /// which file `watch.rs` (task M6.5.10) needs to know to start tailing. `None` when
    /// no `SessionStart` has landed for the root conversation yet, or when the root
    /// conversation does not exist yet at all. It only reads `Draft.transcript_path`.
    pub fn transcript_path(&self) -> Option<&str> {
        self.entries.get(&None)?.draft.transcript_path.as_deref()
    }

    /// Whether a new session with its own transcript file has started since the last
    /// call (review F2), clearing the flag. The reader asks when the path it tails no
    /// longer matches: `true` means switch files as a new session, `false` means the same
    /// session moved files, which is a restart.
    pub fn take_new_session(&mut self) -> bool {
        self.entries
            .get_mut(&None)
            .is_some_and(|entry| std::mem::take(&mut entry.draft.new_session))
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
        if entry
            .mutate(Some(caps), |draft| enrich::apply(draft, records, caps))
            .revised
        {
            vec![None]
        } else {
            Vec::new()
        }
    }

    /// The reader's restart (decision A7) as one step: drop every enrichment-sourced
    /// value, then apply `records` from the start of the file, and record a revision only
    /// if the result differs from what a client saw before (task M6.5.10's one rule). A
    /// file rewritten with the same content therefore costs no revision at all, where
    /// `reset_enrichment` then `enrich` would cost two. A re-read longer than one
    /// `TRANSCRIPT_READ_BUDGET` pass still shows its later turns un-enriched until the
    /// passes that bring them back, and each of those is a real, visible change.
    pub fn restart_enrichment(&mut self, records: &[Record], caps: Caps) -> Vec<Option<String>> {
        let mut changed = Vec::new();
        for (key, entry) in &mut self.entries {
            let revised = if key.is_none() {
                entry.mutate(Some(caps), |draft| {
                    enrich::reset(draft) | enrich::apply(draft, records, caps)
                })
            } else {
                entry.mutate(None, enrich::reset)
            }
            .revised;
            if revised {
                changed.push(key.clone());
            }
        }
        changed
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
            if entry.mutate(None, enrich::reset).revised {
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
        let mutation = if let Some(entry) = self.entries.get_mut(&resolved) {
            entry.mutate(Some(caps), f)
        } else {
            let mut entry = Entry::new(
                Draft::new(self.window_id, resolved.clone(), self.runtime),
                self.current_degraded,
            );
            let mutation = entry.mutate(Some(caps), f);
            // Kept whenever its state changed, visibly or not: a `SessionStart` that only
            // sets `transcript_path` is no revision, but the reader needs the path.
            if mutation.changed {
                self.entries.insert(resolved.clone(), entry);
            }
            mutation
        };
        mutation.revised.then_some(resolved)
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
