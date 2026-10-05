//! Milestone 9.6 task M9.6.15, fix round 1 (ruling T15-7, m1 and m6): a later round's
//! log line and the orchestrator's note, pinned word for word. An `amend` round says
//! how its amendment is written; a `full` round starts with ruling T14-3's step; an
//! `off` round is 9.3's, with neither. A back at an `amend` round's spec gate is
//! refused: the round never brainstormed.

use proto::{DocGateAction, DocGateKind, RoundDesign};

use super::design_fixture::*;
use super::design_rounds_fixture::*;
use super::fixture::RUN_ID;

/// How round 2's amendment of R1 and R2 is written.
const HOW: &str = "submit only its new and changed requirements with submit_doc kind \"spec\" and amend true, new ones from R3 on and a changed one under its own number, and plan only those";

/// Round 2 started with `design`, the run asking `max_questions`: its new log lines and
/// notes.
fn round_two(design: RoundDesign, max_questions: u32) -> (Vec<String>, Vec<String>) {
    let mut fx = design_complete();
    fx.run_mut().limits.orch.design.max_questions = max_questions;
    let (lines, said) = (log_lines(&fx).len(), notes(&fx).len());
    iterate_with(&mut fx, Some(design)).unwrap();
    let new = |all: Vec<String>, from: usize| all[from..].to_vec();
    (new(log_lines(&fx), lines), new(notes(&fx), said))
}

/// 9.3's lines of round 2's start, with `own` (the design round's line) between them.
fn lines_with(own: Option<&str>) -> Vec<String> {
    let widens = format!("round 2 widens the run to stages: branches anthrex/{RUN_ID}/stage-<n>");
    let mut lines = vec![widens];
    lines.extend(own.map(String::from));
    lines.push("round 2 started by the user".into());
    lines
}

#[test]
fn an_amend_rounds_line_and_note() {
    let (lines, notes) = round_two(RoundDesign::Amend, 3);
    assert_eq!(lines, lines_with(Some("round 2 amends the spec")));
    assert_eq!(notes, [format!("round 2 amends the spec: {HOW}")]);
}

#[test]
fn a_full_rounds_note_starts_with_rule_48_or_no_questions() {
    let (lines, notes) = round_two(RoundDesign::Full, 3);
    let own = "round 2 brainstorms, then amends the spec";
    assert_eq!(lines, lines_with(Some(own)));
    assert_eq!(
        notes,
        [format!(
            "round 2 starts at the brainstorm. Then follow rule 48. Its spec is then an amendment: {HOW}"
        )]
    );
    let (_, notes) = round_two(RoundDesign::Full, 0);
    assert_eq!(
        notes,
        [format!(
            "round 2 starts at the brainstorm. Then call start_brainstorm with empty answers: this run asks no questions. Its spec is then an amendment: {HOW}"
        )]
    );
}

#[test]
fn an_off_round_has_no_line_and_no_note() {
    let (lines, notes) = round_two(RoundDesign::Off, 3);
    assert_eq!(lines, lines_with(None), "9.3's lines only");
    assert!(notes.is_empty(), "{notes:?}");
}

/// m6: an `amend` round has no brainstorm of its own to go back to.
#[test]
fn a_back_at_an_amend_rounds_spec_gate_is_refused() {
    let mut fx = amendment_at_gate();
    let back = DocGateAction::Back {
        note: "Compare more.".into(),
    };
    let refused = act(&mut fx, DocGateKind::Spec, back);
    assert_eq!(
        refused,
        Err("this round has no brainstorm to go back to".into())
    );
    assert_eq!(
        gate(&fx).map(|g| (g.0, g.2)),
        Some((DocGateKind::Spec, None))
    );
}
