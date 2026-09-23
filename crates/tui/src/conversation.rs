//! The conversation view's state (task M6.5.12): which conversation is shown, the
//! sub-agent trail, folding, the cursor and search. Pure — no I/O; every update returns
//! `Vec<Effect>`, and the only effects it ever returns are `SubscribeConversation` and
//! `UnsubscribeConversation` (spec decision 11: the view is read-only).

use crate::app::Effect;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use proto::{Block, ClientMsg, Conversation, DegradeReason, DropCause, TurnPatch};
use std::collections::BTreeSet;

mod rows;
pub use rows::{Cursor, DetailKind, Row};

/// One step down the sub-agent trail, taken from the `SubagentSpawn` block descended into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Crumb {
    pub agent_id: String,
    pub label: String,
}

/// `/`'s state. `hits` are cursors, not row indices (decision A10's rule, applied to
/// search): a delta that shifts rows must not move a hit onto a different block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Search {
    pub query: String,
    pub typing: bool,
    pub hits: Vec<Cursor>,
    pub hit: usize,
}

/// One conversation on the trail: the root (`agent_id: None`) or a sub-agent.
#[derive(Debug, Clone, Default)]
struct Level {
    agent_id: Option<String>,
    conversation: Option<Conversation>,
    /// Decision A10: keyed by `(turn_id, block_index)`, never by row.
    unfolded: BTreeSet<(u64, usize)>,
    /// Decision A11. `None` follows the newest row.
    cursor: Option<Cursor>,
    /// Set whenever this level's `SubscribeConversation` is outstanding — at open, at
    /// descent and at a resubscribe (decision A12) — until its snapshot lands, so deltas
    /// already in flight are dropped quietly instead of each asking for a snapshot.
    awaiting_snapshot: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ConversationView {
    open: bool,
    window_id: u32,
    /// `levels[0]` is the root; `levels[i + 1]` belongs to `trail[i]`.
    levels: Vec<Level>,
    trail: Vec<Crumb>,
    search: Option<Search>,
    gone_reason: Option<String>,
}

impl ConversationView {
    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn open(&mut self, window_id: u32) -> Vec<Effect> {
        let mut effects = self.close();
        *self = ConversationView {
            open: true,
            window_id,
            levels: vec![Level {
                awaiting_snapshot: true,
                ..Level::default()
            }],
            ..ConversationView::default()
        };
        effects.push(self.subscribe(None));
        effects
    }

    pub fn close(&mut self) -> Vec<Effect> {
        if !self.open {
            return vec![];
        }
        let effects = self
            .levels
            .iter()
            .rev()
            .map(|level| self.unsubscribe(level.agent_id.clone()))
            .collect();
        self.reset();
        effects
    }

    /// Back to closed, keeping only `gone_reason` for the caller to show.
    fn reset(&mut self) {
        *self = ConversationView {
            gone_reason: self.gone_reason.take(),
            ..ConversationView::default()
        };
    }

    fn subscribe(&self, agent_id: Option<String>) -> Effect {
        Effect::Send(ClientMsg::SubscribeConversation {
            window_id: self.window_id,
            agent_id,
            from_rev: None,
        })
    }

    fn unsubscribe(&self, agent_id: Option<String>) -> Effect {
        Effect::Send(ClientMsg::UnsubscribeConversation {
            window_id: self.window_id,
            agent_id,
        })
    }

    fn top(&self) -> Option<&Level> {
        self.levels.last()
    }

    /// The trail level a message for `(window_id, agent_id)` belongs to, if any.
    fn level_index(&self, window_id: u32, agent_id: &Option<String>) -> Option<usize> {
        if !self.open || window_id != self.window_id {
            return None;
        }
        self.levels.iter().position(|l| &l.agent_id == agent_id)
    }

    pub fn trail(&self) -> &[Crumb] {
        &self.trail
    }

    pub fn search(&self) -> Option<&Search> {
        self.search.as_ref()
    }

