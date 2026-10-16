//! Milestone 9.17 decisions 1 to 4: what the run is for, in the worker's and the
//! reviewer's prompts, and what it leaves out, in the reviewer's alone.
//! Pure: text from the run alone, no I/O (milestone 9.17 decision 3).

use super::messages::one_line;
use super::model::{Run, Task};
use super::orch::plan_frame::frame_for;
use crate::lessons::dossier::break_markers;

/// Decision 3's heading of the plan frames' items (exact).
pub const OUT_OF_SCOPE_HEAD: &str = "Out of scope for this run (a change that does one of these is a finding; a brief that asks for one is a brief finding):";
/// Decision 3's heading of a design run's approved Non-goals (exact).
pub const NON_GOALS_HEAD: &str = "Non-goals (from the approved spec):";
/// Decision 3: the whole block is at most this many bytes.
pub const OUT_OF_SCOPE_MAX: usize = 2048;
/// Room kept for the cut marker, as `requirements_block` keeps it.
const MARKER_ROOM: usize = 32;

/// Decision 1: the second line of the worker's and the reviewer's prompts.
pub fn goal_line(run: &Run) -> String {
    format!("Run goal: {}", run.goal)
}

/// Decision 3: the out-of-scope block of `task`'s reviewer prompt, or `None` when
/// there is nothing to list. A design run's approved Non-goals come first, then the
/// items of the frame of the task's round and of its epic's frame, each once, one line
/// each. A text cannot forge a prompt line or a `--- … ---` marker.
pub fn out_of_scope_block(run: &Run, task: &Task) -> Option<String> {
    let run_items = frame_for(run, task.round).map(|f| f.out_of_scope.as_slice());
    let epic_items = (task.spec.epic.as_deref())
        .and_then(|e| run.orch.epics.iter().find(|r| r.epic == e))
        .and_then(|r| r.frame.as_ref())
        .map(|f| f.out_of_scope.as_slice());
    let mut items: Vec<String> = Vec::new();
    for item in run_items.into_iter().chain(epic_items).flatten() {
        let clean = break_markers(one_line(item).trim());
        if !clean.is_empty() && !items.contains(&clean) {
            items.push(clean);
        }
    }
    let non_goals = (run.orch.design.as_ref())
        .map(|d| d.non_goals_section.trim())
        .filter(|text| !text.is_empty());
    let mut lines: Vec<String> = Vec::new();
    if let Some(text) = non_goals {
        lines.push(NON_GOALS_HEAD.to_string());
        lines.extend(text.lines().map(|l| break_markers(one_line(l).trim_end())));
    }
    if !items.is_empty() {
        lines.push(OUT_OF_SCOPE_HEAD.to_string());
        lines.extend(items.iter().map(|item| format!("- {item}")));
    }
    if lines.is_empty() {
        return None;
    }
    Some(capped(lines.join("\n")))
}

/// `full`, or its first bytes on a character and `[cut: <n> bytes]`, within
/// [`OUT_OF_SCOPE_MAX`].
fn capped(full: String) -> String {
    if full.len() <= OUT_OF_SCOPE_MAX {
        return full;
    }
    let mut kept = OUT_OF_SCOPE_MAX - MARKER_ROOM;
    while !full.is_char_boundary(kept) {
        kept -= 1;
    }
    format!("{}\n[cut: {} bytes]", &full[..kept], full.len() - kept)
}

#[cfg(test)]
#[path = "contract_scope_tests.rs"]
mod tests;
