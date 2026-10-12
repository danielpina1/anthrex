//! Decision 46: stopping the run service, `RunService::stop` and the loop's own
//! `stop_now`, which saves every run's last `run.json`.

use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::sync::oneshot;

use super::{Msg, RunService, effects, guard, unix_now};
use crate::run::engine::{Event, EventKind};

impl RunService {
    /// Decision 46: after `stop` the service ignores every event, and the last
    /// `run.json` of each run is the one written here.
    pub async fn stop(&self) {
        // A model probe in flight ends now; `ProbeChild` kills and reaps its CLI.
        self.models_cancel.cancel();
        if self.saved.load(Ordering::SeqCst) {
            return;
        }
        let (ack, done) = oneshot::channel();
        if self.tx.send(Msg::Stop(ack)).is_ok()
            && tokio::time::timeout(
                Duration::from_millis(self.stop_wait_ms.load(Ordering::SeqCst)),
                done,
            )
            .await
            .is_ok_and(|r| r.is_ok())
        {
            return;
        }
        // No acknowledgement: the loop is gone, never ran, or is stuck. It is aborted and
        // awaited first, so no `run.json` is written here beside a live loop (ruling
        // T22-minors, m8). A blocking write the loop had already handed to a thread
        // cannot be cancelled; it runs to its end.
        let abort = crate::lock(&self.loop_abort).take();
        if let Some(abort) = abort {
            abort.abort();
        }
        drop(self.loop_gate.lock().await);
        // Ruling T22-N2: only saves that finished count. A loop stopped inside
        // `stop_now` has set `stopped`, but its saves may not all have run.
        if self.saved.load(Ordering::SeqCst) {
            return;
        }
        self.stop_now().await;
    }

    pub(super) async fn stop_now(&self) {
        let runs = {
            let mut state = crate::lock(&self.state);
            let stop = Event {
                now: unix_now(),
                kind: EventKind::Stop,
            };
            if guard::guarded_step(&mut state, stop).is_err() {
                // The state is as it was; `stopped` below still ends the service.
                tracing::error!("the run engine panicked on stop");
            }
            state.runs.values().cloned().collect::<Vec<_>>()
        };
        self.stopped.store(true, Ordering::SeqCst);
        for run in runs {
            effects::save(&self.writes, run).await;
        }
        self.write_due_reports(unix_now(), true).await;
        self.saved.store(true, Ordering::SeqCst);
        let waiting: Vec<_> = crate::lock(&self.replies).drain().collect();
        for (_, reply) in waiting {
            let _ = reply.send(Err("the daemon is shutting down".to_string()));
        }
    }

    /// Milestone 9.14 task 11: waits for every op the loop started. Each holds its run's
    /// order lock (`op_lock`) for its whole life, so taking each exclusively waits for
    /// it. For test rigs, after [`RunService::stop`]: the daemon's process exit ends its
    /// ops, but a test's directory must outlive them. Unbounded; the caller bounds it.
    /// An Accept or Discard op spawned but not yet scheduled may be missed; the caller
    /// must bound the wait.
    #[doc(hidden)]
    pub async fn wait_for_ops(&self) {
        let orders: Vec<_> = crate::lock(&self.op_order).values().cloned().collect();
        for order in orders {
            drop(order.write_owned().await);
        }
    }

    /// How many runs have an op holding their order lock at this moment (M9.14.11): a
    /// test rig's `stop()` checks it is none once it returns.
    /// An Accept or Discard op spawned but not yet scheduled may be missed; the caller
    /// must bound the wait.
    #[doc(hidden)]
    pub fn ops_in_flight(&self) -> usize {
        let orders: Vec<_> = crate::lock(&self.op_order).values().cloned().collect();
        orders.iter().filter(|o| o.try_write().is_err()).count()
    }
}
