//! Milestone 9.1 decisions 23–25 and 27: the test scheduler's slot accounting. Pure —
//! no `std::process`, `std::fs`, locks, clocks or async (design decision 1); the async
//! wrapper that waits for a grant is `super::TestScheduler`.
//!
//! Waiting requests are ordered by (class, critical first, ask order). Only the first
//! may be granted: it gets `min(asked, free)` as soon as one slot is free, or, when it
//! is exclusive, every slot once none is held. The next is considered only after that,
//! so a request never waits behind a lower-priority one (decision 24).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The four variables that carry a slot count to a command (decisions 26 and 27).
pub const SLOT_VARS: [&str; 4] = [
    "CARGO_BUILD_JOBS",
    "NEXTEST_TEST_THREADS",
    "RUST_TEST_THREADS",
    "ANTHREX_TEST_SLOTS",
];

/// The share of the daemon's slots held right after a grant (decision 26).
pub const LOAD_VAR: &str = "ANTHREX_TEST_LOAD";

/// A request's class, highest first (decision 24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Priority {
    Candidate,
    Gate,
    FullStage,
    FullIdle,
    Verify,
}

/// How many slots a step asks for (decision 24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Want {
    One,
    Half,
    All,
}

impl Want {
    /// The slots this asks for out of `slots`: one; half, rounded up and at least one;
    /// or all of them.
    pub fn of(self, slots: u32) -> u32 {
        match self {
            Want::One => 1,
            Want::Half => slots.div_ceil(2).max(1),
            Want::All => slots.max(1),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotRequest {
    pub priority: Priority,
    /// On the run's critical path: first among equal classes.
    pub critical: bool,
    pub want: Want,
    /// Decision 25: granted only when no slot is held, and then gets every slot.
    pub exclusive: bool,
    /// What asked, for logs.
    pub label: String,
}

/// One request's place in the book, from ask to release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Ticket(u64);

#[derive(Debug)]
pub struct SlotBook {
    slots: u32,
    /// Granted tickets and their slots.
    held: BTreeMap<Ticket, u32>,
    /// Waiting requests by (class, not critical, ask order).
    waiting: BTreeMap<(Priority, bool, u64), SlotRequest>,
    next: u64,
}

impl SlotBook {
    /// A book of `slots` slots (at least one).
    pub fn new(slots: u32) -> Self {
        SlotBook {
            slots: slots.max(1),
            held: BTreeMap::new(),
            waiting: BTreeMap::new(),
            next: 0,
        }
    }

    pub fn slots(&self) -> u32 {
        self.slots
    }

    /// The slots granted and not yet released.
    pub fn held(&self) -> u32 {
        self.held.values().sum()
    }

    /// How many requests wait for a grant.
    pub fn waiting(&self) -> usize {
        self.waiting.len()
    }

    /// Queues `req`; [`SlotBook::grant`] says when it may start.
    pub fn ask(&mut self, req: SlotRequest) -> Ticket {
        let seq = self.next;
        self.next += 1;
        self.waiting.insert((req.priority, !req.critical, seq), req);
        Ticket(seq)
    }

    /// Every waiting ticket that may start now, in order, with its slots; each is held
    /// from here until [`SlotBook::release`].
    pub fn grant(&mut self) -> Vec<(Ticket, u32)> {
        let mut granted = Vec::new();
        loop {
            let free = self.slots - self.held();
            let Some(entry) = self.waiting.first_entry() else {
                break;
            };
            let req = entry.get();
            let slots = match req.exclusive {
                true if free == self.slots => self.slots,
                false if free >= 1 => req.want.of(self.slots).min(free),
                _ => break,
            };
            let ticket = Ticket(entry.key().2);
            entry.remove();
            self.held.insert(ticket, slots);
            granted.push((ticket, slots));
        }
        granted
    }

    /// Gives back `ticket`'s slots, or its place when it was still waiting (an
    /// abandoned wait). Unknown tickets are ignored.
    pub fn release(&mut self, ticket: Ticket) {
        if self.held.remove(&ticket).is_none() {
            self.waiting.retain(|key, _| key.2 != ticket.0);
        }
    }

    /// Held slots over all slots, one decimal place, rounded half up (`0.5`).
    pub fn load(&self) -> String {
        let tenths =
            (u64::from(self.held()) * 10 + u64::from(self.slots) / 2) / u64::from(self.slots);
        format!("{}.{}", tenths / 10, tenths % 10)
    }
}

/// The four [`SLOT_VARS`], each `n`.
pub fn slot_vars(n: u32) -> Vec<(String, String)> {
    SLOT_VARS
        .iter()
        .map(|name| (name.to_string(), n.to_string()))
        .collect()
}

/// Decision 27: a worker's caps, each of [`SLOT_VARS`] at `max(1, test_slots /
/// max_writers)`. Workers run their own tests unscheduled, inside their sandbox.
pub fn worker_caps(test_slots: u32, max_writers: u8) -> Vec<(String, String)> {
    slot_vars((test_slots / u32::from(max_writers.max(1))).max(1))
}

#[cfg(test)]
#[path = "book_tests.rs"]
mod tests;
