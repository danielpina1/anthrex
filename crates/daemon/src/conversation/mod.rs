//! Per-window conversation state (spec decision 5 onward, milestone 6.5). `summary`
//! (task M6.5.4) renders a tool's one-line description; `build` (task M6.5.5) is the
//! pure transform from one hook into one conversation's `Draft`; `store` (task M6.5.6)
//! is `ConversationSet` -- revisions, the delta ring and the caps that turn a `Draft`
//! into the `rev`/`degraded`/`dropped_turns` bookkeeping the wire protocol reports;
//! `enrich` (task M6.5.8) lays transcript records onto that hook-built timeline;
//! `watch` (task M6.5.10) runs the transcript reader while a client is subscribed.

use std::time::Instant;

/// `pub(crate)` so a headless session's cursor judges a hook's prompt against a sent
/// text the way enrichment does (M8a.7, ruling T7-N1).
pub(crate) mod align;
mod build;
mod enrich;
mod entry;
mod store;
mod summary;
pub mod watch;
pub use store::{ConversationSet, Visible};
pub use summary::{SUMMARY_MAX_GRAPHEMES, for_tool};

/// The three caps from `[conversation]`, resolved to `usize` (spec decision 7).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Caps {
    pub max_turns: usize,
    pub max_bytes: usize,
    pub max_result_bytes: usize,
}

impl Caps {
    pub fn from_config(c: &config::Conversation) -> Self {
        Caps {
            max_turns: c.max_turns as usize,
            max_bytes: c.max_bytes as usize,
            max_result_bytes: c.max_result_bytes as usize,
        }
    }
}

impl Default for Caps {
    fn default() -> Self {
        Caps::from_config(&config::Conversation::default())
    }
}

/// Decision A8, derived from the tracker's own cap rather than typed independently
/// (`docs/timing-budgets.md` standing rule 1): the tracker already refuses to hold more
/// than `MAX_SUBAGENTS` sub-agents, so `MAX_SUBAGENTS + 1` (the sub-agents plus the
/// window's own root conversation) is the most conversations a window can legitimately
/// need.
pub const MAX_CONVERSATIONS_PER_WINDOW: usize = crate::subagents::MAX_SUBAGENTS + 1;
/// Decision A3: how many revisions' patch batches `ConversationSet::delta_since` can
/// still answer without falling back to a full snapshot.
pub const DELTA_HISTORY: usize = 64;

/// One conversation's working state: everything `build::apply` (and, later, `enrich`)
/// needs to turn hooks and transcript records into `proto::Turn`s, minus the
/// revision/degrade/drop bookkeeping task M6.5.6's store adds on top.
///
/// `tool_started` is a side table, not part of `turns`, because `duration_ms` is
/// wall-clock — `PreToolUse` to `PostToolUse` — and `build::apply` reads no clock of its
/// own (it is handed `now: Instant` by its caller).
///
/// Fix round 1, finding F1: this used to be `HashMap<usize, Instant>`, keyed by the
/// `ToolCall` block's index within the open turn's `blocks`. That was safe only as long
/// as nothing ever inserted, removed or reordered a turn's `blocks` -- an invariant task
/// M6.5.8's own brief (rule 2) is specified to break (`Record::AssistantText` inserts a
/// `Text` block at the *front* of a turn with none, shifting every `ToolCall` after it
/// down by one). The review measured the exact failure: a two-pending-tool case reports
/// one tool's duration using *another* tool's start time once a front-insert has shifted
/// the indices, and a single-pending-tool case loses its duration to `None` outright,
/// both silently, with `state` still reporting `Ok`. Re-keyed by the tool's own identity
/// -- `(tool_use_id, name)` -- entries pushed by `pre_tool_use` and consumed by
/// `build::take_tool_start`, which selects the matching entry with the exact same
/// two-tier rule `find_target` uses to select the matching block (`build::
/// select_by_tier`, shared by both so the two selections cannot drift apart), immune to
/// any later block-position mutation.
///
/// Entries are removed when consumed (`take_tool_start`). Both `push_turn` and
/// `close_open_turn` also clear `tool_started` outright, and fix round 2's re-review
/// measured that this pairing is deliberately redundant, not one load-bearing clear
/// plus one no-op: with *either* clear alone, `take_tool_start`'s name-tier fallback
/// cannot mismatch a denied tool's orphaned entry (left behind because no matching
/// `PostToolUse` ever arrives) against an unrelated same-named tool in a later turn.
/// The wrong pairing only reproduces when *both* clears are deleted at once
/// (`review_denied_tool_does_not_leak_its_start_time_into_a_later_turn` in
/// `build_tests.rs`, which goes red only in that combination). Each clear is kept as
/// its own defence in depth against the other one someday being removed by a change
/// that looks locally safe -- deleting either alone is *expected* to leave the suite
/// green, and must not be read as proof the other is dead code.
struct Draft {
    window_id: u32,
    agent_id: Option<String>,
    runtime: proto::Runtime,
    session_id: Option<String>,
    transcript_path: Option<String>,
    turns: Vec<proto::Turn>,
    next_turn_id: u64,
    tool_started: Vec<(Option<String>, String, Instant)>,
    /// How many `User` turns the store's cap enforcement has dropped from the front
    /// (task M6.5.8): transcript ordinal `k` maps to surviving `User` turn
    /// `k - dropped_user_turns`. Maintained by `Entry::enforce_caps` beside
    /// `dropped_turns`, and never reset -- it describes the hook-built timeline, not
    /// enrichment.
    dropped_user_turns: u32,
    /// The absolute index (counting dropped turns) of the current session's first
    /// `User` turn: transcript ordinal `k` is that session's `k`-th prompt, and the
    /// transcript file starts again at 0 when a new session (Claude's `/clear`) starts a
    /// new file in the same window. Task M6.5.10 fix round 1, review F2.
    session_base: u32,
    /// A new session with a new transcript file started since the reader last asked
    /// (`ConversationSet::take_new_session`): the reader switches files without the
    /// restart a path change in the same session would be.
    new_session: bool,
    /// The new session's file may already hold earlier turns — any switch whose source
    /// is not `startup` or `clear` (`ConversationSet::take_at_end`, fix round 2 N1/N2,
    /// re-review 2 M1): the reader opens it at its end and the session's base is set
    /// then (`enrich::open_session_at_end`).
    at_end_pending: bool,
    /// What `enrich::apply` changed, and the alignment state it carries across
    /// batches (task M6.5.8): everything `enrich::reset` needs to undo exactly the
    /// enrichment and nothing hook-built.
    enrichment: enrich::Enrichment,
}

