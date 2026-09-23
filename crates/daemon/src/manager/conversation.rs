//! The conversation half of the manager (task M6.5.10): subscriptions, the change
//! broadcast, and the transcript reader's two touch points with the window table.
//!
//! Split out of `manager/mod.rs` for AGENTS.md hard rule 8. Everything here that takes
//! the manager lock does only in-memory work under it: the transcript file is read by
//! `crate::conversation::watch` on the blocking pool, between [`WindowManager::reader_step`]
//! (which hands the reader its `Tail`) and [`WindowManager::apply_transcript`] (which
//! takes it back), and the lock is held across neither the read nor anything else that
//! can stall (AGENTS.md hard rule 2).

use super::WindowManager;
use super::entry::Entry;
use crate::conversation::{Caps, ConversationSet, Visible};
use crate::transcript::parser_for;
use crate::transcript::reader::{ReadOutcome, Tail};
use proto::conversation::{GONE_SUBAGENT_UNKNOWN, GONE_WINDOW_REMOVED, GONE_WINDOW_UNKNOWN};
use proto::{DaemonMsg, DegradeReason, TurnPatch};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};
use tokio_util::sync::CancellationToken;

/// The `rev` a `conversation_changes` notification carries when the window it names is
/// gone: every conversation of that window has ended, whatever the notification's
/// `agent_id`. A per-client task turns it into one `ConversationGone` per key it holds
/// for the window (`GONE_WINDOW_REMOVED`). A real `rev` counts one per hook or read
/// pass, so it cannot reach this.
pub const CONVERSATION_GONE: u64 = u64::MAX;

/// Capacity of the change broadcast, the same shape as the git broadcast in
/// `server.rs`. A receiver that lags is resynchronised from current state, so the number
/// bounds memory, not correctness.
pub(super) const CONVERSATION_CHANGES_CAPACITY: usize = 1024;

/// One window's transcript reader bookkeeping (decision 9).
#[derive(Default)]
pub(super) struct TranscriptSlot {
    /// Where the reader is in the file. `None` while a pass holds it on the blocking
    /// pool, and before the first pass. Kept here rather than in the reader task so a
    /// reader that stops after its linger and a later one that starts again continue
    /// from the same offset instead of re-reading the file.
    tail: Option<Tail>,
    /// The next outcome is applied as a restart (`ConversationSet::restart_enrichment`)
    /// even if the tail did not report one: the path changed, or a pass was lost to a
    /// panic and the file is being read again from the start.
    restart_next: bool,
    /// A reader task exists for this window. Set and cleared only under the manager
    /// lock, which is what stops two readers racing on one window.
    reader_running: bool,
    /// When the window's last conversation subscriber left, while the reader lingers.
    idle_since: Option<Instant>,
    /// The file the last applied pass read, for observing from outside which file the
    /// reader is on (`conversation_transcript_read`).
    last_read: Option<PathBuf>,
    /// The file the current session's reader opened at its end, if it did. A `Tail`
    /// rebuilt for it without a switch (after `transcript_lost`) is opened at the end
    /// again, never read from the start at the open-time base (re-review 2, M2).
    at_end_path: Option<PathBuf>,
}

/// What the reader does next ([`WindowManager::reader_step`]).
pub enum ReaderStep {
    /// Nobody is subscribed and the linger has run out, or the window is gone.
    Stop,
    /// Nothing to read yet (no `transcript_path`); poll again.
    Wait,
    /// Read one pass from this tail, then hand it back through `apply_transcript`.
    Read(Tail),
}

