//! Task M9.6.17: what the gate screen and `run status` read off the snapshot beside the
//! gate: why the orchestrator revises (`DocGateInfo.revising_cause`, from the gate's
//! `Revision`), the current round's design mode (`RunInfo.round_design`) and a halted
//! design run's phase (`RunInfo.halted_phase`).

use proto::{DocGateAction, DocGateKind, RevisingCause, RoundDesign, RunState};

use super::design_fixture::{act, at_plan_gate, at_spec_gate, budget_halted};
use super::design_rounds_fixture::{design_complete, iterate_with};
use super::fixture::*;
use crate::run::design::state::Revision;
use crate::run::snapshot::snapshot;

fn info(fx: &Fixture) -> proto::RunInfo {
    snapshot(&fx.state, fx.now).runs.remove(0)
}

fn cause(fx: &Fixture) -> Option<RevisingCause> {
    info(fx).doc_gate.map(|g| g.revising_cause)
}

#[test]
fn the_gate_names_why_the_orchestrator_revises() {
    let mut fx = at_spec_gate(false);
    assert_eq!(cause(&fx), Some(RevisingCause::Changes), "an open gate");
    let changes = DocGateAction::Changes {
        note: "Split R2.".into(),
        review: false,
    };
    act(&mut fx, DocGateKind::Spec, changes).unwrap();
    assert_eq!(cause(&fx), Some(RevisingCause::Changes));

    let mut fx = at_plan_gate(false);
    let back = DocGateAction::Back {
        note: "R2 is wrong.".into(),
    };
    act(&mut fx, DocGateKind::Plan, back).unwrap();
    let gate = info(&fx).doc_gate.expect("the spec gate reopened");
    assert_eq!(gate.kind, DocGateKind::Spec);
    assert_eq!(gate.revising.as_deref(), Some("R2 is wrong."));
    assert_eq!(gate.revising_cause, RevisingCause::Back);

    let design = fx.run_mut().orch.design.as_mut().unwrap();
    design.gate.as_mut().unwrap().cause = Revision::ReadBack;
    assert_eq!(cause(&fx), Some(RevisingCause::ReadBack));
}

#[test]
fn the_snapshot_names_the_rounds_design_mode() {
    let mut fx = design_complete();
    assert_eq!(info(&fx).round_design, None, "round 1 has no round mode");
    iterate_with(&mut fx, Some(RoundDesign::Off)).unwrap();
    assert_eq!(fx.run().round(), 2);
    assert_eq!(info(&fx).round_design, Some(RoundDesign::Off));

    let mut fx = design_complete();
    iterate_with(&mut fx, None).unwrap();
    assert_eq!(info(&fx).round_design, Some(RoundDesign::Amend));
}

#[test]
fn a_halted_design_run_names_the_phase_it_resumes_into() {
    let fx = at_spec_gate(false);
    assert_eq!(info(&fx).halted_phase, None, "not halted");
    let fx = budget_halted();
    let run = info(&fx);
    assert_eq!(run.state, RunState::Halted);
    assert_eq!(run.halted_phase, Some(RunState::Planning));
    // A run without the design flow names none.
    let (fx, _, _) = super::race::racing();
    assert_eq!(info(&fx).halted_phase, None);
}
