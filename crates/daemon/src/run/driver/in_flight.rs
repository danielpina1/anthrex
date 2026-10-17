//! The ops in flight, by run and op, with their kind's name: entered in `start_op`
//! (`effects_ops.rs`) and left when the op's task ends however it ends (a guard), so a
//! test's teardown can wait for every op but those parked for good at a closed launch
//! gate ([`RunService::stop_and_settle`](super::RunService::stop_and_settle)). The
//! daemon itself never waits on it: its process exit ends its ops. A map insert and
//! remove under a short lock per op, no I/O.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::run::model::OpId;

/// The op kinds that can wait at the launch gate (`LaunchGate::wait`, in the manager's
/// window launches and the decider's call). With the gate closed they park there for
/// good, so a teardown cannot wait for them; whatever such an op does before it reaches
/// the gate is not waited for either (`CreateWindow`'s worker git directories come after
/// it).
pub(super) const LAUNCH_KINDS: [&str; 8] = [
    "CreateOrchestrator",
    "RestartOrchestrator",
    "StartScout",
    "StartPlanner",
    "StartDesignAgent",
    "CreateWindow",
    "ResumeSession",
    "Decide",
];

#[derive(Default)]
pub(super) struct InFlight {
    ops: Mutex<HashMap<(String, OpId), &'static str>>,
    left: tokio::sync::Notify,
}

/// One op's place in [`InFlight`], given up when dropped.
pub(super) struct Entered {
    of: Arc<InFlight>,
    key: (String, OpId),
}

impl Drop for Entered {
    fn drop(&mut self) {
        crate::lock(&self.of.ops).remove(&self.key);
        self.of.left.notify_waiters();
    }
}

impl InFlight {
    pub(super) fn enter(self: &Arc<Self>, run_id: &str, op: OpId, name: &'static str) -> Entered {
        let key = (run_id.to_string(), op);
        crate::lock(&self.ops).insert(key.clone(), name);
        Entered {
            of: self.clone(),
            key,
        }
    }

    /// The names of the ops in flight that `parked` does not leave out, sorted.
    pub(super) fn waited_for(&self, parked: impl Fn(&str) -> bool) -> Vec<&'static str> {
        let mut names: Vec<_> = crate::lock(&self.ops)
            .values()
            .copied()
            .filter(|name| !parked(name))
            .collect();
        names.sort_unstable();
        names
    }

    /// Returns once no op in flight is one `parked` does not leave out. Unbounded; the
    /// caller bounds it.
    pub(super) async fn settle(&self, parked: impl Fn(&str) -> bool) {
        loop {
            let left = self.left.notified();
            tokio::pin!(left);
            left.as_mut().enable();
            if self.waited_for(&parked).is_empty() {
                return;
            }
            left.await;
        }
    }
}
