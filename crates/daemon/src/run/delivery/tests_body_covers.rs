//! Milestone 9.6 decision 31 (task M9.6.12): a design run's stage PR names the
//! requirements its tasks cover and the committed spec, on one line after the stack;
//! a run without the flow, or whose documents were never committed, has the body of
//! 9.5 byte for byte. Pure.

use proto::DesignMode;

use super::super::tests::{staged, task_mut};
use super::pr_body;
use crate::run::design::state::{DesignState, Requirement};
use crate::run::model::Run;

const SPEC: &str = "docs/anthrex/specs/2026-10-05-password-reset.md";

fn requirement(id: &str) -> Requirement {
    Requirement {
        id: id.into(),
        text: format!("{id} text"),
    }
}

/// [`staged`] as a design run whose spec (R1 to R3) was committed at [`SPEC`], `t2`
/// covering R2 and `t3` covering R1 and R2 (stage 2), `t4` covering R3 (stage 3).
fn designed() -> Run {
    let mut run = staged();
    run.design_mode = DesignMode::Full;
    run.orch.design = Some(DesignState {
        requirements: ["R1", "R2", "R3"].map(requirement).to_vec(),
        committed: Some("c".repeat(40)),
        spec_path: Some(SPEC.into()),
        ..DesignState::default()
    });
    task_mut(&mut run, "t2").spec.covers = vec!["R2".into()];
    task_mut(&mut run, "t3").spec.covers = vec!["R2".into(), "R1".into()];
    task_mut(&mut run, "t4").spec.covers = vec!["R3".into()];
    run
}

/// Decision 31: `Covers <ids> (spec: <path>)`, the ids verbatim in the spec's order
/// (ruling T4-2), after the stack line.
#[test]
fn a_pr_body_names_the_requirements_its_stage_covers() {
    let run = designed();
    let body = pr_body(&run, 2);
    let stack = "**Stack:** based on stage 1, #141\n";
    let want = format!("{stack}Covers R1, R2 (spec: {SPEC})\n\n### Look here first\n");
    assert!(body.contains(&want), "{body}");
    let body = pr_body(&run, 3);
    assert!(
        body.contains(&format!("Covers R3 (spec: {SPEC})\n")),
        "{body}"
    );
    // A stage whose tasks cover nothing has no line.
    let body = pr_body(&run, 1);
    assert!(!body.contains("Covers"), "{body}");
}

/// A run without the flow, even with `covers` on its tasks, and a design run whose
/// documents were not committed (`docs_dir = ""`), have 9.5's body exactly.
#[test]
fn a_body_without_a_committed_spec_is_9_5_s() {
    let plain = staged();
    let mut covered = designed();
    covered.design_mode = DesignMode::Off;
    covered.orch.design = None;
    assert_eq!(pr_body(&covered, 2), pr_body(&plain, 2));
    let mut uncommitted = designed();
    if let Some(design) = uncommitted.orch.design.as_mut() {
        design.committed = None;
        design.spec_path = None;
    }
    assert_eq!(pr_body(&uncommitted, 2), pr_body(&plain, 2));
    assert!(!pr_body(&plain, 2).contains("Covers"));
}