impl Draft {
    fn new(window_id: u32, agent_id: Option<String>, runtime: proto::Runtime) -> Self {
        Draft {
            window_id,
            agent_id,
            runtime,
            session_id: None,
            transcript_path: None,
            turns: Vec::new(),
            next_turn_id: 1,
            tool_started: Vec::new(),
            dropped_user_turns: 0,
            session_base: 0,
            new_session: false,
            at_end_pending: false,
            enrichment: enrich::Enrichment::default(),
        }
    }

    /// The index of the last turn whose `state` is `Running`; there is at most one
    /// (build.rs's "the open turn").
    fn open_turn_index(&self) -> Option<usize> {
        self.turns
            .iter()
            .rposition(|turn| turn.state == proto::TurnState::Running)
    }

    /// Appends a turn, returning its index. Also clears `tool_started` (see the struct
    /// doc comment): deliberately redundant with `close_open_turn`'s own clear, each
    /// independently sufficient to keep a denied tool's orphaned entry from leaking
    /// into a later turn -- not a no-op kept "just in case".
    fn push_turn(
        &mut self,
        role: proto::Role,
        state: proto::TurnState,
        blocks: Vec<proto::Block>,
        now_unix_secs: u64,
    ) -> usize {
        let id = self.next_turn_id;
        self.next_turn_id += 1;
        self.turns.push(proto::Turn {
            id,
            role,
            at_unix_secs: now_unix_secs,
            state,
            blocks,
        });
        self.tool_started.clear();
        self.turns.len() - 1
    }

    /// The open turn's index, opening a fresh `Running` `Assistant` turn first when
    /// there is none.
    fn ensure_open_assistant_turn(&mut self, now_unix_secs: u64) -> usize {
        match self.open_turn_index() {
            Some(index) => index,
            None => self.push_turn(
                proto::Role::Assistant,
                proto::TurnState::Running,
                Vec::new(),
                now_unix_secs,
            ),
        }
    }

    /// Appends `block` to the open turn (opening one if needed), returning the new
    /// block's index within that turn.
    fn append_to_open_turn(&mut self, block: proto::Block, now_unix_secs: u64) -> usize {
        let index = self.ensure_open_assistant_turn(now_unix_secs);
        self.turns[index].blocks.push(block);
        self.turns[index].blocks.len() - 1
    }

    /// Closes the open turn, if any: `state = Complete`, and every still-`Pending`
    /// `ToolCall` in it becomes `Denied`. Returns whether anything changed.
    ///
    /// `tool_started.clear()` below is deliberately redundant with `push_turn`'s own
    /// clear (see the struct doc comment), each independently sufficient on its own: a
    /// denied `ToolCall` never gets a `PostToolUse`, so its `tool_started` entry would
    /// otherwise never be consumed, and could later be picked up by
    /// `take_tool_start`'s name-tier fallback for an unrelated same-named tool in a
    /// *later* turn (`find_target` cannot make the same mistake, since it only ever
    /// searches the currently open turn's own blocks).
    fn close_open_turn(&mut self) -> bool {
        let Some(index) = self.open_turn_index() else {
            return false;
        };
        let turn = &mut self.turns[index];
        let mut changed = turn.state != proto::TurnState::Complete;
        turn.state = proto::TurnState::Complete;
        for block in &mut turn.blocks {
            if let proto::Block::ToolCall { state, .. } = block
                && *state == proto::ToolState::Pending
            {
                *state = proto::ToolState::Denied;
                changed = true;
            }
        }
        self.tool_started.clear();
        changed
    }

    /// Sum of `proto::Turn::byte_size` over every turn currently held -- the same
    /// measure `store.rs`'s cap enforcement bounds. Not cached: recomputed on demand,
    /// same as `proto::Conversation::byte_size` it mirrors.
    fn byte_size(&self) -> usize {
        self.turns.iter().map(proto::Turn::byte_size).sum()
    }

    /// Builds the wire `Conversation`. `rev`, `degraded`, `dropped_turns` and
    /// `dropped_by` are not `Draft`'s own state -- they are `store.rs`'s revision and
    /// cap bookkeeping, kept per-key one level up rather than here, so every caller
    /// passes them in explicitly rather than this method hard-coding a placeholder
    /// (task M6.5.6: the previous placeholders -- `rev: 0`, `degraded: None`,
    /// `dropped_turns: 0`, `dropped_by: None` -- had no test asserting any of them).
    fn to_conversation(
        &self,
        rev: u64,
        degraded: Option<proto::DegradeReason>,
        dropped_turns: u32,
        dropped_by: Option<proto::DropCause>,
    ) -> proto::Conversation {
        proto::Conversation {
            window_id: self.window_id,
            agent_id: self.agent_id.clone(),
            session_id: self.session_id.clone(),
            runtime: self.runtime,
            rev,
            degraded,
            dropped_turns,
            dropped_by,
            turns: self.turns.clone(),
        }
    }
}