fn now_unix_secs() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl WindowManager {
    /// A broadcast of `(window_id, agent_id, rev)` for every conversation revision, and
    /// `(window_id, None, CONVERSATION_GONE)` when a window is removed.
    pub fn conversation_changes(
        &self,
    ) -> tokio::sync::broadcast::Receiver<(u32, Option<String>, u64)> {
        self.conversations.subscribe()
    }

    pub(super) fn caps(&self) -> Caps {
        Caps::from_config(&self.config.conversation)
    }

    /// Sends one notification per key whose `rev` advanced. Never blocks: a broadcast
    /// send waits for no receiver.
    pub(super) fn notify_conversations(
        &self,
        window_id: u32,
        set: &ConversationSet,
        keys: &[Option<String>],
    ) {
        for key in keys {
            if let Some(rev) = set.rev(key.as_deref()) {
                let _ = self.conversations.send((window_id, key.clone(), rev));
            }
        }
    }

    /// Every conversation of a removed window has ended.
    pub(super) fn notify_window_gone(&self, window_id: u32) {
        let _ = self
            .conversations
            .send((window_id, None, CONVERSATION_GONE));
    }

    /// Subscribes a client to one conversation: increments the window's conversation
    /// viewer count and, on 0 → 1, spawns the transcript reader (decision 9). Returns the
    /// message to send, which is a `ConversationGone` (and subscribes nothing) for an
    /// unknown window or sub-agent. A client re-sending a subscription it already holds
    /// must use [`conversation_reply`](Self::conversation_reply) instead, or the count
    /// would go up twice for one subscriber.
    pub fn subscribe_conversation(
        self: &Arc<Self>,
        window_id: u32,
        agent_id: Option<String>,
        from_rev: Option<u64>,
        shutdown: CancellationToken,
    ) -> DaemonMsg {
        let mut inner = crate::lock(&self.inner);
        let Some(entry) = inner.entries.get_mut(&window_id) else {
            return gone(window_id, agent_id, GONE_WINDOW_UNKNOWN);
        };
        if !knows(entry, agent_id.as_deref()) {
            return gone(window_id, agent_id, GONE_SUBAGENT_UNKNOWN);
        }
        entry.conversation_viewers = entry.conversation_viewers.saturating_add(1);
        entry.transcript.idle_since = None;
        if entry.conversations.transcript_path().is_none() {
            // A shell, or an agent that has not reported its transcript yet: known
            // without reading anything, so the first answer already says so.
            self.degrade(window_id, entry, Some(DegradeReason::NoTranscriptPath));
        }
        let parser = parser_for(entry.spec.runtime);
        let spawn = parser.filter(|_| !entry.transcript.reader_running);
        if spawn.is_some() {
            entry.transcript.reader_running = true;
        }
        let reply = reply(entry, window_id, agent_id, from_rev);
        drop(inner);
        if let Some(parser) = spawn {
            crate::conversation::watch::spawn_reader(self.clone(), window_id, parser, shutdown);
        }
        reply
    }

    /// Decrements. On 1 → 0 the reader keeps going for `conversation.linger_secs`, then
    /// stops between passes (see `reader_step`).
    pub fn unsubscribe_conversation(&self, window_id: u32, agent_id: Option<&str>) {
        let _ = agent_id;
        let mut inner = crate::lock(&self.inner);
        if let Some(entry) = inner.entries.get_mut(&window_id) {
            entry.conversation_viewers = entry.conversation_viewers.saturating_sub(1);
            if entry.conversation_viewers == 0 {
                entry.transcript.idle_since = Some(Instant::now());
            }
        }
    }

    /// The message that brings a client holding `from_rev` up to date, without
    /// subscribing anything: a `ConversationDelta` when the store still holds that
    /// revision and it is behind, otherwise a full `ConversationSnapshot` — including
    /// when `from_rev` is already current, because a delta whose two revisions are the
    /// same number is a shape this protocol never sends.
    pub fn conversation_reply(
        &self,
        window_id: u32,
        agent_id: Option<String>,
        from_rev: Option<u64>,
    ) -> DaemonMsg {
        let inner = crate::lock(&self.inner);
        match inner.entries.get(&window_id) {
            None => gone(window_id, agent_id, GONE_WINDOW_REMOVED),
            Some(entry) if !knows(entry, agent_id.as_deref()) => {
                gone(window_id, agent_id, GONE_SUBAGENT_UNKNOWN)
            }
            Some(entry) => reply(entry, window_id, agent_id, from_rev),
        }
    }

    pub fn conversation_snapshot(
        &self,
        window_id: u32,
        agent_id: Option<&str>,
    ) -> Option<proto::Conversation> {
        let inner = crate::lock(&self.inner);
        inner
            .entries
            .get(&window_id)?
            .conversations
            .snapshot(agent_id)
    }

    /// The patches after `from_rev`, the revision they reach and the delta's
    /// always-current fields. `None` when the store no longer holds `from_rev` (or the
    /// key or window is unknown): the caller sends a snapshot.
    pub fn conversation_delta(
        &self,
        window_id: u32,
        agent_id: Option<&str>,
        from_rev: u64,
    ) -> Option<(u64, Vec<TurnPatch>, Visible)> {
        let inner = crate::lock(&self.inner);
        let set = &inner.entries.get(&window_id)?.conversations;
        let (to_rev, patches) = set.delta_since(agent_id, from_rev)?;
        Some((to_rev, patches, set.visible(agent_id)?))
    }

    /// Whether a transcript reader task exists for this window, for observing decision
    /// 9's "only while subscribed, plus the linger" from outside.
    pub fn conversation_reader_running(&self, window_id: u32) -> bool {
        let inner = crate::lock(&self.inner);
        inner
            .entries
            .get(&window_id)
            .is_some_and(|entry| entry.transcript.reader_running)
    }

    /// The transcript file the reader's last applied pass read, if any. Lets a caller
    /// wait for the reader to have switched files, rather than sleeping.
    pub fn conversation_transcript_read(&self, window_id: u32) -> Option<PathBuf> {
        let inner = crate::lock(&self.inner);
        inner.entries.get(&window_id)?.transcript.last_read.clone()
    }

    /// The reader's decision point, between passes and never during one: stop, wait, or
    /// take the tail for one pass. Under the lock only long enough to decide and to move
    /// the `Tail` out.
    ///
    /// `conversation.linger_secs` may legitimately be 0. The reader then stops at its
    /// next step after the last subscriber leaves: after the pass in flight finishes,
    /// never mid-pass, so a reader is gone within `linger_secs + TRANSCRIPT_POLL +
    /// TRANSCRIPT_READ_TIMEOUT` (`conversation/watch.rs`, `transcript/reader.rs`) of the
    /// last unsubscribe, and a pass's result is always applied.
    pub fn reader_step(&self, window_id: u32) -> ReaderStep {
        let linger = Duration::from_secs(self.config.conversation.linger_secs);
        let mut inner = crate::lock(&self.inner);
        let Some(entry) = inner.entries.get_mut(&window_id) else {
            return ReaderStep::Stop;
        };
        if entry.conversation_viewers == 0
            && entry
                .transcript
                .idle_since
                .is_none_or(|since| since.elapsed() >= linger)
        {
            entry.transcript.reader_running = false;
            entry.transcript.idle_since = None;
            return ReaderStep::Stop;
        }
        let Some(path) = entry.conversations.transcript_path().map(PathBuf::from) else {
            self.degrade(window_id, entry, Some(DegradeReason::NoTranscriptPath));
            return ReaderStep::Wait;
        };
        // Both flags are consumed at every step, so one left from a switch the reader
        // never saw cannot excuse a later same-session move from its restart. Either one
        // pending means the session changed since the kept `Tail` was made, even when the
        // path is the same again (re-review 2, I2: `/clear` then `/resume` back while
        // nobody watched), so that `Tail` is never continued.
        let resume = entry.conversations.take_at_end();
        let new_session = entry.conversations.take_new_session();
        match entry.transcript.tail.take() {
            Some(tail) if tail.path() == path && !resume && !new_session => {
                return ReaderStep::Read(tail);
            }
            // Another file. A new session's (Claude's `/clear`) is read from its start
            // with the old session's enrichment kept (review F2); the same session in
            // another file is a restart, so nothing the old file enriched outlives it.
            Some(tail) if tail.path() != path => entry.transcript.restart_next = !new_session,
            _ => {}
        }
        // A resumed session's file already holds its earlier turns, so it is opened at
        // its end (fix round 2, N1/N2), and so is a `Tail` rebuilt for that same file
        // with no switch since, after a lost pass (re-review 2, M2).
        let at_end = resume
            || (!new_session && entry.transcript.at_end_path.as_deref() == Some(path.as_path()));
        entry.transcript.at_end_path = at_end.then(|| path.clone());
        ReaderStep::Read(if at_end {
            Tail::at_end(path)
        } else {
            Tail::new(path)
        })
    }

    /// Applies one pass's outcome and takes the tail back. The records enrich the root
    /// conversation only (task M6.5.8's routing): the file is the root's.
    pub fn apply_transcript(&self, window_id: u32, tail: Tail, outcome: ReadOutcome) {
        let caps = self.caps();
        let mut inner = crate::lock(&self.inner);
        let Some(entry) = inner.entries.get_mut(&window_id) else {
            return;
        };
        if entry
            .conversations
            .transcript_path()
            .map(std::path::Path::new)
            != Some(tail.path())
        {
            // The session moved to another file while this pass read the old one; the
            // next step starts the new file, as a restart unless it is a new session's.
            entry.transcript.restart_next = !entry.conversations.take_new_session();
            return;
        }
        let restart = outcome.restarted || std::mem::take(&mut entry.transcript.restart_next);
        let set = &mut entry.conversations;
        let changed = if outcome.opened_at_end {
            set.open_at_end(&outcome.records, restart, caps)
        } else if restart {
            set.restart_enrichment(&outcome.records, caps)
        } else {
            set.enrich(&outcome.records, caps)
        };
        if set.take_oversize(None) {
            tracing::warn!(
                window_id,
                "a single conversation turn exceeds conversation.max_bytes; kept"
            );
        }
        entry.transcript.last_read = Some(tail.path().to_path_buf());
        entry.transcript.tail = Some(tail);
        self.notify_conversations(window_id, &entry.conversations, &changed);
        self.degrade(window_id, entry, outcome.degraded);
    }

    /// A pass panicked on the blocking pool and took its `Tail` with it. The next pass
    /// reads the file from the start, applied as a restart so nothing is enriched twice.
    pub fn transcript_lost(&self, window_id: u32) {
        let mut inner = crate::lock(&self.inner);
        if let Some(entry) = inner.entries.get_mut(&window_id) {
            entry.transcript.restart_next = true;
        }
    }

    /// Sets the window's degrade reason, logging once per distinct reason (decision 2)
    /// and notifying every conversation whose `rev` advanced.
    fn degrade(&self, window_id: u32, entry: &mut Entry, reason: Option<DegradeReason>) {
        let (changed, is_new) = entry.conversations.set_degraded(reason);
        if is_new {
            match reason {
                Some(reason) => tracing::warn!(window_id, ?reason, "conversation degraded"),
                None => tracing::info!(window_id, "conversation no longer degraded"),
            }
        }
        self.notify_conversations(window_id, &entry.conversations, &changed);
    }

    /// Drives one parsed hook into the window's conversations (`handle_hook`), then
    /// notifies each key whose `rev` advanced.
    pub(super) fn conversation_hook(
        &self,
        window_id: u32,
        entry: &mut Entry,
        hook: &crate::hooks::ParsedHook,
        now: Instant,
    ) {
        let spawn = hook
            .agent_id
            .as_deref()
            .and_then(|id| entry.state.subagents.spawn_origin(id));
        let changed = entry.conversations.on_hook(
            entry.spec.runtime,
            hook,
            spawn.as_ref(),
            now_unix_secs(),
            now,
            self.caps(),
        );
        for key in &changed {
            if entry.conversations.take_oversize(key.as_deref()) {
                tracing::warn!(window_id, agent_id = ?key, "a single conversation turn exceeds conversation.max_bytes; kept");
            }
        }
        self.notify_conversations(window_id, &entry.conversations, &changed);
    }
}

