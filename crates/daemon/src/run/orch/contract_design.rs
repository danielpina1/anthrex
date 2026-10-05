//! Milestone 9.6 (DF §5.2, §9): the design flow's contract texts. Task M9.6.13 adds the
//! requirements a design run's worker and reviewer prompts carry (decisions 25 and 26);
//! task M9.6.14 adds rules 47–52. Pure.

use crate::run::model::{Run, Task};

/// Decision 25: the requirements block's cap, in bytes.
pub const REQUIREMENTS_MAX: usize = 6 * 1024;

/// Decision 26: the reviewer's line after the block.
pub const JUDGE_LINE: &str = "Judge the change against each requirement above; cite its id (R2) in any finding that concerns it.";

/// Room kept for the cut marker, `\n[cut: <n> bytes]`.
const MARKER_ROOM: usize = 32;

/// Decision 25: the approved spec's requirements task `task` covers, in the spec's
/// order, and its Goal section, at most [`REQUIREMENTS_MAX`] bytes (a longer block is
/// cut and ends `[cut: <n> bytes]`, as the brainstorm pack does). `None` for a run
/// without the design flow, or a task that covers none of the approved requirements.
/// The requirements are the approved spec's, stored at its approval from the driver's
/// read-back (`DesignState.requirements`), never parsed here.
pub fn requirements_block(run: &Run, task: &Task) -> Option<String> {
    let design = run.orch.design.as_ref()?;
    let covers = &task.spec.covers;
    let lines: Vec<String> = (design.requirements.iter())
        .filter(|r| covers.contains(&r.id))
        .map(|r| format!("{}  {}", r.id, r.text))
        .collect();
    if lines.is_empty() {
        return None;
    }
    let full = format!(
        "Spec requirements this task delivers:\n{}\nGoal (from the spec): {}",
        lines.join("\n"),
        design.goal_section
    );
    if full.len() <= REQUIREMENTS_MAX {
        return Some(full);
    }
    let mut kept = REQUIREMENTS_MAX - MARKER_ROOM;
    while !full.is_char_boundary(kept) {
        kept -= 1;
    }
    Some(format!(
        "{}\n[cut: {} bytes]",
        &full[..kept],
        full.len() - kept
    ))
}

#[cfg(test)]
#[path = "contract_design_tests.rs"]
mod tests;
