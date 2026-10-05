//! Milestone 9.6 decision 27 (task M9.6.13): REPORT.md's `## Requirements` section, its
//! outcomes and the design phases' spend line; a run without the design flow's report
//! is 9.5's.

use proto::{AgentRole, DocAuthor, DocKind, Effort, Route, Runtime, Strength, TaskState};

use super::*;
use crate::run::design::state::{AgentSpend, DesignState, NewDoc, Requirement, store};
use crate::run::model::Run;
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

fn plain_run() -> Run {
    let tasks =
        ["t1", "t2", "t3"].map(|id| task_toml(id, "S", &format!("[\"crates/{id}/**\"]"), ""));
    run_ok(&plan_with(PROFILE, &tasks))
}

fn spend(label: &str, role: AgentRole, (calls, tokens): (u32, u64)) -> AgentSpend {
    AgentSpend {
        label: label.into(),
        role,
        route: Route {
            runtime: Runtime::Codex,
            model: "m".into(),
            strength: Strength::Frontier,
            effort: Effort::High,
        },
        sessions: 1,
        calls,
        tokens,
        secs: 0,
        outcome: "ok".into(),
    }
}

/// R1 is covered by t1 (merged) and t2 (blocked), R2 by t1 alone, R3 by t3 (merged);
/// R4 by none. Two brainstorm versions, one spec, one plan; the phases' spend.
fn design_run() -> Run {
    let mut run = plain_run();
    let requirement = |id: &str| Requirement {
        id: id.into(),
        text: format!("{id} text"),
    };
    run.orch.design = Some(DesignState {
        requirements: ["R1", "R2", "R3", "R4"].map(requirement).to_vec(),
        ..DesignState::default()
    });
    let covers = [vec!["R1", "R2"], vec!["R1"], vec!["R3"]];
    let states = [TaskState::Merged, TaskState::Blocked, TaskState::Merged];
    for (k, (covers, state)) in covers.into_iter().zip(states).enumerate() {
        run.tasks[k].spec.covers = covers.into_iter().map(String::from).collect();
        run.tasks[k].state = state;
    }
    for kind in [
        DocKind::Brainstorm,
        DocKind::Brainstorm,
        DocKind::Spec,
        DocKind::Plan,
    ] {
        let doc = NewDoc::new(kind, DocAuthor::Orchestrator, "submitted", "text");
        store(&mut run, doc, 1_000).unwrap();
    }
    let design = run.orch.design.as_mut().unwrap();
    let brainstormer = AgentRole::Brainstormer;
    design.add_session(
        1,
        "brainstorming",
        spend("claude", brainstormer, (30, 400_000)),
    );
    design.add_session(
        1,
        "brainstorming",
        spend("codex", brainstormer, (25, 300_000)),
    );
    design.add_session(
        1,
        "specifying",
        spend("spec-r1", AgentRole::DocReviewer, (9, 50_000)),
    );
    design.add_phase_secs(1, "brainstorming", 600);
    design.add_phase_secs(1, "specifying", 659);
    design.add_phase_secs(1, "planning", 300);
    run
}

const SECTION: &str = "
## Requirements

| Req | Tasks | Outcome |
|---|---|---|
| R1 | t1, t2 | merged, blocked |
| R2 | t1 | merged |
| R3 | t3 | merged |
| R4 | - | not covered |

design phases: 64 calls, 750000 tokens, 25 min, 4 gate versions
";

/// Decision 27: a row per approved requirement, its covering tasks and their outcome
/// (`merged` when all merged, else their states), then the spend line exactly; the
/// section sits after the tasks and before the tiers.
#[test]
fn report_md_has_the_requirements_table_with_outcomes() {
    let run = design_run();
    let mut out = String::new();
    section(&run, &mut out);
    assert_eq!(out, SECTION);
    let report = super::super::report::render(&run, 2_000);
    let at = |needle: &str| {
        report
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} in {report}"))
    };
    assert!(at("\n## t3:") < at("\n## Requirements"), "{report}");
    assert!(at("\n## Requirements") < at("\n## Log"), "{report}");
    assert!(report.contains(SECTION), "{report}");
}

/// A design run before its spec is approved has no table, only the spend line.
#[test]
fn a_design_run_without_an_approved_spec_shows_only_the_spend() {
    let mut run = plain_run();
    run.orch.design = Some(DesignState::default());
    let mut out = String::new();
    section(&run, &mut out);
    assert_eq!(
        out,
        "\n## Requirements\n\nNo spec was approved.\n\ndesign phases: 0 calls, 0 tokens, 0 min, 0 gate versions\n"
    );
}

/// The addendum: a run without the design flow writes no section, so its REPORT.md is
/// 9.5's, byte for byte, even when a task carries `covers`.
#[test]
fn a_non_design_report_is_unchanged() {
    let mut run = plain_run();
    let before = super::super::report::render(&run, 2_000);
    run.tasks[0].spec.covers = vec!["R1".into()];
    let mut out = String::new();
    section(&run, &mut out);
    assert_eq!(out, "");
    assert_eq!(super::super::report::render(&run, 2_000), before);
}
