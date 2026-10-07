//! Milestone 9.5 task 20b (ruling T20-1 (b)): `LaneInfo.kept`, a lane whose checkout was
//! kept because its racer did not exit in time. It round-trips inside a snapshot, is
//! not written while false, and a snapshot from before the field still decodes.

use super::*;

fn lane(lane: RaceLane, state: LaneState, kept: bool) -> LaneInfo {
    LaneInfo {
        lane,
        route: a_route(Runtime::Codex, Strength::Standard, Effort::MEDIUM, "gpt-6"),
        state,
        checkout: format!("t1.{}", lane.label()),
        head: Some("d1d1d1d".into()),
        reason: None,
        salvage_ref: Some("refs/anthrex/salvage/r/t1/1".into()),
        kept,
    }
}

/// A snapshot of one run whose task `t1` races: lane a won, lane b lost and was kept.
fn racing_snapshot() -> RunsSnapshot {
    let mut task = a_task_info();
    task.race = Some(RaceInfo {
        lanes: vec![
            lane(RaceLane::A, LaneState::Won, false),
            lane(RaceLane::B, LaneState::Lost, true),
        ],
        winner: Some(RaceLane::A),
        adopted: false,
    });
    let mut run = a_run_info();
    run.tasks = vec![task];
    RunsSnapshot {
        revision: 4,
        runs: vec![run],
        now: 1_700_000_000,
        proposals: Vec::new(),
        idle_orchestrators: Vec::new(),
    }
}

#[test]
fn a_kept_lane_round_trips_in_a_snapshot() {
    let snapshot = racing_snapshot();
    both_ways(&lane(RaceLane::B, LaneState::Out, true));
    both_ways(&snapshot);
    both_ways(&DaemonMsg::Run(crate::run_wire::RunReply::Snapshot(
        snapshot,
    )));
}

#[test]
fn kept_is_written_only_when_true() {
    let kept = serde_json::to_value(lane(RaceLane::B, LaneState::Lost, true)).unwrap();
    assert_eq!(kept["kept"], json!(true), "{kept}");
    let not = serde_json::to_value(lane(RaceLane::A, LaneState::Won, false)).unwrap();
    assert!(not.get("kept").is_none(), "{not}");
}

#[test]
fn a_snapshot_from_before_kept_decodes() {
    let mut value = serde_json::to_value(racing_snapshot()).unwrap();
    let lanes = value["runs"][0]["tasks"][0]["race"]["lanes"]
        .as_array_mut()
        .unwrap();
    for lane in lanes.iter_mut() {
        lane.as_object_mut().unwrap().remove("kept");
    }
    assert!(lanes.iter().all(|l| l.get("kept").is_none()));
    let check = |old: RunsSnapshot| {
        let race = old.runs[0].tasks[0].race.as_ref().expect("the race");
        assert_eq!(race.lanes.len(), 2);
        assert!(race.lanes.iter().all(|l| !l.kept), "absent reads false");
        assert_eq!(race.lanes[1].state, LaneState::Lost);
    };
    check(serde_json::from_value(value.clone()).expect("JSON from before kept"));
    // MessagePack, as a daemon from before the field sent it (named maps).
    let packed = rmp_serde::to_vec_named(&value).unwrap();
    check(rmp_serde::from_slice(&packed).expect("MessagePack from before kept"));
}
