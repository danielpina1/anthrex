//! The OTLP receiver's connection slots (milestone 9 decision 14b): at most
//! `OTLP_BASE_CONNECTIONS` plus the live orchestrators are served at once, and when every
//! slot is taken, a connection that never presented a valid token is closed first, so a
//! local process holding slots cannot starve a real orchestrator.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

use super::OTLP_SLOT_WAIT;

/// The connections being served (decision 14b). A connection that has presented a valid
/// token is never closed to make room; one that has not is, oldest first.
#[derive(Default)]
pub(super) struct Slots {
    open: Mutex<BTreeMap<u64, Arc<ConnState>>>,
    next: AtomicU64,
    freed: Notify,
}

#[derive(Default)]
pub(super) struct ConnState {
    pub(super) tokened: AtomicBool,
    pub(super) close: CancellationToken,
}

/// One served connection's place; given back when dropped.
pub(super) struct Slot {
    slots: Arc<Slots>,
    id: u64,
    pub(super) state: Arc<ConnState>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        crate::lock(&self.slots.open).remove(&self.id);
        self.slots.freed.notify_waiters();
    }
}

impl Slots {
    /// A place under `cap`, closing one untokened connection when every place is taken,
    /// or `None` after [`OTLP_SLOT_WAIT`].
    pub(super) async fn acquire(self: &Arc<Self>, cap: usize) -> Option<Slot> {
        let deadline = tokio::time::Instant::now() + OTLP_SLOT_WAIT;
        let mut evicted = false;
        loop {
            let freed = self.freed.notified();
            tokio::pin!(freed);
            freed.as_mut().enable();
            {
                let mut open = crate::lock(&self.open);
                if open.len() < cap {
                    let id = self.next.fetch_add(1, Ordering::Relaxed);
                    let state = Arc::new(ConnState::default());
                    open.insert(id, state.clone());
                    let slots = self.clone();
                    return Some(Slot { slots, id, state });
                }
                if !evicted {
                    let victim = open
                        .values()
                        .find(|c| !c.tokened.load(Ordering::SeqCst) && !c.close.is_cancelled());
                    if let Some(victim) = victim {
                        victim.close.cancel();
                        evicted = true;
                    }
                }
            }
            tokio::select! {
                _ = freed => {}
                _ = tokio::time::sleep_until(deadline) => return None,
            }
        }
    }
}
