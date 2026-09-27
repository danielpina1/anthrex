//! Milestone 8c task 1: the snapshot fields the live run view reads, and the
//! placeholders later milestones fill. Every one is `#[serde(default)]`, so a snapshot
//! without them still decodes.

use super::fixtures::*;
use crate::messages::DaemonMsg;
use crate::run::{Effort, RouteSpec, Runtime, Strength};
use crate::run_info::{PlanEditInfo, RunsSnapshot, TaskEventInfo};
use crate::run_wire::RunReply;
use crate::{PlannerInfo, PlannerState};

fn a_planner() -> PlannerInfo {
    PlannerInfo {
        epic: "A".into(),
        title: "daemon".into(),
        area: vec!["crates/daemon/**".into()],
        route: a_route(Runtime::Claude, Strength::Frontier, Effort::High, ""),
        window_id: Some(12),
        state: PlannerState::Finished,
        started_at: 1_700_000_050,
        ended_at: Some(1_700_000_900),
        edits_accepted: 3,
        edits_rejected: 1,
        last_rejection: Some("t9 owns overlap t2".into()),
        replans: vec!["split t4".into()],
    }
}

/// Every new field set, each to a value no sibling of the same type has.
fn a_view_snapshot() -> RunsSnapshot {
    let mut run = a_run_info();
    run.approved_at = Some(1_700_000_010);
    run.plan_edits = vec![
        PlanEditInfo {
            at: 1_700_000_300,
            text: "split t2".into(),
        },
        PlanEditInfo {
            at: 1_700_000_200,
            text: "amend t1".into(),
        },
    ];
    run.plan_edits_since_approval = 2;
    run.planners = vec![a_planner()];
    run.estimate_left_secs = Some(1_800);
    run.bound_ratio_permille = Some(1_250);
    let task = &mut run.tasks[0];
    task.brief = "Add the reset token model.".into();
    task.acceptance = vec!["tokens expire after one hour".into()];
    task.route_spec = RouteSpec {
        runtime: Some(Runtime::Codex),
        model: Some("gpt-5-codex".into()),
        strength: None,
        effort: Some(Effort::Low),
    };
    task.history = vec![
        TaskEventInfo {
            at: 1_700_000_500,
            text: "review round 1 requested changes".into(),
        },
        TaskEventInfo {
            at: 1_700_000_400,
            text: "dispatched".into(),
        },
    ];
    let round = &mut task.rounds[0];
    round.rate_limited_since = Some(1_700_000_120);
    round.rate_limited_until = Some(1_700_000_420);
    round.sent_back_at = vec![1_700_000_150, 1_700_000_250];
    RunsSnapshot {
        revision: 42,
        runs: vec![run],
        now: 1_700_001_000,
    }
}

#[test]
fn snapshot_view_fields_round_trip() {
    let msg = DaemonMsg::Run(RunReply::Snapshot(a_view_snapshot()));
    let packed = rmp_serde::to_vec_named(&msg).unwrap();
    let back: DaemonMsg = rmp_serde::from_slice(&packed).unwrap();
    assert_eq!(back, msg);
    let DaemonMsg::Run(RunReply::Snapshot(snap)) = back else {
        panic!("must decode back to RunReply::Snapshot");
    };
    // By name, so a swap between same-typed fields cannot survive (see run_tests.rs).
    assert_eq!(snap.now, 1_700_001_000);
    assert_eq!(snap.revision, 42);
    let run = &snap.runs[0];
    assert_eq!(run.approved_at, Some(1_700_000_010));
    assert_eq!(run.plan_edits[0].at, 1_700_000_300);
    assert_eq!(run.plan_edits[0].text, "split t2");
    assert_eq!(run.plan_edits[1].text, "amend t1");
    assert_eq!(run.plan_edits_since_approval, 2);
    assert_eq!(run.estimate_left_secs, Some(1_800));
    assert_eq!(run.bound_ratio_permille, Some(1_250));
    assert_eq!(run.planners, vec![a_planner()]);
    let planner = &run.planners[0];
    assert_eq!(
        (planner.started_at, planner.ended_at),
        (1_700_000_050, Some(1_700_000_900))
    );
    assert_eq!((planner.edits_accepted, planner.edits_rejected), (3, 1));
    let task = &run.tasks[0];
    assert_eq!(task.brief, "Add the reset token model.");
    assert_eq!(task.acceptance, ["tokens expire after one hour"]);
    assert_eq!(task.route_spec.strength, None);
    assert_eq!(task.route_spec.runtime, Some(Runtime::Codex));
    assert_eq!(task.route_spec.effort, Some(Effort::Low));
    assert_eq!(task.history[0].at, 1_700_000_500);
    assert_eq!(task.history[1].text, "dispatched");
    let round = &task.rounds[0];
    assert_eq!(round.rate_limited_since, Some(1_700_000_120));
    assert_eq!(round.rate_limited_until, Some(1_700_000_420));
    assert_eq!(round.sent_back_at, [1_700_000_150, 1_700_000_250]);
}

/// Removes `key` from `obj`, asserting it was there: the test must strip what an
/// earlier sender would not have sent, not keys that never existed.
fn strip(obj: &mut serde_json::Value, key: &str) {
    let map = obj.as_object_mut().expect("an object");
    assert!(map.remove(key).is_some(), "{key} is a field");
}

#[test]
fn snapshot_without_view_fields_defaults() {
    // M8b's round-trip shape: the snapshot as it was, with none of the new keys. Its
    // `history` held strings, which a protocol-9 peer does not read (the bump covers
    // the changed element type), so it goes too; the key itself defaults.
    let mut json = serde_json::to_value(a_view_snapshot()).unwrap();
    strip(&mut json, "now");
    for run in json["runs"].as_array_mut().unwrap() {
        for key in [
            "approved_at",
            "plan_edits",
            "plan_edits_since_approval",
            "planners",
            "estimate_left_secs",
            "bound_ratio_permille",
        ] {
            strip(run, key);
        }
        for task in run["tasks"].as_array_mut().unwrap() {
            for key in ["brief", "acceptance", "route_spec", "history"] {
                strip(task, key);
            }
            for round in task["rounds"].as_array_mut().unwrap() {
                for key in ["rate_limited_since", "rate_limited_until", "sent_back_at"] {
                    strip(round, key);
                }
            }
        }
    }
    let snap: RunsSnapshot = serde_json::from_value(json).unwrap();
    assert_eq!(snap.now, 0);
    let run = &snap.runs[0];
    assert_eq!(run.approved_at, None);
    assert!(run.plan_edits.is_empty());
    assert_eq!(run.plan_edits_since_approval, 0);
    assert!(run.planners.is_empty());
    assert_eq!(run.estimate_left_secs, None);
    assert_eq!(run.bound_ratio_permille, None);
    let task = &run.tasks[0];
    assert_eq!(task.brief, "");
    assert!(task.acceptance.is_empty());
    assert_eq!(task.route_spec, RouteSpec::default());
    assert!(task.history.is_empty());
    let round = &task.rounds[0];
    assert_eq!(round.rate_limited_since, None);
    assert_eq!(round.rate_limited_until, None);
    assert!(round.sent_back_at.is_empty());
}

#[test]
fn planner_state_is_snake_case_and_defaults_to_planning() {
    assert_eq!(PlannerState::default(), PlannerState::Planning);
    assert_eq!(
        serde_json::to_string(&PlannerState::Finished).unwrap(),
        "\"finished\""
    );
}
