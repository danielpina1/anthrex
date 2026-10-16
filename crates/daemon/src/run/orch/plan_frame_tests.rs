//! Milestone 9.17 task M9.17.3 (decision 18): the frame types, their lookup, and a run
//! from before them.

use super::*;
use crate::run::orch::test_support::{plan_frame, run_of};
use crate::run::orch::{EpicRecord, PlannerPhase, RunOrch};

#[test]
fn frame_for_finds_the_rounds_frame_only() {
    let mut run = run_of(1);
    assert_eq!(frame_for(&run, 1), None);
    run.orch.plan_frames = vec![
        plan_frame(1, &[("Which store?", "Postgres")], &["no UI"]),
        plan_frame(2, &[], &["no docs"]),
    ];
    assert_eq!(frame_for(&run, 1).unwrap().out_of_scope, ["no UI"]);
    assert_eq!(frame_for(&run, 2).unwrap().out_of_scope, ["no docs"]);
    assert_eq!(frame_for(&run, 3), None);
}

#[test]
fn frames_round_trip_and_are_left_out_while_empty() {
    let frame = plan_frame(2, &[("Which store?", "Postgres")], &["no UI"]);
    let json = serde_json::to_value(&frame).unwrap();
    assert_eq!(
        json,
        serde_json::json!({
            "round": 2,
            "decided": [{"question": "Which store?", "answer": "Postgres"}],
            "out_of_scope": ["no UI"],
            "at": 1_800_000_000u64,
        })
    );
    assert_eq!(serde_json::from_value::<PlanFrame>(json).unwrap(), frame);
    // A run and an epic without a frame write no key, and read back without one.
    let orch = serde_json::to_value(RunOrch::default()).unwrap();
    assert!(orch.get("plan_frames").is_none(), "{orch}");
    let epic = serde_json::to_value(EpicRecord::new("e1", PlannerPhase::Queued)).unwrap();
    assert!(epic.get("frame").is_none(), "{epic}");
    let mut with = EpicRecord::new("e1", PlannerPhase::Queued);
    with.frame = Some(frame.clone());
    let back: EpicRecord = serde_json::from_value(serde_json::to_value(&with).unwrap()).unwrap();
    assert_eq!(back.frame, Some(frame));
    let orch = RunOrch {
        plan_frames: vec![plan_frame(1, &[], &[])],
        ..RunOrch::default()
    };
    let back: RunOrch = serde_json::from_value(serde_json::to_value(&orch).unwrap()).unwrap();
    assert_eq!(back, orch);
}

#[test]
fn bounds_are_decision_13s() {
    assert_eq!((DECIDED_MAX, OUT_OF_SCOPE_ITEMS_MAX), (20, 20));
}
