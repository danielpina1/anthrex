//! Decision 22 (DF §5.1, task M9.6.11): what a sub-planner is given of the approved
//! spec. Its first turn ends with the requirements its epic's tasks cover, verbatim
//! from `DesignState.requirements`, then the spec's Goal and Interfaces sections. An
//! epic whose tasks cover nothing yet (the orchestrator wrote no interface task for it)
//! gets every requirement. Coverage is still checked over the whole graph. Pure.

use std::fmt::Write as _;

use proto::TaskState;

use crate::run::model::Run;

/// The block appended to `epic`'s sub-planner's first turn; empty for a run without
/// the flow or before its spec's requirements are stored.
pub fn block(run: &Run, epic: &str) -> String {
    let Some(design) = run.orch.design.as_ref() else {
        return String::new();
    };
    if design.requirements.is_empty() {
        return String::new();
    }
    let cited: Vec<&str> = (run.tasks.iter())
        .filter(|t| t.state != TaskState::Cancelled && t.spec.epic.as_deref() == Some(epic))
        .flat_map(|t| t.spec.covers.iter().map(String::as_str))
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
