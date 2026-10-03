//! Milestone 9.5's daemon model (review ruling I3): a racing task's lanes (decisions
//! 19–20), a paired task's test writer (decision 25) and a run's per-runtime writer
//! caps (decision 16). Added once, here, for every reducer task that uses them; nothing
//! reads them yet. Pure, like `model.rs`, which re-exports them.
//!
//! The fields they hang from (`Task.{race, pair, race_wait_since}`, `Run.concurrency`
//! and the `lane` of `AgentRound`, `CheckRecord`, `ProofRecord`, `ReviewRecord` and
//! `PendingOp`) are `#[serde(default)]` and skipped while unset, so a run with no race,
//! pair or cap writes the same `run.json` as milestone 9.3.

use proto::{GateCounts, LaneState, PairPhase, RaceLane, Route, Spend};
use serde::{Deserialize, Serialize};

use super::DoneClaim;

/// One runtime's writer cap inside a run (decision 16), keyed in `Run.concurrency` by
/// the runtime label `Run.rate_limits` uses. `cap` starts at `max_writers`; a rate
/// limit halves it, a quiet `recover_after_secs` raises it one step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeConcurrency {
    pub cap: u8,
    pub last_rate_limit_at: Option<u64>,
    pub last_change_at: Option<u64>,
    pub halvings: u32,
    pub recoveries: u32,
}

/// One lane of a racing task (decision 19): its own standalone checkout `<task>.<lane>`,
/// its own racer session and its own gate counters (decision 20).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lane {
    pub lane: RaceLane,
    pub route: Route,
    pub review_route: Option<Route>,
    /// The lane's checkout name, `<task>.<lane>`.
    pub checkout: String,
    pub state: LaneState,
    /// The lane's racer session number (the task's session numbering).
    pub session: u32,
    pub start_commit: Option<String>,
    pub head: Option<String>,
    pub done: Option<DoneClaim>,
    pub failures: u8,
    pub bounces: GateCounts,
    pub stalls: u8,
    pub budget_exceeded: u8,
    pub spent: Spend,
    /// Why the lane left the race (`Out`).
    pub reason: Option<String>,
    pub salvage_ref: Option<String>,
    /// The stale lock files removed from the lane's git directory before salvage.
    pub cleared_locks: Vec<String>,
    /// When the lane's racer was sent `KillWindow` (decision 22's wait).
    pub kill_sent_at: Option<u64>,
    /// The racer's session has ended (`ProcessExited`, or it had already ended).
    pub exited: bool,
    /// The lane's checkout was removed.
    pub removed: bool,
    /// The lane's checkout was kept: its racer did not exit in time.
    pub kept: bool,
}

/// A racing task's two lanes and, once decided, the lane that became the task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Race {
    pub lanes: Vec<Lane>,
    pub winner: Option<RaceLane>,
    /// The winner was adopted (the other lane was out), not crowned by passing.
    pub adopted: bool,
    pub started_at: u64,
}

/// A paired task's test writer and its red commit (decisions 25–26).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pair {
    pub phase: PairPhase,
    pub writer_route: Route,
    pub test: Option<String>,
    pub red: Option<String>,
    /// `Some(true)` once the red-only proof saw the test fail at `red`.
    pub red_checked: Option<bool>,
    /// The test writer's gate failures, moved here when the implementer starts.
    pub writer_failures: u8,
    pub writer_sessions: u32,
}

#[cfg(test)]
#[path = "model_tuning_tests.rs"]
mod tests;
