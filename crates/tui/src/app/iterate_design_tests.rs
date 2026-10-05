//! Milestone 9.6 task 18: the iterate dialog opened on a design run has the `design`
//! row, showing the run's own round mode (`RunInfo.round_design`, task 17's field; a
//! design run's first round is `full`); on a run without the flow it has none.

use crate::app::{App, Modal};
use crate::run_iterate::IterateForm;
use crate::tree::run_fixtures::RUN_ID;
use crate::ui::doc_gate::tests::{app_with, design_run};
use proto::{DesignMode, DocGateKind, RoundDesign};

fn opened(run: proto::RunInfo) -> IterateForm {
    let mut app: App = app_with(run, 80, 24);
    assert!(app.open_iterate(RUN_ID).is_empty());
    match app.modal {
        Some(Modal::Iterate(form)) => form,
        other => panic!("no iterate dialog: {other:?}"),
    }
}

#[test]
fn a_design_run_opens_the_dialog_with_its_round_mode() {
    let mut run = design_run(DocGateKind::Spec);
    run.round = 2;
    run.round_design = Some(RoundDesign::Off);
    let form = opened(run);
    assert_eq!(form.design, Some(RoundDesign::Amend));
    assert_eq!(form.current, Some(RoundDesign::Off));
    // Round 1 of a design run is a full one.
    let form = opened(design_run(DocGateKind::Spec));
    assert_eq!(form.current, Some(RoundDesign::Full));
}

#[test]
fn a_run_without_the_flow_opens_the_dialog_without_the_row() {
    let mut run = design_run(DocGateKind::Spec);
    run.design = DesignMode::Off;
    run.doc_gate = None;
    let form = opened(run);
    assert_eq!((form.design, form.current), (None, None));
}
