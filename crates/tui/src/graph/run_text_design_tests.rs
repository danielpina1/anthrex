//! Milestone 9.6 task 18 (DF §6.1, §6.2): the orchestrator node of a design run names
//! its phase, `⏸ <kind> v<n>` while a gate waits for the user, and the phase a halted
//! run resumes into; an `off` round and a run without the flow read as before.

use super::*;
use crate::tree::run_fixtures::{PROJECT, RUN_ID, run, task};
use proto::{DesignMode, DocGateInfo, DocGateKind, RevisingCause, RoundDesign, Size};

fn gate(kind: DocGateKind, version: u32, revising: Option<&str>) -> DocGateInfo {
    DocGateInfo {
        kind,
        version,
        revising: revising.map(str::to_owned),
        disputed: Vec::new(),
        not_reviewed: None,
        changes_summary: Vec::new(),
        same_runtime: false,
        report: None,
        revising_cause: RevisingCause::Changes,
    }
}

fn design(state: RunState) -> RunInfo {
    let mut info = run(RUN_ID, PROJECT, state);
    info.design = DesignMode::Full;
    info.tasks = vec![task("t1", "a", Size::S, TaskState::Pending)];
    info
}

#[test]
fn the_run_view_shows_the_phase_and_the_open_gate() {
    let both = |info: &RunInfo| {
        (
            run_text_in(info, true, false),
            run_text_in(info, true, true),
        )
    };
    for (state, word) in [
        (RunState::Brainstorming, "brainstorming"),
        (RunState::Specifying, "specifying"),
        (RunState::Planning, "planning"),
    ] {
        let info = design(state);
        assert_eq!(run_text(&info, true), format!("orchestrator  {word}"));
        assert_eq!(run_text(&info, false), format!("run 3f9a  {word}"));
    }
    let mut info = design(RunState::AwaitingApproval);
    for (kind, version, want) in [
        (DocGateKind::Brainstorm, 1, "brainstorm v1"),
        (DocGateKind::Spec, 2, "spec v2"),
        (DocGateKind::Plan, 4, "plan v4"),
    ] {
        info.doc_gate = Some(gate(kind, version, None));
        assert_eq!(
            both(&info),
            (
                format!("orchestrator  ⏸ {want}"),
                format!("orchestrator  || {want}")
            )
        );
    }
    // While the orchestrator revises, the gate's phase: nothing waits for the user.
    info.doc_gate = Some(gate(DocGateKind::Spec, 2, Some("split R1")));
    assert_eq!(run_text(&info, true), "orchestrator  specifying");
    info.doc_gate = Some(gate(DocGateKind::Brainstorm, 1, Some("again")));
    assert_eq!(run_text(&info, true), "orchestrator  brainstorming");
    // Halted from a design phase: the phase its plain resume returns to.
    let mut halted = design(RunState::Halted);
    halted.halted_phase = Some(RunState::Specifying);
    assert_eq!(
        run_text(&halted, true),
        "orchestrator  halted in specifying"
    );
    assert_eq!(run_text(&halted, false), "run 3f9a  halted in specifying");
    // Halted after the plan was approved: its progress, as before.
    halted.halted_phase = None;
    assert_eq!(run_text(&halted, true), "orchestrator  0/1");
}

#[test]
fn an_off_round_and_a_run_without_the_flow_read_as_before() {
    // An `off` round of a design run: 9.3's round, nothing of the flow.
    let mut info = design(RunState::Planning);
    info.round = 2;
    info.round_design = Some(RoundDesign::Off);
    assert_eq!(run_text(&info, true), "orchestrator  planning");
    info.state = RunState::AwaitingApproval;
    assert_eq!(run_text_in(&info, true, false), "orchestrator  0/1");
    info.state = RunState::Halted;
    info.halted_phase = Some(RunState::Planning);
    assert_eq!(run_text(&info, true), "orchestrator  0/1");
    // A run without the flow, at its gate and halted.
    for state in [
        RunState::AwaitingApproval,
        RunState::Halted,
        RunState::Running,
    ] {
        let mut plain = run(RUN_ID, PROJECT, state);
        plain.tasks = vec![task("t1", "a", Size::S, TaskState::Pending)];
        assert_eq!(run_text_in(&plain, true, false), "orchestrator  0/1");
        assert_eq!(run_text_in(&plain, false, true), "run 3f9a  0/1");
    }
}
