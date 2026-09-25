//! A headless window's status from its session events alone (decision 27), kept apart
//! from the PTY status machine in `crate::status`. Pure.

use super::{SessionEvent, TurnOutcome};
use proto::Status;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeadlessStatus {
    pub status: Status,
    pub tool: Option<String>,
    pub turn_open: bool,
    pub rate_limited: bool,
    /// The status a pending retry interrupted, given back when the retry ends.
    pub before_retry: Status,
}

impl Default for HeadlessStatus {
    fn default() -> Self {
        HeadlessStatus {
            status: Status::Starting,
            tool: None,
            turn_open: false,
            rate_limited: false,
            before_retry: Status::Starting,
        }
    }
}

/// The status after `event`:
///
/// - `Starting` until the first `Init` or `TurnStarted`; lines before it (Claude's hook
///   progress) keep it.
/// - `Working` while a turn is open. Claude repeats `Init` at every turn's start (M8a.1),
///   so a later `Init` opens a turn, including one Claude Code starts by itself.
/// - `Attention` while a rate-limit retry is pending, or after a failed turn (cleared by
///   the next turn). A retry ends at the next event that is not bookkeeping (`Other`,
///   `Unknown`, `StderrLine`, `Diagnostic`, which Claude emits constantly and which say
///   nothing about the retry, and the driver's `ProcessStarted`), and gives back the
///   status it interrupted.
/// - `Idle` between turns, after a completed or interrupted one.
/// - `Exited` once the process has ended.
///
/// `tool` is the latest top-level `ToolUse`'s name (a sub-agent's is not the window's),
/// cleared when the turn ends.
pub fn next(current: &HeadlessStatus, event: &SessionEvent) -> HeadlessStatus {
    let mut state = current.clone();
    if matches!(
        event,
        SessionEvent::Other { .. }
            | SessionEvent::Unknown { .. }
            | SessionEvent::StderrLine { .. }
            | SessionEvent::Diagnostic { .. }
            | SessionEvent::ProcessStarted { .. }
    ) {
        return state;
    }
    if state.rate_limited {
        state.rate_limited = false;
        state.status = state.before_retry;
    }
    match event {
        SessionEvent::ProcessExited { .. } => {
            state.status = Status::Exited;
            state.turn_open = false;
            state.tool = None;
        }
        SessionEvent::Init { .. } | SessionEvent::TurnStarted => {
            state.status = Status::Working;
            state.turn_open = true;
        }
        SessionEvent::ApiRetry { .. } => {
            state.before_retry = state.status;
            state.status = Status::Attention;
            state.rate_limited = true;
        }
        SessionEvent::TurnEnded { outcome, .. } => {
            state.status = match outcome {
                TurnOutcome::Failed { .. } => Status::Attention,
                TurnOutcome::Completed | TurnOutcome::Interrupted => Status::Idle,
            };
            state.turn_open = false;
            state.tool = None;
        }
        SessionEvent::ToolUse {
            name, parent: None, ..
        } => {
            state.tool = Some(name.clone());
        }
        _ => {}
    }
    state
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
