//! Decision 22 (DF §5.1, task M9.6.11; ruling T11-1): what a sub-planner is given of
//! the approved spec. Its first turn ends with the requirements its epic owns, verbatim
//! from `DesignState.requirements`, then the spec's Goal and Interfaces sections. They
//! are the ids its `spawn_subplanner` named (`covers`); a re-plan's are those with the
//! ones its epic's tasks cover. An epic that names none (a design run's spawn is
//! refused without them; only a spawn before ruling T11-1 has none) gets every
//! requirement. Coverage is still checked over the whole graph. Pure.

use std::fmt::Write as _;

use proto::TaskState;

use crate::run::model::Run;
use crate::run::orch::EpicRecord;

/// The block appended to `epic`'s sub-planner's first turn; empty for a run without
/// the flow or before its spec's requirements are stored.
pub fn block(run: &Run, epic: &EpicRecord) -> String {
    let Some(design) = run.orch.design.as_ref() else {
        return String::new();
    };
    if design.requirements.is_empty() {
        return String::new();
    }
    let tasks = (run.tasks.iter())
        .filter(|t| t.state != TaskState::Cancelled && t.spec.epic.as_deref() == Some(&epic.epic))
        .flat_map(|t| t.spec.covers.iter());
    let cited: Vec<&str> = (epic.covers.iter().chain(tasks))
        .map(String::as_str)
        .collect();
    let mut out = String::from("\n\nSpec requirements this epic delivers:\n");
    let mine =
        (design.requirements.iter()).filter(|r| cited.is_empty() || cited.contains(&r.id.as_str()));
    for r in mine {
        let _ = writeln!(out, "{}  {}", r.id, r.text);
    }
    let _ = write!(out, "Goal (from the spec): {}", design.goal_section);
    if !design.interfaces_section.is_empty() {
        let _ = write!(
            out,
            "\nInterfaces (from the spec):\n{}",
            design.interfaces_section
        );
    }
    out
}
