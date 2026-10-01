//! Ruling C-24: a stage's tier 3 shows `running` while its job is in flight, below
//! `bisecting`.

use proto::FullState;

use super::full::{full_job, merge_tiered, profile, verify_ok};
use super::merge::{commit, doc_task, window_of};
use super::propagate::{land_propagates, stages_on};
use crate::run::snapshot_stages::stage_infos;

#[test]
fn a_running_tier3_shows_running_below_bisecting() {
    let tasks = [doc_task("t1", ""), doc_task("t2", "stage = 2")];
    let (mut fx, windows) = stages_on(&profile(), &tasks);
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    land_propagates(&mut fx);
    merge_tiered(&mut fx, "t2", window_of(&windows, "t2"), &commit(2));
    verify_ok(&mut fx);
    let (_, spec) = full_job(&fx);
    assert_eq!(spec.stage, 1);
    let states = |fx: &super::fixture::Fixture| -> Vec<FullState> {
        stage_infos(fx.run()).iter().map(|s| s.full.state).collect()
    };
    assert_eq!(states(&fx), [FullState::Running, FullState::None]);
    // A bisect of the stage outranks the job in flight.
    let head = fx.run().stage(1).unwrap().head.clone();
    fx.run_mut().stages[0].bisect = Some(
        serde_json::from_value(serde_json::json!({
            "head": head, "tests": ["a::works"], "base": head, "candidates": [],
            "lo": 0, "hi": 1
        }))
        .unwrap(),
    );
    assert_eq!(states(&fx), [FullState::Bisecting, FullState::None]);
}
