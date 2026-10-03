//! The run's attention lines about its orchestrator's window that the driver reports
//! (whole-branch fix round 2's held wake-up; milestone 9.5 decisions 38 and 39's start
//! prompt), moved out of `orch/mod.rs` in task M9.5.5b. Pure.

use super::RunOrch;

/// The attention line of a held wake-up ([`RunOrch::wake_held`]).
pub const WAKE_HELD: &str =
    "orchestrator wake-up held: its window was at a prompt; type in it to continue";

/// Milestone 9.5 decision 39's attention line, while [`RunOrch::start_prompt`] (the
/// driver's report) or [`RunOrch::first_turn_late`] (decision 38's bound) is set.
pub const START_PROMPT: &str =
    "orchestrator waits at a start prompt; focus its window (C-b T, Enter) to answer it";

/// The lines [`RunOrch`]'s in-memory flags add to a run's attention; none for a
/// terminal run. [`START_PROMPT`] once, whichever of its two causes holds.
pub fn orchestrator_lines(orch: &RunOrch, terminal: bool) -> Vec<String> {
    let mut lines = Vec::new();
    if terminal {
        return lines;
    }
    if orch.wake_held {
        lines.push(WAKE_HELD.to_string());
    }
    if orch.start_prompt || orch.first_turn_late {
        lines.push(START_PROMPT.to_string());
    }
    lines
}
