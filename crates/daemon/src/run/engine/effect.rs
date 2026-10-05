//! [`Effect`]: what a reducer step asks the driver to do. Split out of `mod.rs` before
//! milestone 9.1 to keep it under the 600-line rule; a pure move.

use std::path::PathBuf;

use super::{OpKind, ReplyId};
use crate::run::model::OpId;

/// `Op` carries a whole `OpKind` (about 256 bytes); effects live only between a step
/// and the driver, so boxing every op would buy nothing.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Reply {
        reply: ReplyId,
        result: Result<String, String>,
    },
    Op {
        run_id: String,
        op: OpId,
        kind: OpKind,
    },
    /// One new turn (decision 29).
    Deliver {
        run_id: String,
        message_ids: Vec<u64>,
        window_id: u32,
        text: String,
    },
    Interrupt {
        window_id: u32,
    },
    KillWindow {
        window_id: u32,
    },
    RetireWindow {
        window_id: u32,
    },
    RemoveWindow {
        window_id: u32,
    },
    WatchWorktree {
        root: PathBuf,
    },
    UnwatchWorktree {
        root: PathBuf,
    },
    Persist {
        run_id: String,
        urgent: bool,
    },
    WriteReport {
        run_id: String,
    },
    Publish {
        structural: bool,
    },
    /// Milestone 9 decision 22: a sub-planner's epic was accepted; the driver retires
    /// its session (`ScoutService::accept_planner`).
    PlannerAccepted {
        window_id: u32,
    },
    /// Decision 22: a sub-planner failed in the engine; the driver stops its session.
    StopPlanner {
        window_id: u32,
        reason: String,
    },
    /// Decision 20: a run scout is halted by the engine (`run cancel`, the `finish`
    /// edit); the driver stops its session on the scout service with `reason`.
    StopScout {
        scout_id: String,
        reason: String,
    },
    /// Decision 39: paste `text` into the idle orchestrator's window (the driver's
    /// `wake.rs`, task M9.13), then answer `OrchEvent::OrchestratorWoken`.
    WakeOrchestrator {
        run_id: String,
        window_id: u32,
        text: String,
        digest_revision: u64,
        /// The highest note seq `text` holds (`OrchEvent::OrchestratorWoken` drops
        /// the notes up to it).
        notes_seq: u64,
        /// Milestone 9.3 decision 11 (D13, fix round 1): `Some(n)` when `text` starts
        /// with the run's `request_wake`, round `n`'s, pasted whole and kept until
        /// delivered. Daemon-internal: no wire field.
        request: Option<u32>,
        /// Milestone 9.5 decision 38: `text` is the session's first prompt, pasted whole
        /// into a window that has sent a signal, held until its `OrchestratorWoken`.
        first_turn: bool,
        /// Ruling T20-1: each pending note `text` holds, with its seq (the note's id),
        /// oldest first; empty for a first turn. The driver pastes a note at most once.
        notes: Vec<(u64, String)>,
    },
    /// Milestone 9.3 decision 23: a continued run takes its idle chain's orchestrator
    /// window, renamed `name` and rebound to `run_id`, its session never restarted
    /// (`driver/chain_ops.rs`, off every lock).
    AdoptOrchestrator {
        run_id: String,
        window_id: u32,
        name: String,
    },
    /// Milestone 9.6 decision 12: write a design document, `text`, to `path`, a new
    /// file (a temp file, fsync, then a link that never replaces one: versions are
    /// immutable), then rewrite `index`'s `versions.json` (`driver/design_io.rs`).
    WriteDoc {
        path: PathBuf,
        text: String,
        index: Option<(PathBuf, String)>,
    },
    /// Milestone 9.6 task M9.6.10: read stored versions back, each checked against its
    /// index entry, and send their texts as `EventKind::DesignChecked`
    /// (`driver/design_restore.rs`): the approved spec's, whose requirements the engine
    /// stores from it.
    ReadBack {
        run_id: String,
        docs: Vec<(crate::run::design::state::DocVersion, PathBuf)>,
    },
}
