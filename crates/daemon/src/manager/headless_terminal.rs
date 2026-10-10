//! Decision 9 (M9.11.9, A12): what the manager applied of each headless process's
//! terminal events, so the engine's forwarder can catch up after its feed lagged
//! (`run/driver/event_loop.rs`, `forward`). Kept under the manager lock, beside the feed
//! send it mirrors (`WindowManager::apply_session_event`).

use std::collections::VecDeque;

use tokio::sync::broadcast;

use super::entry::Process;
use super::{Inner, WindowManager, WindowSignal};
use crate::headless::SessionEvent;

/// How many of a headless window's processes, newest last, keep a [`TerminalRecord`]. A
/// lag that hides more of one window's processes than this loses the oldest ones'
/// records (a Codex window runs one process per turn).
pub const TERMINAL_PIDS_KEPT: usize = 32;

/// What the manager applied of one process's terminal events. `turn_ends` counts the
/// prompted `TurnEnded`s (an unprompted one is not the delivered turn's end), and
/// `last_turn_end` is the newest of them; `exited` is the process's `ProcessExited`.
#[derive(Debug, Clone, PartialEq)]
pub struct TerminalRecord {
    pub window_id: u32,
    pub pid: u32,
    pub turn_ends: u64,
    pub last_turn_end: Option<SessionEvent>,
    pub exited: Option<SessionEvent>,
}

/// Records a prompted `TurnEnded` or a `ProcessExited` of `pid`; any other event is not
/// terminal and is ignored.
pub(super) fn record(
    terminal: &mut VecDeque<TerminalRecord>,
    window_id: u32,
    pid: u32,
    event: &SessionEvent,
) {
    let turn_end = matches!(event, SessionEvent::TurnEnded { .. });
    if !turn_end && !matches!(event, SessionEvent::ProcessExited { .. }) {
        return;
    }
    if !terminal.iter().any(|r| r.pid == pid) {
        if terminal.len() >= TERMINAL_PIDS_KEPT {
            terminal.pop_front();
        }
        terminal.push_back(TerminalRecord {
            window_id,
            pid,
            turn_ends: 0,
            last_turn_end: None,
            exited: None,
        });
    }
    let Some(record) = terminal.iter_mut().find(|r| r.pid == pid) else {
        return;
    };
    if turn_end {
        record.turn_ends += 1;
        record.last_turn_end = Some(event.clone());
    } else {
        record.exited = Some(event.clone());
    }
}

impl WindowManager {
    /// Every headless window's [`TerminalRecord`]s.
    pub fn headless_terminal_events(&self) -> Vec<TerminalRecord> {
        records(&crate::lock(&self.inner))
    }

    /// After `old` lagged: a fresh feed receiver, every signal `old` still retained, in
    /// order, and every [`TerminalRecord`], all under one hold of the lock every feed
    /// send is made under. An event applied before the cut is in the records (and in the
    /// retained signals, if it was not lost), one applied after is on the fresh receiver
    /// only. Draining `old` is memory only: no send can happen while the lock is held.
    pub fn signals_and_terminal_events(
        &self,
        old: &mut broadcast::Receiver<WindowSignal>,
    ) -> (
        broadcast::Receiver<WindowSignal>,
        Vec<WindowSignal>,
        Vec<TerminalRecord>,
    ) {
        let inner = crate::lock(&self.inner);
        let mut retained = Vec::new();
        loop {
            match old.try_recv() {
                Ok(signal) => retained.push(signal),
                // Lost again before the lock was taken: the records cover it.
                Err(broadcast::error::TryRecvError::Lagged(_)) => {}
                Err(_) => break,
            }
        }
        (self.signals.subscribe(), retained, records(&inner))
    }
}

fn records(inner: &Inner) -> Vec<TerminalRecord> {
    (inner.entries.values())
        .filter_map(|entry| match &entry.process {
            Process::Headless(window) => Some(window.terminal.iter().cloned()),
            _ => None,
        })
        .flatten()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::headless::TurnOutcome;

    fn turn_end(text: &str) -> SessionEvent {
        SessionEvent::TurnEnded {
            outcome: TurnOutcome::Completed,
            usage: None,
            denials: vec![text.to_string()],
        }
    }

    /// Each process keeps its own count, its newest turn end and its exit; the oldest
    /// process's record goes once more than `TERMINAL_PIDS_KEPT` processes are recorded.
    #[test]
    fn records_count_per_process_and_keep_the_newest_processes() {
        let mut terminal = VecDeque::new();
        record(&mut terminal, 3, 70, &turn_end("first"));
        record(&mut terminal, 3, 71, &turn_end("other"));
        record(&mut terminal, 3, 70, &turn_end("second"));
        let exit = SessionEvent::ProcessExited {
            code: Some(0),
            signal: None,
        };
        record(&mut terminal, 3, 70, &exit);
        record(&mut terminal, 3, 70, &SessionEvent::TurnStarted);
        assert_eq!(terminal.len(), 2);
        assert_eq!(terminal[0].turn_ends, 2);
        assert_eq!(terminal[0].last_turn_end, Some(turn_end("second")));
        assert_eq!(terminal[0].exited, Some(exit));
        assert_eq!((terminal[1].pid, terminal[1].turn_ends), (71, 1));
        assert_eq!(terminal[1].exited, None);
        for pid in 100..100 + TERMINAL_PIDS_KEPT as u32 {
            record(&mut terminal, 3, pid, &turn_end("x"));
        }
        assert_eq!(terminal.len(), TERMINAL_PIDS_KEPT);
        assert!(
            terminal.iter().all(|r| r.pid >= 100),
            "the oldest went first"
        );
    }
}