    pub fn gone_reason(&self) -> Option<&str> {
        self.gone_reason.as_deref()
    }

    /// The conversation currently shown, once its snapshot has arrived.
    pub fn conversation(&self) -> Option<&Conversation> {
        self.top()?.conversation.as_ref()
    }

    pub fn cursor(&self) -> Option<&Cursor> {
        self.top()?.cursor.as_ref()
    }

    /// Selects a row by what it shows (decision A11); task M6.5.13's mouse uses this.
    pub fn set_cursor(&mut self, cursor: Cursor) {
        if let Some(top) = self.levels.last_mut() {
            top.cursor = Some(cursor);
        }
    }

    pub fn is_unfolded(&self, turn_id: u64, block: usize) -> bool {
        self.top()
            .is_some_and(|top| top.unfolded.contains(&(turn_id, block)))
    }

    pub fn key(&self) -> Option<(u32, Option<String>)> {
        let top = self.top().filter(|_| self.open)?;
        Some((self.window_id, top.agent_id.clone()))
    }

    pub fn rev(&self) -> Option<u64> {
        self.conversation().map(|c| c.rev)
    }

    /// Derived afresh from the conversation and the fold set; never stored.
    pub fn rows(&self) -> Vec<Row> {
        match self.top() {
            Some(Level {
                conversation: Some(conversation),
                unfolded,
                ..
            }) => rows::rows(conversation, unfolded),
            _ => vec![],
        }
    }

    /// The row the cursor names; if that row is gone, the nearest row after where it
    /// was; if nothing follows, the last row. No cursor follows the newest row.
    ///
    /// One exception: a `Line` whose block still has rows stays on that block — its last
    /// remaining line, or its `Tool` row once folded. Moving on to the next block would
    /// put the selection on a different call, or on a spawn `Enter` would descend into.
    pub fn selected_row(&self, rows: &[Row]) -> Option<usize> {
        if rows.is_empty() {
            return None;
        }
        let last = rows.len() - 1;
        let Some(cursor) = self.cursor() else {
            return Some(last);
        };
        let target = cursor.order();
        if let Cursor::Line(turn, block, _) = cursor {
            let same_block = |row: &Row| match row.cursor() {
                Cursor::Block(t, b) | Cursor::Line(t, b, _) => (t, b) == (*turn, *block),
                _ => false,
            };
            if let Some(at) = rows
                .iter()
                .rposition(|row| same_block(row) && row.cursor().order() <= target)
            {
                return Some(at);
            }
        }
        Some(
            rows.iter()
                .position(|row| row.cursor().order() >= target)
                .unwrap_or(last),
        )
    }

    /// Keys while the view is open and no modal is (the table in task M6.5.12's brief).
    /// Returns only `SubscribeConversation` / `UnsubscribeConversation` (spec decision 11).
    pub fn on_key(&mut self, key: KeyEvent) -> Vec<Effect> {
        // Ruling N4: a key with Ctrl or Alt held is never one of the view's keys — not
        // `q`, not `j`, not a character of the query. Shift is how capitals arrive.
        if !self.open
            || key
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return vec![];
        }
        if self.search.as_ref().is_some_and(|s| s.typing) {
            self.on_search_key(key);
            return vec![];
        }
        match key.code {
            KeyCode::Char('j') | KeyCode::Down => self.move_by(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_by(-1),
            KeyCode::Char('g') => self.move_to(|_| 0),
            KeyCode::Char('G') => self.move_to(|len| len - 1),
            KeyCode::Enter | KeyCode::Char('o') => return self.activate(),
            KeyCode::Esc => return self.back(),
            KeyCode::Char('q') => return self.close(),
            KeyCode::Char('/') => {
                self.search = Some(Search {
                    query: String::new(),
                    typing: true,
                    hits: vec![],
                    hit: 0,
                });
            }
            KeyCode::Char('n') => self.walk_hits(1),
            KeyCode::Char('N') => self.walk_hits(-1),
            _ => {}
        }
        vec![]
    }

