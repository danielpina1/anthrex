//! Milestone 9.6 decision 27 (DF §5.2, §14 risk 1): REPORT.md's `## Requirements`, a
//! design run's table of each approved requirement, the tasks that cover it and their
//! outcome, and one line with what the design phases spent. A run without the design
//! flow has no section, so its report is 9.5's. Pure, as `report.rs`.

use proto::{DocKind, TaskState};

use super::model::{Run, Task};
use super::report_escape::escape_cell;

/// The section, after the tasks (`report.rs::render`): the table of the approved
/// spec's requirements (none before its approval), then the spend line, exactly
/// `design phases: <calls> calls, <tokens> tokens, <m> min, <v> gate versions`: the
/// design agents' calls and tokens, the orchestrator phases' clock time in whole
/// minutes, and the three gates' versions.
pub fn section(run: &Run, out: &mut String) {
    let Some(design) = run.orch.design.as_ref() else {
        return;
    };
    out.push_str("\n## Requirements\n\n");
    if design.requirements.is_empty() {
        out.push_str("No spec was approved.\n");
    } else {
        out.push_str("| Req | Tasks | Outcome |\n|---|---|---|\n");
    }
    for requirement in &design.requirements {
        let tasks: Vec<&Task> = (run.tasks.iter())
            .filter(|t| t.spec.covers.contains(&requirement.id))
            .collect();
        let ids: Vec<&str> = tasks.iter().map(|t| t.spec.id.as_str()).collect();
        let (ids, outcome) = match tasks.is_empty() {
            true => ("-".to_string(), "not covered".to_string()),
            false => (ids.join(", "), outcome(&tasks)),
        };
        let id = escape_cell(&requirement.id);
        out.push_str(&format!("| {id} | {} | {outcome} |\n", escape_cell(&ids)));
    }
    let totals = design.spend_totals();
    let versions: u32 = [DocKind::Brainstorm, DocKind::Spec, DocKind::Plan]
        .into_iter()
        .map(|kind| design.gate_versions(kind))
        .sum();
    out.push_str(&format!(
        "\ndesign phases: {} calls, {} tokens, {} min, {versions} gate versions\n",
        totals.calls,
        totals.tokens,
        totals.secs / 60
    ));
}

/// `merged` when every covering task merged, else their states, comma-joined.
fn outcome(tasks: &[&Task]) -> String {
    if tasks.iter().all(|t| t.state == TaskState::Merged) {
        return "merged".to_string();
    }
    let states: Vec<&str> = tasks.iter().map(|t| t.state.label()).collect();
    states.join(", ")
}

#[cfg(test)]
#[path = "report_design_tests.rs"]
mod tests;
