//! The event loop's bookkeeping, moved out of `driver.rs` unchanged so that file stays
//! under 600 lines (milestone 8b's file-size table).

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

/// A window the engine retired (decision 52): its group is killed at `kill_at` if it
/// still runs, and the window removed at `remove_at`.
pub(super) struct Retiring {
    pub(super) kill_at: Instant,
    pub(super) remove_at: Instant,
}

/// The event loop's bookkeeping.
pub(super) struct Book {
    pub(super) retiring: HashMap<u32, Retiring>,
    /// The process of each window the engine killed: its exit is `killed_by_engine`.
    pub(super) killed: HashMap<u32, u32>,
    /// The roots registered with `GitRoots`, so a watch or unwatch is never doubled.
    pub(super) watched: HashSet<PathBuf>,
    /// Runs with a counter-only change not yet persisted (decision 43).
    pub(super) dirty: BTreeSet<String>,
    pub(super) last_counter_save: Instant,
    pub(super) reports_written: HashMap<String, Instant>,
    pub(super) reports_due: BTreeSet<String>,
    pub(super) publish_due: bool,
    /// The ready proposals' generation the last push carried (M9.0.5 decision 10).
    pub(super) proposals_published: u64,
    /// Decision 48: the intents of each kind appended so far.
    pub(super) intents: HashMap<&'static str, u32>,
}
