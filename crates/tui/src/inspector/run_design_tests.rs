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
