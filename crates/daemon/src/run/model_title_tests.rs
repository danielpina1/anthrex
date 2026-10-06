//! The run title: `Run.title` persists through `run.json` (`journal::save_run` and
//! `journal::load_all`), and a protocol-17 `run.json`, which has none, loads with an
//! empty title and writes none back.

use super::Run;
use super::tuning::tests::{save_and_load, tmp};

/// Milestone 9.5's captured `run.json` (see `model_design_tests.rs`).
const M95_RUN: &str = include_str!("../../tests/fixtures/run/m95-run.json");

fn old_run() -> Run {
    serde_json::from_str(M95_RUN).expect("m95-run.json parses")
}

#[test]
fn an_old_run_json_loads_with_no_title() {
    let captured: serde_json::Value = serde_json::from_str(M95_RUN).unwrap();
    assert!(
        captured.get("title").is_none(),
        "the fixture predates the title"
    );
    let mut run = old_run();
    assert_eq!(run.title, "");
    let dir = tmp();
    let (loaded, text) = save_and_load(&mut run, dir.path());
    assert_eq!(loaded.title, "");
    let again: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert!(again.get("title").is_none(), "an empty title is left out");
}

#[test]
fn a_title_round_trips_through_run_json() {
    let mut run = old_run();
    run.title = "Password reset flow".into();
    let dir = tmp();
    let (loaded, text) = save_and_load(&mut run, dir.path());
    assert_eq!(loaded, run);
    assert_eq!(loaded.title, "Password reset flow");
    let again: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(again["title"], "Password reset flow");
}
