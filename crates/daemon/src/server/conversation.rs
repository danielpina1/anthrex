//! One client's conversation subscriptions (task M6.5.10), modelled on `git_task` in
//! `server.rs`: a task beside the request loop that owns the subscriptions, listens to
//! `WindowManager::conversation_changes`, and sends through the connection's `out_tx`.
//!
//! The request loop hands it `SubscribeConversation` and `UnsubscribeConversation` as
//! commands rather than answering them itself, so that a subscription's first answer and
//! every later update leave through one task, in order, and so that the per-key
//! `last_sent` revision each update is computed from has exactly one owner.

use crate::manager::{CONVERSATION_GONE, WindowManager};
use proto::DaemonMsg;
use proto::conversation::{GONE_TOO_LARGE, GONE_WINDOW_REMOVED};
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

type Key = (u32, Option<String>);

pub(super) enum Command {
    Subscribe {
        window_id: u32,
        agent_id: Option<String>,
        from_rev: Option<u64>,
    },
    Unsubscribe {
        window_id: u32,
        agent_id: Option<String>,
    },
}

/// How many commands may wait for the task. The request loop awaits room, so a client
/// that sends subscriptions faster than it reads their answers slows only itself, the
/// same backpressure its own `out_tx` already applies to the PTY subscription.
const COMMAND_QUEUE: usize = 64;

/// The task and the channel into it.
pub(super) struct ConversationTask {
    commands: mpsc::Sender<Command>,
    task: JoinHandle<()>,
}

impl ConversationTask {
    /// Subscribes to the change broadcast before returning, so no revision made after a
    /// later `Subscribe` is answered can be missed.
    pub(super) fn spawn(
        manager: Arc<WindowManager>,
        out: mpsc::Sender<DaemonMsg>,
        shutdown: CancellationToken,
    ) -> Self {
        let (commands, commands_rx) = mpsc::channel(COMMAND_QUEUE);
        let changes = manager.conversation_changes();
        let task = tokio::spawn(run(manager, commands_rx, changes, out, shutdown));
        ConversationTask { commands, task }
    }

    /// Fails only once the task has ended, which it does when the client is gone.
    pub(super) async fn send(&self, command: Command) -> bool {
        self.commands.send(command).await.is_ok()
    }

    /// Ends the task and waits until it has released every subscription it held, the
    /// way `Subscription::stop` waits for its forwarder.
    pub(super) async fn stop(self) {
        self.task.abort();
        let _ = self.task.await;
    }
}

/// The keys this client is subscribed to, each with the revision it was last sent.
/// Dropping it unsubscribes every one, exactly as `Subscription::drop` unfocuses, so an
/// aborted task still gives back the reader's viewer count.
struct Subscriptions {
    manager: Arc<WindowManager>,
    last_sent: BTreeMap<Key, u64>,
}

impl Drop for Subscriptions {
    fn drop(&mut self) {
        for (window_id, agent_id) in self.last_sent.keys() {
            self.manager
                .unsubscribe_conversation(*window_id, agent_id.as_deref());
        }
    }
}

impl Subscriptions {
    /// Records what `message` tells the client, and whether it is still subscribed.
    fn record(&mut self, key: Key, message: &DaemonMsg) {
        match message {
            DaemonMsg::ConversationSnapshot { conversation, .. } => {
                self.last_sent.insert(key, conversation.rev);
            }
            DaemonMsg::ConversationDelta { to_rev, .. } => {
                self.last_sent.insert(key, *to_rev);
            }
            // The window or sub-agent is gone. Giving the count back is a no-op for a
            // window that no longer exists, and needed for a sub-agent that stopped
            // being known while its window lives on.
            _ => {
                if self.last_sent.remove(&key).is_some() {
                    self.manager
                        .unsubscribe_conversation(key.0, key.1.as_deref());
                }
            }
        }
    }
}

