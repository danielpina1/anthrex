//! Whole-branch fix round 2, item 2: what the driver reports to the engine about a
//! waiting wake-up, split out of `driver/wake.rs` (milestone 9.5, task M9.5.5b).

use std::collections::HashSet;
use std::time::Duration;

use super::super::RunService;
use crate::run::engine::{EventKind, OrchEvent};

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
        // Fix round 3, item 1: the changes are sent under the `held` lock (`send` is a
        // non-blocking unbounded send), so two checks at once (the tick and the window
        // watch) send in the order they updated the set, and the engine's last word is
        // always the set's.
        let mut held = crate::lock(&self.wakes.held);
        let gone: Vec<String> = held.difference(&now_held).cloned().collect();
        let new: Vec<String> = now_held.difference(&held).cloned().collect();
        let changes = gone
            .into_iter()
            .map(|r| (r, false))
            .chain(new.into_iter().map(|r| (r, true)));
        for (run_id, is_held) in changes {
            self.send(EventKind::Orch(OrchEvent::WakeHeld {
                run_id,
                held: is_held,
            }));
        }
        *held = now_held;
    }
}
