//! The run driver's event loop (M9.11.7, moved from `driver.rs`): the loop that receives
//! each message in turn, `handle` (one event: the step, then its effects), and `forward`
//! (the manager's session feed, translated).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Instant;

use tokio::sync::{broadcast, mpsc};
use tokio_util::sync::CancellationToken;

use super::super::engine::{Event, EventKind};
use super::super::slots;
use super::{Msg, RunService, guard, translate, unix_now};
use crate::manager::WindowSignal;

impl RunService {
    /// The event loop's body: each message in turn, until `Stop` or the channel closes.
    /// `gate` is held for the loop's whole life (`RunService::loop_gate`).
    pub(super) async fn event_loop(
        self: Arc<Self>,
        gate: Option<tokio::sync::OwnedMutexGuard<()>>,
        rx: Option<mpsc::UnboundedReceiver<Msg>>,
    ) {
        let service = self;
        let _gate = gate;
        let Some(mut rx) = rx else { return };
        while let Some(msg) = rx.recv().await {
            match msg {
                Msg::Event(kind) => service.handle(kind).await,
                Msg::Usage => service.drain_usage().await,
                Msg::Stop(ack) => {
                    service.stop_now().await;
                    let _ = ack.send(());
                    return;
                }
            }
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
            match guard::guarded_step(&mut state, Event { now, kind }) {
                Ok(fx) => {
                    self.metered.refresh_live(&state);
                    self.release_ended_orchestrators(&state);
                    Some(self.mark_writes(guard::prepare_guarded(&state, fx, now)))
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
    /// arrives.
    pub(super) async fn forward(
        self: Arc<Self>,
        mut signals: broadcast::Receiver<WindowSignal>,
        shutdown: CancellationToken,
    ) {
        let mut last_activity = HashMap::new();
        loop {
            let received = tokio::select! {
                _ = shutdown.cancelled() => return,
                received = signals.recv() => received,
            };
            match received {
                Ok(signal) => {
                    if let Some(agent) = translate(&signal, &mut last_activity, Instant::now()) {
                        self.send(EventKind::Signal {
                            window_id: signal.window_id,
                            signal: agent,
                        });
                    }
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    tracing::warn!(missed, "the run engine lagged on the session feed");
                }
                Err(broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}
