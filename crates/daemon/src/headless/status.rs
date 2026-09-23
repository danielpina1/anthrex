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
}

impl Default for HeadlessStatus {
    fn default() -> Self {
        HeadlessStatus {
            status: Status::Starting,
            tool: None,
            turn_open: false,
            rate_limited: false,
        }
    }
}

/// The status after `event`:
///
/// - `Starting` until the first `Init` or `TurnStarted`; lines before it (Claude's hook
///   progress) keep it.
/// - `Working` while a turn is open. Claude repeats `Init` at every turn's start (M8a.1),
///   so a later `Init` opens a turn, including one Claude Code starts by itself.
/// - `Attention` while a rate-limit retry is pending (cleared by any later event), or
///   after a failed turn (cleared by the next turn).
/// - `Idle` between turns, after a completed or interrupted one.
/// - `Exited` once the process has ended.
///
/// `tool` is the latest top-level `ToolUse`'s name (a sub-agent's is not the window's),
/// cleared when the turn ends.
pub fn next(current: &HeadlessStatus, event: &SessionEvent) -> HeadlessStatus {
    let mut state = current.clone();
    let was_rate_limited = state.rate_limited;
    state.rate_limited = false;
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
        other => {
            if let SessionEvent::ToolUse {
                name, parent: None, ..
            } = other
            {
                state.tool = Some(name.clone());
            }
            if was_rate_limited {
                state.status = if state.turn_open {
                    Status::Working
                } else {
                    Status::Idle
                };
            }
        }
    }
    state
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
