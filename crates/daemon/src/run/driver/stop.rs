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
}