    fn move_by(&mut self, delta: isize) {
        let rows = self.rows();
        if let Some(current) = self.selected_row(&rows) {
            let next = current.saturating_add_signed(delta).min(rows.len() - 1);
            self.set_cursor(rows[next].cursor());
        }
    }

    fn move_to(&mut self, index: impl FnOnce(usize) -> usize) {
        let rows = self.rows();
        if !rows.is_empty() {
            self.set_cursor(rows[index(rows.len())].cursor());
        }
    }

    /// `Enter`/`o`: fold a tool call, or descend into a spawn. Anything else: nothing.
    fn activate(&mut self) -> Vec<Effect> {
        let rows = self.rows();
        let Some(index) = self.selected_row(&rows) else {
            return vec![];
        };
        match &rows[index] {
            Row::Tool { turn_id, block } => {
                let key = (*turn_id, *block);
                let top = self.levels.last_mut().expect("an open view has a level");
                if !top.unfolded.remove(&key) {
                    top.unfolded.insert(key);
                }
                top.cursor = Some(Cursor::Block(*turn_id, *block));
                vec![]
            }
            Row::Spawn { turn_id, block, .. } => self.descend(*turn_id, *block),
            _ => vec![],
        }
    }

    /// Pushes a crumb from the `SubagentSpawn` at `(turn_id, block)` and subscribes to
    /// it. The parent stays subscribed so `Esc` returns to it live.
    fn descend(&mut self, turn_id: u64, block: usize) -> Vec<Effect> {
        let spawn = self.conversation().and_then(|c| {
            let turn = c.turns.iter().find(|t| t.id == turn_id)?;
            match turn.blocks.get(block)? {
                Block::SubagentSpawn {
                    agent_id, label, ..
                } => Some((agent_id.clone(), label.clone())),
                _ => None,
            }
        });
        let Some((agent_id, label)) = spawn else {
            return vec![];
        };
        // A key already on the trail (malformed data: an agent that spawns itself or an
        // ancestor) would put two levels on one daemon subscription; refuse it.
        if self
            .levels
            .iter()
            .any(|level| level.agent_id.as_deref() == Some(agent_id.as_str()))
        {
            return vec![];
        }
        self.trail.push(Crumb {
            agent_id: agent_id.clone(),
            label,
        });
        self.levels.push(Level {
            agent_id: Some(agent_id.clone()),
            awaiting_snapshot: true,
            ..Level::default()
        });
        self.search = None;
        self.gone_reason = None;
        vec![self.subscribe(Some(agent_id))]
    }

    /// `Esc`: leave search, else pop a crumb, else close.
    fn back(&mut self) -> Vec<Effect> {
        if self.search.take().is_some() {
            return vec![];
        }
        if self.trail.is_empty() {
            return self.close();
        }
        self.pop()
    }

    fn pop(&mut self) -> Vec<Effect> {
        self.trail.pop();
        let level = self.levels.pop().expect("a crumb has a level");
        self.search = None;
        vec![self.unsubscribe(level.agent_id)]
    }

    fn on_search_key(&mut self, key: KeyEvent) {
        let search = self.search.as_mut().expect("typing implies a search");
        match key.code {
            KeyCode::Esc => {
                self.search = None;
                return;
            }
            KeyCode::Enter => {
                search.typing = false;
                search.hit = 0;
                if let Some(first) = search.hits.first().cloned() {
                    self.set_cursor(first);
                }
                return;
            }
            KeyCode::Backspace => {
                search.query.pop();
            }
            KeyCode::Char(c) => search.query.push(c),
            _ => return,
        }
        self.refresh_search(self.levels.len() - 1);
    }

    fn walk_hits(&mut self, step: isize) {
        let Some(search) = self.search.as_mut().filter(|s| !s.hits.is_empty()) else {
            return;
        };
        let len = search.hits.len() as isize;
        search.hit = (search.hit as isize + step).rem_euclid(len) as usize;
        let cursor = search.hits[search.hit].clone();
        self.set_cursor(cursor);
    }