/// A key a client may subscribe to: the root always, a sub-agent once a hook or the
/// sub-agent tracker has named it.
fn knows(entry: &Entry, agent_id: Option<&str>) -> bool {
    match agent_id {
        None => true,
        Some(id) => {
            entry.conversations.rev(Some(id)).is_some()
                || entry.state.subagents.spawn_origin(id).is_some()
        }
    }
}

fn gone(window_id: u32, agent_id: Option<String>, reason: &str) -> DaemonMsg {
    DaemonMsg::ConversationGone {
        window_id,
        agent_id,
        reason: reason.to_string(),
    }
}

fn reply(
    entry: &Entry,
    window_id: u32,
    agent_id: Option<String>,
    from_rev: Option<u64>,
) -> DaemonMsg {
    let set = &entry.conversations;
    let key = agent_id.as_deref();
    if let Some(from_rev) = from_rev
        && set.rev(key).is_some_and(|rev| rev > from_rev)
        && let (Some((to_rev, turns)), Some(visible)) =
            (set.delta_since(key, from_rev), set.visible(key))
    {
        return conversation_delta_message(window_id, agent_id, from_rev, to_rev, turns, visible);
    }
    DaemonMsg::ConversationSnapshot {
        window_id,
        conversation: set.snapshot_or_empty(key),
        agent_id,
    }
}

/// Builds a `ConversationDelta` from `conversation_delta`'s parts.
pub fn conversation_delta_message(
    window_id: u32,
    agent_id: Option<String>,
    from_rev: u64,
    to_rev: u64,
    turns: Vec<TurnPatch>,
    visible: Visible,
) -> DaemonMsg {
    DaemonMsg::ConversationDelta {
        window_id,
        agent_id,
        from_rev,
        to_rev,
        turns,
        session_id: visible.session_id,
        degraded: visible.degraded,
        dropped_turns: visible.dropped_turns,
        dropped_by: visible.dropped_by,
    }
}
