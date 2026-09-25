//! A connection's run requests (M8a.22). Each request is answered on its own task
//! through a weak handle on the connection's outgoing channel, so a slow git read never
//! stalls the connection's loop, and never holds a closed connection open. `Subscribe` forwards every snapshot the service publishes
//! (decision 47) until `Unsubscribe` or the connection ends.

use crate::run::driver::RunService;
use proto::{DaemonMsg, RunReply, RunRequest};
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinHandle;

/// One connection's run traffic.
pub(super) struct RunApi {
    runs: Arc<RunService>,
    out_tx: mpsc::Sender<DaemonMsg>,
    subscription: Option<JoinHandle<()>>,
}

impl RunApi {
    pub(super) fn new(runs: Arc<RunService>, out_tx: mpsc::Sender<DaemonMsg>) -> Self {
        RunApi {
            runs,
            out_tx,
            subscription: None,
        }
    }

    pub(super) fn handle(&mut self, request: RunRequest) {
        match request {
            RunRequest::Subscribe => {
                self.unsubscribe();
                self.subscription = Some(tokio::spawn(forward(
                    self.runs.clone(),
                    self.out_tx.clone(),
                )));
            }
            RunRequest::Unsubscribe => self.unsubscribe(),
            request => {
                // A weak sender (ruling T22-minors, m1): the request carries on after
                // the client leaves, but never keeps its connection's writer alive.
                let (runs, out_tx) = (self.runs.clone(), self.out_tx.downgrade());
                tokio::spawn(async move {
                    let reply = runs.request(request).await;
                    if let Some(out_tx) = out_tx.upgrade() {
                        let _ = out_tx.send(DaemonMsg::Run(reply)).await;
                    }
                });
            }
        }
    }

    fn unsubscribe(&mut self) {
        if let Some(task) = self.subscription.take() {
            task.abort();
        }
    }
}

impl Drop for RunApi {
    fn drop(&mut self) {
        self.unsubscribe();
    }
}

/// The current snapshot at once, then every one published after it, each with a
/// revision above the last sent. A receiver that lags resynchronises from the latest.
async fn forward(runs: Arc<RunService>, out_tx: mpsc::Sender<DaemonMsg>) {
    let mut pushes = runs.pushes();
    let first = runs.current();
    let mut sent = first.revision;
    if out_tx
        .send(DaemonMsg::Run(RunReply::Snapshot(first)))
        .await
        .is_err()
    {
        return;
    }
    loop {
        let snapshot = match pushes.recv().await {
            Ok(snapshot) => (*snapshot).clone(),
            Err(broadcast::error::RecvError::Lagged(_)) => runs.current(),
            Err(broadcast::error::RecvError::Closed) => return,
        };
        if snapshot.revision <= sent {
            continue;
        }
        sent = snapshot.revision;
        if out_tx
            .send(DaemonMsg::Run(RunReply::Snapshot(snapshot)))
            .await
            .is_err()
        {
            return;
        }
    }
}