    pub fn on_snapshot(
        &mut self,
        window_id: u32,
        agent_id: Option<String>,
        mut conversation: Conversation,
    ) -> Vec<Effect> {
        let Some(index) = self.level_index(window_id, &agent_id) else {
            return vec![];
        };
        // Row order follows turn ids (see `rows.rs`), so hold them in that order.
        conversation.turns.sort_by_key(|t| t.id);
        let level = &mut self.levels[index];
        level.conversation = Some(conversation);
        level.awaiting_snapshot = false;
        self.refresh_search(index);
        vec![]
    }

    /// Applies a delta to whichever trail level it belongs to; a delta for a key not on
    /// the trail is ignored. One whose `from_rev` is not the level's `rev` is discarded
    /// and answered with a resubscribe for a full snapshot (decision A12).
    #[allow(clippy::too_many_arguments)]
    pub fn on_delta(
        &mut self,
        window_id: u32,
        agent_id: Option<String>,
        from_rev: u64,
        to_rev: u64,
        turns: Vec<TurnPatch>,
        session_id: Option<String>,
        degraded: Option<DegradeReason>,
        dropped_turns: u32,
        dropped_by: Option<DropCause>,
    ) -> Vec<Effect> {
        let Some(index) = self.level_index(window_id, &agent_id) else {
            return vec![];
        };
        let level = &mut self.levels[index];
        if level.awaiting_snapshot {
            return vec![];
        }
        let Some(conversation) = level.conversation.as_mut().filter(|c| c.rev == from_rev) else {
            level.awaiting_snapshot = true;
            return vec![self.subscribe(agent_id)];
        };
        for patch in turns {
            match patch {
                TurnPatch::Upsert(turn) => {
                    match conversation.turns.binary_search_by_key(&turn.id, |t| t.id) {
                        Ok(at) => conversation.turns[at] = turn,
                        Err(at) => conversation.turns.insert(at, turn),
                    }
                }
                TurnPatch::Drop { id } => {
                    conversation.turns.retain(|t| t.id != id);
                    level.unfolded.retain(|(turn, _)| *turn != id);
                }
            }
        }
        conversation.rev = to_rev;
        conversation.session_id = session_id;
        conversation.degraded = degraded;
        conversation.dropped_turns = dropped_turns;
        conversation.dropped_by = dropped_by;
        self.refresh_search(index);
        vec![]
    }

    /// Recomputes search hits when `index` is the level on screen, keeping the current
    /// hit on the same block when it still matches.
    fn refresh_search(&mut self, index: usize) {
        if index + 1 != self.levels.len() {
            return;
        }
        let Some(search) = self.search.as_mut() else {
            return;
        };
        let hits = match &self.levels[index].conversation {
            Some(conversation) => rows::search_hits(conversation, &search.query),
            None => vec![],
        };
        let current = search.hits.get(search.hit);
        search.hit = current
            .and_then(|c| hits.iter().position(|h| h == c))
            .unwrap_or(0);
        search.hits = hits;
    }

    /// The daemon ended a subscription (`ConversationGone`). For a sub-agent on the trail,
    /// that level and every one below it are popped; for the root, the view closes. Only
    /// the deeper levels, which the daemon did not end, are unsubscribed. The reason is
    /// kept in `gone_reason` for the caller to show.
    pub fn on_gone(
        &mut self,
        window_id: u32,
        agent_id: Option<String>,
        reason: String,
    ) -> Vec<Effect> {
        let Some(index) = self.level_index(window_id, &agent_id) else {
            return vec![];
        };
        let effects = self.levels[index + 1..]
            .iter()
            .rev()
            .map(|level| self.unsubscribe(level.agent_id.clone()))
            .collect();
        self.gone_reason = Some(reason);
        if index == 0 {
            self.reset();
        } else {
            self.levels.truncate(index);
            self.trail.truncate(index - 1);
            self.search = None;
        }
        effects
    }
}

#[cfg(test)]
#[path = "conversation_tests.rs"]
mod tests;
