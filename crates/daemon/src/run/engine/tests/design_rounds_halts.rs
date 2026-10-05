//! Milestone 9.6 task M9.6.15, fix round 1 (ruling T15-4, task 7's carry): a design run
//! halted at an open design gate, not revising, resumes to that gate. `--rebaseline`
//! there is refused with ruling T7-1's text, so the run never reaches `running` without
//! an approved plan, and the gate's resume starts no phase clock.

use proto::{DocGateKind, RunState};
use serde_json::json;

use super::design_clock::resume_rebaselined;
use super::design_fixture::*;
use super::design_rounds::round_task;
use super::design_rounds_fixture::*;
use super::dispatch::replies;
use super::fixture::*;
use super::orch_restore::resume;

/// `fx` halted, retryably or not, as a delivery result can halt it.
fn halted(fx: &mut Fixture, retryable: bool) {
    let now = fx.now;
    let run = fx.run_mut();
    crate::run::engine::merge::halt(run, "remote stage branch moved".into(), now);
    run.halt_retryable = retryable;
    assert_eq!(fx.run().state, RunState::Halted);
}

/// Round 2 at its open spec gate, or at its open plan gate.
fn at_gate(kind: DocGateKind) -> Fixture {
    match kind {
        DocGateKind::Spec => amendment_at_gate(),
        _ => round_plan_gate(json!([round_task("t2", &["R2", "R3"])])),
    }
}

/// Ruling T15-4: a halt at round 2's open gate, retryable or not. `--rebaseline` is
/// refused with T7-1's text, naming the gate's phase, and changes nothing; the plain
/// resume returns the run to the same gate, with no phase clock.
#[test]
fn a_halt_at_an_open_round_gate_resumes_to_the_gate() {
    for (kind, phase) in [
        (DocGateKind::Spec, "specifying"),
        (DocGateKind::Plan, "planning"),
    ] {
        for retryable in [false, true] {
            let mut fx = at_gate(kind);
            let version = gate(&fx).unwrap().1;
            halted(&mut fx, retryable);
            let refusal =
                format!("run {RUN_ID} halted in its {phase} phase; resume it without --rebaseline");
            assert_eq!(resume_rebaselined(&mut fx), vec![Err(refusal)], "{kind:?}");
            assert_eq!(fx.run().state, RunState::Halted);
            let answer = replies(&resume(&mut fx));
            assert_eq!(
                answer,
                vec![Ok(format!("run {RUN_ID} resumed"))],
                "{kind:?}"
            );
            let run = fx.run();
            assert_eq!(
                run.state,
                RunState::AwaitingApproval,
                "{kind:?} {retryable}"
            );
            assert_eq!(gate(&fx), Some((kind, version, None)));
            let design = run.orch.design.as_ref().unwrap();
            assert_eq!(design.phase_started, None, "no clock at an open gate");
            assert_eq!(design.halted_from, None);
            let kind_name = match kind {
                DocGateKind::Spec => "spec",
                _ => "plan",
            };
            let line = format!("resumed; the run waits at its {kind_name} gate again");
            assert!(
                run.log.iter().any(|l| l.text == line),
                "{line}: {:#?}",
                run.log
            );
        }
    }
}

/// Ruling T15-4: a gate the orchestrator is revising is a phase; its resume restarts
/// the phase's clock as before.
#[test]
fn a_halt_at_a_revising_gate_restarts_its_clock() {
    let mut fx = plan_revising();
    halted(&mut fx, false);
    assert!(replies(&resume(&mut fx))[0].is_ok());
    let run = fx.run();
    assert_eq!(run.state, RunState::AwaitingApproval);
    let design = run.orch.design.as_ref().unwrap();
    assert!(design.phase_started.is_some());
}
