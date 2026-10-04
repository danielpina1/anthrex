//! What the driver reports to the engine, and logs, about the orchestrator's window
//! while a wake-up waits (split out of `driver/wake.rs` in milestone 9.5, task
//! M9.5.5b): a wake-up held at a prompt (whole-branch fix round 2, item 2), a start
//! prompt (decision 39), and why a wake-up waits and when it went (decision 41).

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use proto::{Runtime, Status, WindowInfo};

use super::super::RunService;
use super::{Seen, Wakes};
use crate::run::engine::{EventKind, OrchEvent};

/// Milestone 9.5 decision 41: why a window does not take a paste now, in the words of
/// the wake log's line (`wake-up for run <id> waits: <reason>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitReason {
    /// `Working`.
    Working,
    /// `Starting`, or `Exited` while the driver confirms the exit.
    Starting,
    /// A client's input this long ago, inside the quiet time.
    Input { secs: u64 },
    /// `Attention`, or an attention no turn end has answered.
    Attention,
    /// A first turn only: the window has sent no hook or title signal (decision 38).
    NoSignal,
}

impl std::fmt::Display for WaitReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WaitReason::Working => f.write_str("window working"),
            WaitReason::Starting => f.write_str("window starting"),
            WaitReason::Input { secs } => write!(f, "input {secs}s ago"),
            WaitReason::Attention => f.write_str("attention open"),
            WaitReason::NoSignal => f.write_str("no hook signal yet"),
        }
    }
}

/// The reason each run's wake-up was last logged as waiting for (decision 41).
#[derive(Default)]
pub(in crate::run::driver) struct Waits {
    logged: Mutex<HashMap<String, std::mem::Discriminant<WaitReason>>>,
    /// Tests only: every line logged.
    #[cfg(test)]
    pub(super) lines: Mutex<Vec<String>>,
}

/// Decision 39's report: since when each run's live orchestrator window has been at a
/// start prompt (and which window), and the runs the engine was last told wait at one.
#[derive(Default)]
pub(in crate::run::driver) struct StartPrompts {
    since: Mutex<HashMap<String, (u32, Instant)>>,
    reported: Mutex<HashSet<String>>,
}

/// The TUI's start-prompt predicate (`app/alerts.rs::orchestrator_text`): an agent
/// window that has sent no signal, quiet (`Idle`, `Done`) or at a prompt (`Attention`).
fn at_start_prompt(window: &WindowInfo) -> bool {
    matches!(window.runtime, Runtime::Claude | Runtime::Codex)
        && !window.signals_seen
        && matches!(
            window.status,
            Status::Idle | Status::Done | Status::Attention
        )
}

impl Wakes {
    /// Logs at `info` why `run_id`'s wake-up waits, once until the kind of reason
    /// changes (an input's age counting up is one reason); `None`: it no longer waits.
    pub(super) fn log_wait(&self, run_id: &str, reason: Option<WaitReason>) {
        let mut logged = crate::lock(&self.waits.logged);
        let Some(reason) = reason else {
            logged.remove(run_id);
            return;
        };
        let kind = std::mem::discriminant(&reason);
        if logged.insert(run_id.to_string(), kind) == Some(kind) {
            return;
        }
        self.log(format!("wake-up for run {run_id} waits: {reason}"));
    }

    /// Forgets the logged reason of each run with no wake-up waiting any more, so the
    /// next one's wait is logged afresh.
    pub(super) fn forget_waits(&self) {
        let pending: HashSet<String> = crate::lock(&self.pending).keys().cloned().collect();
        crate::lock(&self.waits.logged).retain(|run_id, _| pending.contains(run_id));
    }

    /// Decision 41's delivery line.
    pub(super) fn log_delivered(&self, window_id: u32, bytes: usize) {
        self.log(format!(
            "wake-up delivered to window {window_id} ({bytes} bytes)"
        ));
    }

    fn log(&self, line: String) {
        tracing::info!("{line}");
        #[cfg(test)]
        crate::lock(&self.waits.lines).push(line);
    }
}

impl RunService {
    /// Whole-branch fix round 2, item 2 (controller ruling: safety over liveness): a
    /// wake-up that waits only because its window was at a prompt (`attention_open`)
    /// is never pasted, but once the window has been `Idle` for its quiet time the run
    /// shows [`crate::run::orch::WAKE_HELD`]; a change either way is sent to the engine.
    pub(super) fn report_held(&self) {
        // The waiting wake-ups copied out first: no lock of ours is held while the
        // manager's is taken.
        let waiting: Vec<(String, u32, Duration)> = crate::lock(&self.wakes.pending)
            .iter()
            .map(|(run_id, p)| (run_id.clone(), p.window_id, p.quiet))
            .collect();
        let now_held: HashSet<String> = waiting
            .into_iter()
            .filter(|(_, window, quiet)| {
                self.manager
                    .held_at_prompt_for(*window)
                    .is_some_and(|idle| idle >= *quiet)
            })
            .map(|(run_id, _, _)| run_id)
            .collect();
        self.send_changes(&self.wakes.held, now_held, |run_id, held| {
            OrchEvent::WakeHeld { run_id, held }
        });
    }

    /// Decision 39: reports, on each change, whether each run's live orchestrator window
    /// has been at a start prompt (`at_start_prompt`) for its run's quiet time.
    pub(super) fn report_start_prompt(&self, seen: &[Seen], windows: &[WindowInfo]) {
        let waiting: HashSet<String> = {
            let mut since = crate::lock(&self.wakes.start_prompts.since);
            let mut next = HashMap::new();
            let mut waiting = HashSet::new();
            for s in seen.iter().filter(|s| s.live && !s.terminal) {
                let window = windows.iter().find(|w| w.id == s.window_id);
                if !window.is_some_and(at_start_prompt) {
                    continue;
                }
                let at = match since.get(&s.run_id) {
                    Some(&(id, at)) if id == s.window_id => at,
                    _ => Instant::now(),
                };
                if at.elapsed() >= s.quiet {
                    waiting.insert(s.run_id.clone());
                }
                next.insert(s.run_id.clone(), (s.window_id, at));
            }
            *since = next;
            waiting
        };
        self.send_changes(
            &self.wakes.start_prompts.reported,
            waiting,
            |run_id, waiting| OrchEvent::StartPrompt { run_id, waiting },
        );
    }

    /// Sends `event(run, true)` for each run new in `now` and `event(run, false)` for each
    /// gone from it, then keeps `now` in `set`. Fix round 3, item 1: the changes are sent
    /// under `set`'s lock (`send` is a non-blocking unbounded send), so two checks at
    /// once (the tick and the window watch) send in the order they updated the set, and
    /// the engine's last word is always the set's.
    fn send_changes(
        &self,
        set: &Mutex<HashSet<String>>,
        now: HashSet<String>,
        event: impl Fn(String, bool) -> OrchEvent,
    ) {
        let mut last = crate::lock(set);
        let gone: Vec<String> = last.difference(&now).cloned().collect();
        let new: Vec<String> = now.difference(&last).cloned().collect();
        let changes = gone
            .into_iter()
            .map(|r| (r, false))
            .chain(new.into_iter().map(|r| (r, true)));
        for (run_id, on) in changes {
            self.send(EventKind::Orch(event(run_id, on)));
        }
        *last = now;
    }
}

#[cfg(test)]
#[path = "wake_report_tests.rs"]
mod tests;
