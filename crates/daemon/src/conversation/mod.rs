//! Per-window conversation state (spec decision 5 onward, milestone 6.5). `summary`
//! (task M6.5.4) renders a tool's one-line description; `build` (task M6.5.5) is the
//! pure transform from one hook into one conversation's `Draft`. Later tasks in this
//! milestone add `store`, `enrich` and `watch` beside them.

use std::collections::HashMap;
use std::time::Instant;

mod build;
mod summary;
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

/// One conversation's working state: everything `build::apply` (and, later, `enrich`)
/// needs to turn hooks and transcript records into `proto::Turn`s, minus the
/// revision/degrade/drop bookkeeping task M6.5.6's store adds on top.
///
/// `tool_started` is a side table, not part of `turns`, because `duration_ms` is
/// wall-clock — `PreToolUse` to `PostToolUse` — and `build::apply` reads no clock of its
/// own (it is handed `now: Instant` by its caller). It is keyed by the `ToolCall`
/// block's index within the *currently open* turn's `blocks`, and is cleared whenever a
/// new turn is pushed (decision: those indices belong to the turn that was open when
/// they were recorded, not to any turn that opens after it).
struct Draft {
    window_id: u32,
    agent_id: Option<String>,
    runtime: proto::Runtime,
    session_id: Option<String>,
    transcript_path: Option<String>,
    turns: Vec<proto::Turn>,
    next_turn_id: u64,
    tool_started: HashMap<usize, Instant>,
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
            tool_started: HashMap::new(),
        }
    }

    /// The index of the last turn whose `state` is `Running`; there is at most one
    /// (build.rs's "the open turn").
    fn open_turn_index(&self) -> Option<usize> {
        self.turns
            .iter()
            .rposition(|turn| turn.state == proto::TurnState::Running)
    }

    /// Appends a turn, returning its index. Always clears `tool_started`: a fresh
    /// turn's blocks start a new index space, and any indices recorded against the
    /// previous open turn no longer mean anything.
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

    fn to_conversation(&self) -> proto::Conversation {
        proto::Conversation {
            window_id: self.window_id,
            agent_id: self.agent_id.clone(),
            session_id: self.session_id.clone(),
            runtime: self.runtime,
            rev: 0,
            degraded: None,
            dropped_turns: 0,
            dropped_by: None,
            turns: self.turns.clone(),
        }
    }
}

/// Every conversation belonging to one window, keyed by `agent_id` (decision A1).
/// Entirely pure: no I/O, no clock of its own -- every entry point takes
/// `now_unix_secs` and `now: Instant`.
///
/// Task M6.5.5 gives this only what `build::apply`'s own tests need in order to drive
/// hook routing across more than one conversation: a `SubagentStart` touches both the
/// parent's conversation (a `SubagentSpawn` block) and the child's (a fresh, empty
/// conversation). The revision counter, delta history, degrade tracking and cap
/// enforcement this type's final shape (see `docs/milestones/M6.5-conversation-view.md`)
/// needs belong to task M6.5.6, the store, and are not implemented here.
pub struct ConversationSet {
    window_id: u32,
    runtime: proto::Runtime,
    drafts: HashMap<Option<String>, Draft>,
}

impl ConversationSet {
    pub fn new(window_id: u32, runtime: proto::Runtime) -> Self {
        ConversationSet {
            window_id,
            runtime,
            drafts: HashMap::new(),
        }
    }

    /// Applies one hook. `spawn_parent` is the `agent_id` of the parent of
    /// `hook.agent_id`, resolved by the caller from `SubagentTracker::parent_of`, and is
    /// read only for a `SubagentStart`. Returns every key whose draft changed.
    ///
    /// A `SubagentStart` is the one hook that spans two conversations: `build::apply`'s
    /// own `SubagentStart` effect (appending a `SubagentSpawn` block to the open turn)
    /// is applied to the *parent's* draft unconditionally -- per the brief's table, the
    /// parent-side effect does not depend on `hook.agent_id` being present (wave-1
    /// review finding F9: an earlier version skipped it entirely when `agent_id` was
    /// absent, so the parent never learned a sub-agent started, while `build::
    /// subagent_start` itself already defends that same case with
    /// `unwrap_or_default()` -- two layers disagreeing on whether the case is handled
    /// was the bug). The *child's* draft, which does need a real id, is created fresh
    /// here, directly, only when `hook.agent_id` is `Some` -- holding one empty
    /// `Running` `Assistant` turn and nothing else. Running `build::apply` on the child
    /// too would append a second, spurious spawn block to its own conversation instead
    /// of its parent's.
    pub fn on_hook(
        &mut self,
        runtime: proto::Runtime,
        hook: &crate::hooks::ParsedHook,
        spawn_parent: Option<&str>,
        now_unix_secs: u64,
        now: Instant,
        caps: Caps,
    ) -> Vec<Option<String>> {
        if hook.kind == crate::hooks::HookKind::SubagentStart {
            let mut changed = Vec::new();
            let parent_key = spawn_parent.map(str::to_owned);
            let parent = self.draft_mut(parent_key.clone());
            if build::apply(parent, runtime, hook, now_unix_secs, now, caps) {
                changed.push(parent_key);
            }
            if let Some(child_id) = hook.agent_id.clone() {
                let child_key = Some(child_id);
                if !self.drafts.contains_key(&child_key) {
                    let child = self.draft_mut(child_key.clone());
                    child.push_turn(
                        proto::Role::Assistant,
                        proto::TurnState::Running,
                        Vec::new(),
                        now_unix_secs,
                    );
                    changed.push(child_key);
                }
            }
            return changed;
        }

        let key = hook.agent_id.clone();
        let draft = self.draft_mut(key.clone());
        if build::apply(draft, runtime, hook, now_unix_secs, now, caps) {
            vec![key]
        } else {
            Vec::new()
        }
    }

    /// A snapshot of the conversation at `agent_id`, or `None` when no hook has ever
    /// touched that key.
    pub fn snapshot(&self, agent_id: Option<&str>) -> Option<proto::Conversation> {
        self.drafts
            .get(&agent_id.map(str::to_owned))
            .map(Draft::to_conversation)
    }

    /// Every key this set currently holds a conversation for.
    pub fn keys(&self) -> Vec<Option<String>> {
        self.drafts.keys().cloned().collect()
    }

    fn draft_mut(&mut self, key: Option<String>) -> &mut Draft {
        let window_id = self.window_id;
        let runtime = self.runtime;
        self.drafts
            .entry(key.clone())
            .or_insert_with(|| Draft::new(window_id, key, runtime))
    }
}
