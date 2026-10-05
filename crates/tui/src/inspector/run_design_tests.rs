//! Task M9.6.17 (task 16's m3): the run inspector says what `anthrex run status` says
//! of a design run's `off` round and of a halted design run's phase.

use super::run_tests::{app_of, inspect_node, pairs, value};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{RUN_ID, gate_fixture};
use proto::{DesignMode, RoundDesign, RunState};

fn design_value(change: impl FnOnce(&mut proto::RunInfo)) -> Option<String> {
    let (mut snap, windows) = gate_fixture();
    snap.runs[0].design = DesignMode::Full;
    change(&mut snap.runs[0]);
    let app = app_of((snap, windows));
    let run = inspect_node(&app, &NodeKey::Run(RUN_ID.into()));
    value(&run, "design").map(str::to_owned)
}

#[test]
fn the_run_inspector_names_an_off_round_and_a_halted_phase() {
    for state in [RunState::Planning, RunState::AwaitingApproval] {
        let off = design_value(|run| {
            run.state = state;
            run.round = 2;
            run.round_design = Some(RoundDesign::Off);
        });
        assert_eq!(off.as_deref(), Some("off this round"), "{state:?}");
    }
    let running = design_value(|run| {
        run.state = RunState::Running;
        run.round_design = Some(RoundDesign::Off);
    });
    assert_eq!(running, None);
    let halted = design_value(|run| {
        run.state = RunState::Halted;
        run.halted_phase = Some(RunState::Specifying);
    });
    assert_eq!(halted.as_deref(), Some("halted in specifying"));
    // Without the design flow, no row.
    let off = design_value(|run| {
        run.design = DesignMode::Off;
        run.state = RunState::Halted;
        run.halted_phase = Some(RunState::Specifying);
    });
    assert_eq!(off, None);
}

/// Ruling T18-1: a design agent's inspection, its state, runtime and session count,
/// and a reviewer's document and review number.
#[test]
fn a_design_agents_inspection_names_its_state_runtime_and_sessions() {
    let (mut snap, windows) = gate_fixture();
    snap.runs[0] = crate::tree::run_rows::design_tests::with_agents();
    let app = app_of((snap, windows));
    let key = |label: &str| NodeKey::DesignAgent {
        run: RUN_ID.into(),
        label: label.into(),
    };
    let codex = inspect_node(&app, &key("codex"));
    assert_eq!(codex.name, "brainstormer codex");
    assert_eq!(codex.right.as_deref(), Some("running · 2 sessions"));
    assert_eq!(
        pairs(&codex),
        [
            ("state", "running"),
            ("runtime", "codex"),
            ("sessions", "2"),
            ("session", "#41 · window closed"),
        ]
    );
    let reviewer = inspect_node(&app, &key("spec-r2"));
    assert_eq!(reviewer.name, "doc reviewer spec-r2");
    assert_eq!(reviewer.right.as_deref(), Some("done"));
    assert_eq!(
        pairs(&reviewer),
        [
            ("state", "done"),
            ("runtime", "codex"),
            ("document", "spec, review 2"),
            ("sessions", "1"),
            ("session", "no window yet"),
        ]
    );
}

/// Ruling T18-5 (final fix wave FW-57): at a design gate the inspector's `gate` line
/// lists that gate kind's own keys, as its screen offers them (the brainstorm and spec
/// gates' document screen, the plan gate's plan review), never 9.5's task-gate keys.
/// While the orchestrator revises, only the reject acts.
#[test]
fn the_gate_line_lists_the_design_gates_own_keys() {
    let gate_value = |kind: proto::DocGateKind, revising: Option<&str>| {
        let (mut snap, windows) = gate_fixture();
        let run = &mut snap.runs[0];
        run.design = DesignMode::Full;
        run.state = RunState::AwaitingApproval;
        run.doc_gate = Some(proto::DocGateInfo {
            kind,
            version: 2,
            revising: revising.map(str::to_owned),
            disputed: Vec::new(),
            not_reviewed: None,
            changes_summary: Vec::new(),
            same_runtime: false,
            report: None,
            revising_cause: proto::RevisingCause::Changes,
        });
        let app = app_of((snap, windows));
        let run = inspect_node(&app, &NodeKey::Run(RUN_ID.into()));
        value(&run, "gate").map(str::to_owned)
    };
    use proto::DocGateKind::{Brainstorm, Plan, Spec};
    assert_eq!(
        gate_value(Brainstorm, None).as_deref(),
        Some("brainstorm v2 · a approve · c changes · e edit · r rethink · x reject · g drafts")
    );
    assert_eq!(
        gate_value(Spec, None).as_deref(),
        Some("spec v2 · a approve · c changes · e edit · b back · x reject")
    );
    assert_eq!(
        gate_value(Plan, None).as_deref(),
        Some("plan v2 · a approve · c changes · e edit · b back · x reject · d remove")
    );
    assert_eq!(
        gate_value(Spec, Some("split R1")).as_deref(),
        Some("spec v2 being revised · x reject")
    );
}
