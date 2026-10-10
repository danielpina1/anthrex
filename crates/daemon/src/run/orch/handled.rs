//! Milestone 9.9 decision 12 (OFA §4.5): what the orchestrator resolved on its own,
//! kept in `RunOrch.handled` and shown quietly. Pure.

use serde::{Deserialize, Serialize};

use crate::run::model::{LogEntry, Run};

/// At most this many records are kept; `RunOrch.handled_total` counts all.
pub const HANDLED_KEPT: usize = 50;

/// The run log keeps this many entries (`engine/requests.rs::LOG_MAX`).
const LOG_MAX: usize = 500;

/// One thing the orchestrator resolved: `op` is the wire name, `target` what it acted on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HandledRecord {
    pub at: u64,
    pub op: String,
    pub target: String,
    pub reason: String,
}

/// Records a resolved op at `now`: a [`HandledRecord`] (oldest dropped past
/// [`HANDLED_KEPT`]), the count, and the log line `orchestrator: <verb> <target> — <reason>`.
pub fn record(run: &mut Run, now: u64, (op, verb, target): (&str, &str, &str), reason: &str) {
    let line = format!("{verb} {target}");
    record_line(run, now, (op, target, &line), reason);
}

/// [`record`] with the log line's words (`orchestrator: <line> — <reason>`) apart from
/// the handled record's `target`.
pub fn record_line(run: &mut Run, now: u64, (op, target, line): (&str, &str, &str), reason: &str) {
    run.orch.handled.push(HandledRecord {
        at: now,
        op: op.into(),
        target: target.into(),
        reason: reason.into(),
    });
    let excess = run.orch.handled.len().saturating_sub(HANDLED_KEPT);
    run.orch.handled.drain(..excess);
    run.orch.handled_total = run.orch.handled_total.saturating_add(1);
    run.log.push(LogEntry {
        at: now,
        text: format!("orchestrator: {line} — {reason}"),
    });
    let excess = run.log.len().saturating_sub(LOG_MAX);
    run.log.drain(..excess);
}
