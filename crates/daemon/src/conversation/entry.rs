//! One conversation's revision bookkeeping, split out of `store.rs` (task M6.5.10 fix
//! round 1, AGENTS.md rule 8): the `Entry` a `ConversationSet` holds per key, the one
//! revision rule (`Entry::mutate`), the cap enforcement and the delta ring's compaction.
//! `store.rs` keeps the set itself: routing by key, and the public entry points.

use super::{Caps, DELTA_HISTORY, Draft};
use proto::{DegradeReason, DropCause, TurnPatch};
use std::collections::{HashMap, VecDeque};

/// One revision's patch batch, tagged with the revision it belongs to so
/// `delta_since` can select the range it needs out of the ring without also keeping a
/// separate index.
#[derive(Debug, Clone)]
pub(super) struct PatchBatch {
    pub(super) rev: u64,
    pub(super) patches: Vec<TurnPatch>,
}

/// Everything a client can observe of one conversation besides its turns: the fields a
/// snapshot and a delta both carry at their current values (decision A4, and
/// `session_id` since task M6.5.10). With the turns, it is the whole of what the one
/// revision rule compares (see `ConversationSet::mutate`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Visible {
    pub session_id: Option<String>,
    pub degraded: Option<DegradeReason>,
    pub dropped_turns: u32,
    pub dropped_by: Option<DropCause>,
}

/// What `ConversationSet::mutate` did: whether the draft's state changed at all, and
/// whether a client could see it (a new revision).
pub(super) struct Mutation {
    pub(super) changed: bool,
    pub(super) revised: bool,
}

/// One conversation's full bookkeeping: `draft` is the hook-built timeline; everything
/// else is this store's own layer on top of it.
pub(super) struct Entry {
    pub(super) draft: Draft,
    pub(super) rev: u64,
    pub(super) degraded: Option<DegradeReason>,
    pub(super) dropped_turns: u32,
    pub(super) dropped_by: Option<DropCause>,
    /// Whether this conversation *currently* holds a single turn that alone exceeds
    /// `caps.max_bytes` and was kept anyway (decision A9's "never trimmed to zero").
    /// Fix round 1, finding F7: recomputed fresh on every `enforce_caps` call (not
    /// merely set-once-true), so it is a genuine current-state query -- `oversize()`'s
    /// own doc comment previously warned it could not be used that way.
    pub(super) oversize_logged: bool,
    /// Whether the *current* `oversize_logged == true` streak has already been reported
    /// through `take_oversize`. Reset to `false` whenever `oversize_logged` goes back to
    /// `false`, so a later re-entry into oversize is reported again -- "exactly once per
    /// transition into oversize" (fix round 1, finding F7).
    pub(super) oversize_reported: bool,
    /// The last `DELTA_HISTORY` revisions' patch batches, oldest first.
    pub(super) history: VecDeque<PatchBatch>,
}

impl Entry {
    /// `degraded` seeds from `ConversationSet::current_degraded` (fix round 1, finding
    /// F6): a conversation created after the day's last `set_degraded` call used to
    /// start at `None` regardless and could only ever catch up if `set_degraded` was
    /// called *again* -- which never happens for a reason that stopped applying between
    /// calls. Seeding at creation means a late-created conversation is correct from its
    /// very first revision, with no extra revision bump needed (it starts at `rev 0`
    /// either way).
    pub(super) fn new(draft: Draft, degraded: Option<DegradeReason>) -> Self {
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
    pub(super) fn degraded(&self) -> Option<DegradeReason> {
        self.degraded.or(self
            .draft
            .enrichment
            .is_misaligned()
            .then_some(DegradeReason::Misaligned))
    }

    pub(super) fn visible(&self) -> Visible {
        Visible {
            session_id: self.draft.session_id.clone(),
            degraded: self.degraded(),
            dropped_turns: self.dropped_turns,
            dropped_by: self.dropped_by,
        }
    }

    pub(super) fn snapshot(&self) -> proto::Conversation {
        self.draft.to_conversation(
            self.rev,
            self.degraded(),
            self.dropped_turns,
            self.dropped_by,
        )
    }

    /// Increments `rev` by exactly 1 and records `patches` as that revision's batch,
    /// evicting the oldest batch once the ring holds `DELTA_HISTORY` of them.
    pub(super) fn record_revision(&mut self, patches: Vec<TurnPatch>) {
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
    pub(super) fn enforce_caps(&mut self, caps: Caps) -> Vec<TurnPatch> {
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

    /// The "diff before/after, enforce caps, record one revision" logic `ConversationSet::touch` runs
    /// against an entry, whether it is already in the set or is being evaluated off to
    /// the side before its first insertion. `caps: None` skips cap
    /// enforcement, for a change that can only shrink the conversation
    /// (`reset_enrichment`).
    ///
    /// **One rule for revisions: no observable change, no new `rev`** (task M6.5.10).
    /// A revision is recorded only when a turn differs or `Visible` does, compared
    /// before and after; `f` reporting a change is necessary, not sufficient. That one
    /// comparison settles the three cases that used to advance `rev` with nothing to
    /// see: a `SessionStart` that changes only `transcript_path` (task M6.5.6 F4), a
    /// restart that re-applies the same records (`restart_enrichment`), and a
    /// `Misaligned` set underneath the reader's own degrade reason, which the reader's
    /// reason hides (task M6.5.8 F4). `set_degraded` compares `Visible` the same way.
    pub(super) fn mutate<F>(&mut self, caps: Option<Caps>, f: F) -> Mutation
    where
        F: FnOnce(&mut Draft) -> bool,
    {
        let visible = self.visible();
        let before: HashMap<u64, proto::Turn> = self
            .draft
            .turns
            .iter()
            .map(|turn| (turn.id, turn.clone()))
            .collect();
        if !f(&mut self.draft) {
            return Mutation {
                changed: false,
                revised: false,
            };
        }
        let mut patches: Vec<TurnPatch> = self
            .draft
            .turns
            .iter()
            .filter(|turn| before.get(&turn.id) != Some(turn))
            .map(|turn| TurnPatch::Upsert(turn.clone()))
            .collect();
        if let Some(caps) = caps {
            patches.extend(self.enforce_caps(caps));
        }
        let revised = !patches.is_empty() || self.visible() != visible;
        if revised {
            self.record_revision(patches);
        }
        Mutation {
            changed: true,
            revised,
        }
    }
}

/// Concatenates a delta window's patches with the compaction `delta_since` promises:
/// only the last `Upsert` per turn id, and no `Upsert` for a turn a later `Drop`
/// removed. Patches are folded in revision order, which is also the order `patches`
/// arrives in (each batch's own patches, batches already filtered and ordered by the
/// caller).
pub(super) fn compact_patches(patches: impl Iterator<Item = TurnPatch>) -> Vec<TurnPatch> {
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
