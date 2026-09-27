//! M8b.15 review (I3, I4): the run service as the OTLP receiver's [`UsageSink`].
//!
//! - **Live runs.** The ids of the runs in the engine's state that have not ended
//!   ([`RunState::is_terminal`](proto::RunState::is_terminal)), refreshed under the
//!   engine lock after every step. The receiver reads them under their own small lock,
//!   never the engine's or the window manager's, and a generation tells it when to
//!   evict the runs that went.
//! - **Coalescing.** Pending totals per run, the latest winning. One `Msg::Usage` is
//!   queued when the map goes from empty to not empty, and the event loop drains the
//!   map into one `OrchestratorUsage` per run. A flood of posts therefore adds at most
//!   one message to the engine's queue, and the map holds at most one total per live
//!   run.

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use proto::TokenUsage;

use super::{Msg, RunService};
use crate::metering::UsageSink;
use crate::run::engine::{EngineState, EventKind};

/// See the module doc.
#[derive(Default)]
pub(super) struct Metered {
    live: Mutex<HashSet<String>>,
    generation: AtomicU64,
    pending: Mutex<BTreeMap<String, TokenUsage>>,
}

impl Metered {
    /// Called under the engine lock after a step: the live runs are `state`'s runs that
    /// have not ended. The generation moves only when they changed.
    pub(super) fn refresh_live(&self, state: &EngineState) {
        let open = || state.runs.values().filter(|run| !run.state.is_terminal());
        let mut live = crate::lock(&self.live);
        if open().count() == live.len() && open().all(|run| live.contains(&run.id)) {
            return;
        }
        *live = open().map(|run| run.id.clone()).collect();
        self.generation.fetch_add(1, Ordering::SeqCst);
    }

    /// Keeps `usage` as `run_id`'s pending total, replacing an earlier one. True when
    /// nothing was pending, so a drain must be queued.
    fn offer(&self, run_id: String, usage: TokenUsage) -> bool {
        let mut pending = crate::lock(&self.pending);
        let was_empty = pending.is_empty();
        pending.insert(run_id, usage);
        was_empty
    }

    fn take(&self) -> BTreeMap<String, TokenUsage> {
        std::mem::take(&mut *crate::lock(&self.pending))
    }
}

impl UsageSink for RunService {
    fn is_live(&self, run_id: &str) -> bool {
        crate::lock(&self.metered.live).contains(run_id)
    }

    fn live_generation(&self) -> u64 {
        self.metered.generation.load(Ordering::SeqCst)
    }

    fn post(&self, run_id: String, usage: TokenUsage) {
        if self.metered.offer(run_id, usage) {
            let _ = self.tx.send(Msg::Usage);
        }
    }
}

impl RunService {
    /// The event loop's `Msg::Usage`: one `OrchestratorUsage` step per pending run.
    pub(super) async fn drain_usage(self: &Arc<Self>) {
        for (run_id, usage) in self.metered.take() {
            self.handle(EventKind::OrchestratorUsage { run_id, usage })
                .await;
        }
    }
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
