//! The run driver's event loop (M9.11.7, moved from `driver.rs`): the loop that receives
//! each message in turn, `handle` (one event: the step, then its effects), and `forward`
//! (the manager's session feed, translated).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::super::engine::{Event, EventKind};
use super::super::slots;
use super::{Msg, RunService, guard, translate, unix_now};
use crate::headless::SessionEvent;
use crate::manager::{TerminalRecord, WindowSignal, WindowSignalKind};

impl RunService {
    /// The event loop's body: each message in turn, until `Stop` or the channel closes.
    /// `gate` is held for the loop's whole life (`RunService::loop_gate`).
    pub(super) async fn event_loop(
        self: Arc<Self>,
        gate: Option<tokio::sync::OwnedMutexGuard<()>>,
        rx: Option<mpsc::UnboundedReceiver<Msg>>,
    ) {
        let service = self;
        // Shared with each event's own task, so the gate opens only once both the loop
        // and the event it was carrying out are gone (`stop`'s fallback waits on it).
        let gate = gate.map(Arc::new);
        let Some(mut rx) = rx else { return };
        while let Some(msg) = rx.recv().await {
            match msg {
                Msg::Event(kind) => service.handle_isolated(kind, &gate).await,
                Msg::Usage => service.drain_usage().await,
                Msg::Stop(ack) => {
                    service.stop_now().await;
                    let _ = ack.send(());
                    return;
                }
            }
        }
    }

    /// Decision 8 (M9.11.8, B1): one event's handling on a task of its own, awaited before
    /// the next message is received, so events stay strictly sequential. A panic anywhere
    /// in it (its effects, the tick's work) costs that event: it is logged with the
    /// event's kind, and a caller still waiting is told [`guard::EFFECTS_PANICKED`] (its
    /// step had been committed; a panic in the step itself is answered inside `handle`).
    /// The spawn is the unwind boundary: no `catch_unwind` around an `.await`.
    async fn handle_isolated(
        self: &Arc<Self>,
        kind: EventKind,
        gate: &Option<Arc<tokio::sync::OwnedMutexGuard<()>>>,
    ) {
        let reply = guard::reply_of(&kind);
        let event = guard::kind_name(&kind);
        let (service, gate) = (self.clone(), gate.clone());
        let mut task = AbortOnDrop(tokio::spawn(async move {
            let _gate = gate;
            service.handle(kind).await;
        }));
        let Err(error) = (&mut task.0).await else {
            return;
        };
        if !error.is_panic() {
            return;
        }
        let panic = guard::panic_text(error.into_panic().as_ref());
        tracing::error!(event, %panic, "the run engine panicked carrying out an event; the event is dropped");
        let waiting = reply.and_then(|id| crate::lock(&self.replies).remove(&id));
        if let Some(tx) = waiting {
            let _ = tx.send(Err(guard::EFFECTS_PANICKED.to_string()));
        }
    }

    /// One event: a step under the engine lock, then its effects with the lock released.
    pub(super) async fn handle(self: &Arc<Self>, kind: EventKind) {
        let now = unix_now();
        let tick = matches!(kind, EventKind::Tick);
        let kind = slots::stamp(self.mark_killed(kind), self.scheduler.slots());
        let reply = guard::reply_of(&kind);
        let prepared = {
            let mut state = crate::lock(&self.state);
            // Decision 8 (B3): a step whose effects could not be prepared at all is put
            // back with them, so memory never holds a step whose saves did not run.
            match guard::step_prepared(&mut state, Event { now, kind }, now) {
                Ok(ready) => {
                    self.metered.refresh_live(&state);
                    self.release_ended_orchestrators(&state);
                    Some(self.mark_writes(ready))
                }
                Err(panic) => {
                    tracing::error!(%panic, "the run engine panicked on an event; the event is dropped");
                    None
                }
            }
        };
        let Some(prepared) = prepared else {
            let waiting = reply.and_then(|id| crate::lock(&self.replies).remove(&id));
            if let Some(tx) = waiting {
                let _ = tx.send(Err(guard::STEP_PANICKED.to_string()));
            }
            return;
        };
        self.execute(prepared, now).await;
        if tick {
            self.on_tick(now).await;
        }
    }