async fn run(
    manager: Arc<WindowManager>,
    mut commands: mpsc::Receiver<Command>,
    mut changes: broadcast::Receiver<(u32, Option<String>, u64)>,
    out: mpsc::Sender<DaemonMsg>,
    shutdown: CancellationToken,
) {
    let mut subs = Subscriptions {
        manager: manager.clone(),
        last_sent: BTreeMap::new(),
    };
    loop {
        let outgoing: Vec<DaemonMsg> = tokio::select! {
            command = commands.recv() => match command {
                None => break,
                Some(command) => answer(&manager, &mut subs, command, &shutdown),
            },
            change = changes.recv() => match change {
                Ok((window_id, _, CONVERSATION_GONE)) => window_gone(&mut subs, window_id),
                Ok((window_id, agent_id, rev)) => {
                    let key = (window_id, agent_id);
                    match subs.last_sent.get(&key) {
                        Some(&last) if rev > last => update(&manager, &mut subs, key, last)
                            .into_iter()
                            .collect(),
                        _ => Vec::new(),
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    // Notifications are only triggers; current state is in the store.
                    tracing::debug!(missed, "client lagged on conversation broadcast; resyncing");
                    let held: Vec<(Key, u64)> =
                        subs.last_sent.iter().map(|(k, v)| (k.clone(), *v)).collect();
                    held.into_iter()
                        .filter_map(|(key, last)| update(&manager, &mut subs, key, last))
                        .collect()
                }
                Err(broadcast::error::RecvError::Closed) => break,
            },
        };
        for message in outgoing {
            let message = fit_in_frame(&mut subs, message);
            // Task M6.5.1's envelope invariant, checked on everything this task sends.
            debug_assert!(message.conversation_envelope_is_consistent(), "{message:?}");
            if out.send(message).await.is_err() {
                return;
            }
        }
    }
}

fn answer(
    manager: &Arc<WindowManager>,
    subs: &mut Subscriptions,
    command: Command,
    shutdown: &CancellationToken,
) -> Vec<DaemonMsg> {
    match command {
        Command::Subscribe {
            window_id,
            agent_id,
            from_rev,
        } => {
            let key = (window_id, agent_id.clone());
            // A client re-subscribing to a key it holds (decision A12's resync) is
            // answered without counting it as a second viewer.
            let message = if subs.last_sent.contains_key(&key) {
                manager.conversation_reply(window_id, agent_id, from_rev)
            } else {
                manager.subscribe_conversation(window_id, agent_id, from_rev, shutdown.clone())
            };
            subs.record(key, &message);
            vec![message]
        }
        Command::Unsubscribe {
            window_id,
            agent_id,
        } => {
            if subs
                .last_sent
                .remove(&(window_id, agent_id.clone()))
                .is_some()
            {
                manager.unsubscribe_conversation(window_id, agent_id.as_deref());
            }
            Vec::new()
        }
    }
}

/// Brings one key up from `last`: a delta when the store still holds `last`, a
/// snapshot otherwise, or `ConversationGone` if the window went away in between. `None`
/// when there is nothing newer than `last`.
fn update(
    manager: &WindowManager,
    subs: &mut Subscriptions,
    key: Key,
    last: u64,
) -> Option<DaemonMsg> {
    let (window_id, agent_id) = key.clone();
    let message = match manager.conversation_delta(window_id, agent_id.as_deref(), last) {
        Some((to_rev, _, _)) if to_rev == last => return None,
        Some((to_rev, turns, visible)) => crate::manager::conversation_delta_message(
            window_id, agent_id, last, to_rev, turns, visible,
        ),
        None => manager.conversation_reply(window_id, agent_id, None),
    };
    subs.record(key, &message);
    Some(message)
}

/// Every key this client holds for `window_id` ends with `GONE_WINDOW_REMOVED`. Nothing
/// is unsubscribed: the window, and its viewer count, no longer exist, and window ids
/// are never reused.
fn window_gone(subs: &mut Subscriptions, window_id: u32) -> Vec<DaemonMsg> {
    let keys: Vec<Key> = subs
        .last_sent
        .keys()
        .filter(|(w, _)| *w == window_id)
        .cloned()
        .collect();
    keys.into_iter()
        .map(|key| {
            subs.last_sent.remove(&key);
            DaemonMsg::ConversationGone {
                window_id: key.0,
                agent_id: key.1,
                reason: GONE_WINDOW_REMOVED.to_string(),
            }
        })
        .collect()
}

/// Room a snapshot's or delta's own fields take beside its turns: field names, the
/// window id, revisions, the degrade reason and drop counts, all small and fixed.
const ENVELOPE: usize = 1024;
/// A generous bound on one encoded `TurnPatch::Drop`.
const DROP_PATCH: usize = 64;
/// What wrapping a turn as a `TurnPatch::Upsert` adds to `Turn::byte_size`: the
/// one-entry map and its `"upsert"` key, 8 bytes encoded, doubled for margin
/// (re-review N3: without it, each patch's estimate could fall a byte short, and
/// thousands of them past `ENVELOPE`).
const UPSERT_WRAPPER: usize = 16;

/// A snapshot's or delta's window, key, and an upper bound on its encoding less
/// `ENVELOPE`; `None` for any other message.
fn frame_bound(message: &DaemonMsg) -> Option<(u32, Option<String>, usize)> {
    match message {
        DaemonMsg::ConversationSnapshot {
            window_id,
            agent_id,
            conversation,
        } => Some((
            *window_id,
            agent_id.clone(),
            conversation.byte_size()
                + 2 * agent_id.as_ref().map_or(0, String::len)
                + conversation.session_id.as_ref().map_or(0, String::len),
        )),
        DaemonMsg::ConversationDelta {
            window_id,
            agent_id,
            turns,
            session_id,
            ..
        } => Some((
            *window_id,
            agent_id.clone(),
            turns
                .iter()
                .map(|patch| match patch {
                    proto::TurnPatch::Upsert(turn) => turn.byte_size() + UPSERT_WRAPPER,
                    proto::TurnPatch::Drop { .. } => DROP_PATCH,
                })
                .sum::<usize>()
                + agent_id.as_ref().map_or(0, String::len)
                + session_id.as_ref().map_or(0, String::len),
        )),
        _ => None,
    }
}

/// A snapshot or delta that cannot be encoded into one frame becomes
/// `ConversationGone { GONE_TOO_LARGE }`, recorded so the viewer is given back (task
/// M6.5.10 review F1). `byte_size` is an upper bound on the encoding, so the message is
/// encoded to check only when that bound does not already prove it fits.
fn fit_in_frame(subs: &mut Subscriptions, message: DaemonMsg) -> DaemonMsg {
    let Some((window_id, agent_id, bound)) = frame_bound(&message) else {
        return message;
    };
    if bound + ENVELOPE <= proto::MAX_FRAME || proto::encode(&message).is_ok() {
        return message;
    }
    tracing::warn!(
        window_id,
        ?agent_id,
        bound,
        "conversation too large for one frame"
    );
    let gone = DaemonMsg::ConversationGone {
        window_id,
        agent_id: agent_id.clone(),
        reason: GONE_TOO_LARGE.to_string(),
    };
    subs.record((window_id, agent_id), &gone);
    gone
}

#[cfg(test)]
mod tests {
    use super::*;
    use proto::{Role, Turn, TurnPatch, TurnState};

    /// The encoded body of `message`, without the frame's length prefix.
    fn encoded(message: &DaemonMsg) -> usize {
        proto::encode(message).unwrap().len() - 4
    }

    fn delta(turns: Vec<TurnPatch>) -> DaemonMsg {
        DaemonMsg::ConversationDelta {
            window_id: u32::MAX,
            agent_id: None,
            from_rev: u64::MAX - 1,
            to_rev: u64::MAX,
            turns,
            session_id: None,
            degraded: None,
            dropped_turns: u32::MAX,
            dropped_by: None,
        }
    }

    /// Re-review N3: each `Upsert` in a delta is wrapped (`{"upsert": turn}`), which
    /// `Turn::byte_size` does not count. The smallest turn with the widest numbers shows
    /// a per-patch estimate below its encoding, and enough of them add up past
    /// `ENVELOPE`'s slack, so the bound must count the wrapper.
    #[test]
    fn a_deltas_bound_covers_every_upserts_wrapper() {
        let upsert = TurnPatch::Upsert(Turn {
            id: u64::MAX,
            role: Role::Assistant,
            at_unix_secs: u64::MAX,
            state: TurnState::Complete,
            blocks: Vec::new(),
        });
        let one = encoded(&delta(vec![upsert.clone()])) - encoded(&delta(Vec::new()));
        let estimate = frame_bound(&delta(vec![upsert.clone()])).unwrap().2;
        assert!(
            estimate >= one,
            "one Upsert: estimate {estimate} bytes, encoded {one} bytes"
        );

        let many = delta(vec![upsert; 4096]);
        let bound = frame_bound(&many).unwrap().2;
        assert!(
            bound + ENVELOPE >= encoded(&many),
            "4096 Upserts: bound {} bytes, encoded {} bytes",
            bound + ENVELOPE,
            encoded(&many)
        );
    }

    #[test]
    fn a_deltas_bound_covers_a_drop() {
        let drop = TurnPatch::Drop { id: u64::MAX };
        let one = encoded(&delta(vec![drop.clone()])) - encoded(&delta(Vec::new()));
        let estimate = frame_bound(&delta(vec![drop])).unwrap().2;
        assert!(estimate >= one, "estimate {estimate}, encoded {one}");
    }
}
