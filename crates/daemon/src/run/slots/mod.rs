//! Milestone 9.1 decisions 23–27: the test scheduler. One per daemon, shared by every
//! run through `RunService`; every engine command that runs repository code waits here
//! for its slots, one step at a time. I/O side: the async wait and the lock. The
//! accounting itself is the pure [`SlotBook`].
//!
//! The book is under a `std::sync::Mutex` taken with `crate::lock` (AGENTS.md rule 3)
//! and held only to update it: never across an `.await` or a command (rule 2). Each
//! waiter has its own `Notify`, whose stored permit means a grant made between the
//! check and the wait is not lost.

mod book;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

use super::engine::EventKind;

pub use book::{
    LOAD_VAR, Priority, SLOT_VARS, SlotBook, SlotRequest, Ticket, Want, slot_vars, worker_caps,
};

/// The daemon's scheduler: `[testing] test_slots`, else [`default_slots`].
pub struct TestScheduler {
    shared: Arc<Shared>,
}

struct Shared {
    inner: Mutex<Inner>,
}

struct Inner {
    book: SlotBook,
    /// Each waiting ticket's wake-up.
    wakers: HashMap<Ticket, Arc<Notify>>,
    /// Granted tickets their waiter has not picked up yet: slots and the load then.
    granted: HashMap<Ticket, (u32, String)>,
}

impl Shared {
    /// Grants what the book allows and wakes each granted waiter. Under the lock.
    fn dispatch(inner: &mut Inner) {
        for (ticket, slots) in inner.book.grant() {
            let load = inner.book.load();
            inner.granted.insert(ticket, (slots, load));
            if let Some(notify) = inner.wakers.get(&ticket) {
                notify.notify_one();
            }
        }
    }

    /// Gives `ticket` back, whether held, granted but not picked up, or waiting.
    fn release(&self, ticket: Ticket) {
        let mut inner = crate::lock(&self.inner);
        inner.wakers.remove(&ticket);
        inner.granted.remove(&ticket);
        inner.book.release(ticket);
        Self::dispatch(&mut inner);
    }
}

impl TestScheduler {
    pub fn new(slots: u32) -> Arc<Self> {
        Arc::new(TestScheduler {
            shared: Arc::new(Shared {
                inner: Mutex::new(Inner {
                    book: SlotBook::new(slots),
                    wakers: HashMap::new(),
                    granted: HashMap::new(),
                }),
            }),
        })
    }

    pub fn slots(&self) -> u32 {
        crate::lock(&self.shared.inner).book.slots()
    }

    /// How many requests wait for a grant (tests of the callers' waits).
    #[cfg(test)]
    pub(crate) fn waiting(&self) -> usize {
        crate::lock(&self.shared.inner).book.waiting()
    }

    /// Waits, without blocking the runtime, until `req` is granted. The grant holds its
    /// slots until it is dropped; dropping this future first gives up its place.
    pub async fn acquire(&self, req: SlotRequest) -> SlotGrant {
        let notify = Arc::new(Notify::new());
        let ticket = {
            let mut inner = crate::lock(&self.shared.inner);
            let ticket = inner.book.ask(req);
            inner.wakers.insert(ticket, notify.clone());
            Shared::dispatch(&mut inner);
            ticket
        };
        let mut waiting = Waiting {
            shared: Some(self.shared.clone()),
            ticket,
        };
        loop {
            let picked = {
                let mut inner = crate::lock(&self.shared.inner);
                let picked = inner.granted.remove(&ticket);
                if picked.is_some() {
                    inner.wakers.remove(&ticket);
                }
                picked
            };
            if let Some((slots, load)) = picked {
                return SlotGrant {
                    shared: waiting.shared.take(),
                    ticket,
                    slots,
                    load,
                };
            }
            notify.notified().await;
        }
    }
}

/// An `acquire` still waiting: gives its place up when the future is dropped.
struct Waiting {
    shared: Option<Arc<Shared>>,
    ticket: Ticket,
}

impl Drop for Waiting {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.take() {
            shared.release(self.ticket);
        }
    }
}

/// Slots held for one step; released on drop.
pub struct SlotGrant {
    shared: Option<Arc<Shared>>,
    ticket: Ticket,
    slots: u32,
    load: String,
}

impl SlotGrant {
    pub fn slots(&self) -> u32 {
        self.slots
    }

    /// Decision 26: [`SLOT_VARS`] at the granted slots, and [`LOAD_VAR`], the share of
    /// the daemon's slots held right after this grant.
    pub fn env(&self) -> Vec<(String, String)> {
        let mut env = slot_vars(self.slots);
        env.push((LOAD_VAR.to_string(), self.load.clone()));
        env
    }
}

impl Drop for SlotGrant {
    fn drop(&mut self) {
        if let Some(shared) = self.shared.take() {
            shared.release(self.ticket);
        }
    }
}

/// Decision 27: a run started through the driver's event loop carries the daemon's
/// `slots`, from which its workers' caps are computed. Restored runs never pass through
/// the loop; `RunService::restore` stamps each one as it is loaded (ruling C-11).
pub(crate) fn stamp(mut kind: EventKind, slots: u32) -> EventKind {
    if let EventKind::Start { run, .. } = &mut kind {
        run.test_slots = slots;
    }
    kind
}

/// Logical cores minus two, at least one (decision 23).
pub fn default_slots() -> u32 {
    slots_for(std::thread::available_parallelism().ok().map(|n| n.get()))
}

/// [`default_slots`] for `cores` (`None` when they cannot be read).
fn slots_for(cores: Option<usize>) -> u32 {
    let spare = cores.unwrap_or(1).saturating_sub(2);
    u32::try_from(spare).unwrap_or(u32::MAX).max(1)
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