    /// The manager's session feed, translated (`observe.rs`) and sent in the order it
    /// arrives. Decision 9 (M9.11.9, A12): on a lag it takes a fresh receiver and the
    /// manager's terminal records at one cut, and sends each exit, and each process's
    /// last turn end, that the lag hid from it ([`Forwarded::catch_up`]).
    pub(super) async fn forward(
        self: Arc<Self>,
        mut signals: broadcast::Receiver<WindowSignal>,
        shutdown: CancellationToken,
    ) {
        let mut last_activity = HashMap::new();
        let mut forwarded = Forwarded::default();
        loop {
            let received = tokio::select! {
                _ = shutdown.cancelled() => return,
                received = signals.recv() => received,
            };
            let caught_up = match received {
                Ok(signal) => {
                    forwarded.note(&signal);
                    if forwarded.prune_due() {
                        forwarded.prune(&self.manager.headless_terminal_events());
                    }
                    vec![signal]
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    let (fresh, records) = self.manager.signals_and_terminal_events();
                    signals = fresh;
                    let caught_up = forwarded.catch_up(&records);
                    let caught = caught_up.len();
                    tracing::warn!(
                        missed,
                        caught,
                        "the run engine lagged on the session feed; caught up from the manager"
                    );
                    caught_up
                }
                Err(broadcast::error::RecvError::Closed) => return,
            };
            for signal in caught_up {
                if let Some(agent) = translate(&signal, &mut last_activity, Instant::now()) {
                    self.send(EventKind::Signal {
                        window_id: signal.window_id,
                        signal: agent,
                    });
                }
            }
        }
    }
}

/// Decision 9: what `forward` has forwarded of each `(window, pid)`'s terminal events:
/// how many prompted turn ends, and whether its exit.
#[derive(Default)]
pub(super) struct Forwarded {
    seen: HashMap<(u32, u32), (u64, bool)>,
    prune_at: usize,
}

/// `Forwarded` drops the processes the manager no longer records once it holds more
/// than this many (or twice what was left at the last prune).
const PRUNE_FLOOR: usize = 1024;

impl Forwarded {
    /// Notes a signal about to be forwarded.
    pub(super) fn note(&mut self, signal: &WindowSignal) {
        let (WindowSignalKind::Session(event), Some(pid)) = (&signal.kind, signal.pid) else {
            return;
        };
        let key = (signal.window_id, pid);
        match event {
            SessionEvent::TurnEnded { .. } => self.seen.entry(key).or_default().0 += 1,
            SessionEvent::ProcessExited { .. } => self.seen.entry(key).or_default().1 = true,
            _ => {}
        }
    }

    /// The signals a lag hid, from the manager's `records`: for each process, its last
    /// turn end when the manager applied more than were forwarded (the turn ends in
    /// between are coalesced into it), then its exit when that was not forwarded. Each
    /// is noted, so it is never sent twice; processes the manager no longer records are
    /// forgotten.
    pub(super) fn catch_up(&mut self, records: &[TerminalRecord]) -> Vec<WindowSignal> {
        let mut out = Vec::new();
        let signal = |r: &TerminalRecord, event: &SessionEvent| WindowSignal {
            window_id: r.window_id,
            pid: Some(r.pid),
            kind: WindowSignalKind::Session(event.clone()),
        };
        for r in records {
            let seen = self.seen.entry((r.window_id, r.pid)).or_default();
            if r.turn_ends > seen.0 {
                seen.0 = r.turn_ends;
                out.extend(r.last_turn_end.as_ref().map(|e| signal(r, e)));
            }
            if let Some(exit) = r.exited.as_ref().filter(|_| !seen.1) {
                seen.1 = true;
                out.push(signal(r, exit));
            }
        }
        self.prune(records);
        out
    }

    fn prune_due(&self) -> bool {
        self.seen.len() > self.prune_at.max(PRUNE_FLOOR)
    }

    fn prune(&mut self, records: &[TerminalRecord]) {
        let kept: HashSet<(u32, u32)> = records.iter().map(|r| (r.window_id, r.pid)).collect();
        self.seen.retain(|key, _| kept.contains(key));
        self.prune_at = self.seen.len() * 2;
    }
}

/// An event's task, aborted if the loop awaiting it is (`stop`'s fallback aborts the loop).
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[cfg(test)]
#[path = "event_loop_tests.rs"]
mod tests;
