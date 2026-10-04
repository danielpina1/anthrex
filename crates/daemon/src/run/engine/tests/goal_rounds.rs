//! Milestone 9.3 task 3: the round model (decision 18). A run from before rounds loads
//! as one round rebuilt from its goal, a started run has round 1, and the snapshot's
//! round heads are cleaned and cut.

use proto::{ROUND_HEAD_CHARS, RoundOrigin, RoundOutcome, RunState};

use super::control_restore::restart;
use super::fixture::*;
use crate::run::engine::OpResult;
use crate::run::model::{Round, Run};
use crate::run::snapshot::snapshot;

/// Round 1 as decision 18 builds it for `run`: its goal, `user`, from its creation, in
/// stage 1, not ended.
fn assert_one_round(run: &Run) {
    assert_eq!(run.rounds.len(), 1, "{:#?}", run.rounds);
    let first = &run.rounds[0];
    assert_eq!(first.n, 1);
    assert_eq!(first.goal, run.goal);
    assert_eq!(first.origin, RoundOrigin::User);
    assert_eq!(first.started_at, run.created_at);
    assert_eq!(first.ended_at, None);
    assert_eq!(first.outcome, None);
    assert_eq!(first.summary, None);
    assert_eq!(first.first_stage, 1);
    assert_eq!(run.round(), 1);
    assert_eq!(run.current_round(), Some(first));
    assert!(run.tasks.iter().all(|t| t.round == 1), "{:?}", run.tasks);
    assert!(run.stages.iter().all(|s| s.round == 1), "{:?}", run.stages);
}

/// A `run.json` from before rounds, the one milestone 9.2 checked in (written by 9.1's
/// code), goes through `restore`: it has one round, rebuilt from `Run.goal`, and the
/// snapshot shows `round: 1` and no `rounds`.
#[test]
fn a_pre_9_3_run_json_loads_as_one_round() {
    let text = include_str!("../../delivery/m9_1_run.json");
    let stored: serde_json::Value = serde_json::from_str(text).unwrap();
    assert!(
        stored.get("rounds").is_none(),
        "the fixture predates rounds"
    );
    let run: Run = serde_json::from_str(text).expect("a pre-9.3 run.json loads");
    assert!(run.rounds.is_empty(), "absent rounds load empty");
    assert_eq!(run.round(), 1, "a run with no round record is in round 1");
    assert_eq!(run.current_round(), None);
    assert!(!run.tasks.is_empty() && !run.stages.is_empty());
    assert!(run.tasks.iter().all(|t| t.round == 1));
    assert!(run.stages.iter().all(|s| s.round == 1));
    assert_eq!(run.orch.request_wake, None);

    let mut fx = Fixture::new(PROFILE);
    fx.state.runs.insert(run.id.clone(), run);
    restart(&mut fx, Vec::new());
    let run = fx.run();
    assert_eq!(run.goal, "Engine test");
    assert_one_round(run);

    let info = snapshot(&fx.state, fx.now).runs.remove(0);
    assert_eq!(info.round, 1);
    assert!(info.rounds.is_empty(), "{:?}", info.rounds);
    let json = serde_json::to_value(&info).unwrap();
    assert_eq!(json["round"], 1);
    assert!(json.get("rounds").is_none(), "{json}");
    for task in json["tasks"].as_array().unwrap() {
        assert_eq!(task["round"], 1, "{task}");
    }
    let stages = json["stages"].as_array().unwrap();
    assert!(!stages.is_empty());
    for stage in stages {
        assert_eq!(stage["round"], 1, "{stage}");
    }
}

/// The older checked-in `run.json`s (milestones 8b and 9) load as one round too.
#[test]
fn older_run_jsons_load_as_one_round_too() {
    for text in [include_str!("m8b_run.json"), include_str!("m9_run.json")] {
        let run: Run = serde_json::from_str(text).expect("an old run.json loads");
        assert!(run.rounds.is_empty());
        let mut fx = Fixture::new(PROFILE);
        fx.state.runs.insert(run.id.clone(), run);
        restart(&mut fx, Vec::new());
        assert_one_round(fx.run());
    }
}

/// A plan-file run and a goal run, each started through `requests::start`, have round 1.
#[test]
fn a_started_run_has_round_one() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "auth", "")]));
    fx.start(false);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    assert_one_round(fx.run());

    let fx = super::orch::planned(false);
    assert_eq!(fx.run().state, RunState::Planning);
    assert!(fx.run().orch.orchestrator.is_some());
    assert_one_round(fx.run());
}

/// A round with `goal` (and `summary`), for the head tests.
fn round(n: u32, goal: &str, summary: Option<&str>) -> Round {
    Round {
        n,
        goal: goal.into(),
        origin: RoundOrigin::Orchestrator,
        started_at: 5,
        ended_at: Some(6),
        outcome: Some(RoundOutcome::Completed),
        summary: summary.map(str::to_string),
        first_stage: 1,
        windows_before: 0,
        scouts_before: 0,
        approved_at: None,
        paused_before: 0,
    }
}

/// KG §5: `goal_head` and `summary_head` are one line, without hidden format
/// characters, cut to 200 characters; a one-round run lists no rounds.
#[test]
fn round_infos_are_cleaned_and_cut() {
    let long = format!("ab\u{202E}c\nd{}", "e".repeat(294));
    assert_eq!(long.chars().count(), 300);
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "auth", "")]));
    fx.start(false);
    assert!(fx.run().round_infos().is_empty(), "one round lists none");

    let run = fx.run_mut();
    run.rounds = vec![round(1, &long, Some(&long)), round(2, "more", None)];
    let infos = run.round_infos();
    assert_eq!(infos.len(), 2);
    let head = &infos[0].goal_head;
    assert_eq!(head.chars().count(), ROUND_HEAD_CHARS);
    assert!(head.starts_with("abc d"), "{head:?}");
    assert!(
        !head.contains('\u{202E}') && !head.contains('\n'),
        "{head:?}"
    );
    assert_eq!(infos[0].summary_head.as_deref(), Some(head.as_str()));
    assert_eq!(infos[0].n, 1);
    assert_eq!(infos[0].origin, RoundOrigin::Orchestrator);
    assert_eq!(infos[0].outcome, Some(RoundOutcome::Completed));
    assert_eq!(infos[1].goal_head, "more");
    assert_eq!(infos[1].summary_head, None);

    let info = snapshot(&fx.state, fx.now).runs.remove(0);
    assert_eq!(info.round, 2);
    assert_eq!(info.rounds, infos);
}

/// Final fix wave C-m3: a round is `ended` in the snapshot once its `ended_at` is set,
/// not when its outcome is: a cancelled round keeps running until its sessions end
/// (decision 16), and `goal_rounds_end::open_round` still reads it open.
#[test]
fn a_round_is_ended_once_its_end_is_recorded() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "auth", "")]));
    fx.start(false);
    let run = fx.run_mut();
    let mut cancelling = round(2, "more", None);
    (cancelling.ended_at, cancelling.outcome) = (None, Some(RoundOutcome::Cancelled));
    run.rounds = vec![round(1, "first", None), cancelling];
    let info = snapshot(&fx.state, fx.now).runs.remove(0);
    let ended: Vec<_> = info
        .rounds
        .iter()
        .map(|r| (r.n, r.outcome, r.ended))
        .collect();
    assert_eq!(
        ended,
        vec![
            (1, Some(RoundOutcome::Completed), true),
            (2, Some(RoundOutcome::Cancelled), false)
        ]
    );
    fx.run_mut().rounds[1].ended_at = Some(7);
    let info = snapshot(&fx.state, fx.now).runs.remove(0);
    assert!(info.rounds[1].ended);
}

/// A stage's round in the snapshot: its created record's, or, before it is created, its
/// tasks'. Task 3 review m1: the placeholder stage 1 of a plan not yet approved is not
/// stage 1's record, so its round is not the stage's.
#[test]
fn a_stage_has_its_records_round_or_its_tasks() {
    let tasks = [task("t1", "S", "a", ""), task("t2", "S", "b", "stage = 2")];
    let mut fx = Fixture::new(&plan_with(PROFILE, &tasks));
    fx.ready(false);
    fx.task_mut("t2").round = 2;
    fx.run_mut().stages[0].round = 3;
    assert_eq!(fx.run().stage(2), None, "stage 2 is not created yet");
    let rounds = |fx: &Fixture| -> Vec<(u16, u32)> {
        let info = snapshot(&fx.state, fx.now).runs.remove(0);
        info.stages.iter().map(|s| (s.n, s.round)).collect()
    };
    assert_eq!(
        rounds(&fx),
        [(1, 1), (2, 2)],
        "the placeholder's round is not used"
    );
    // Approved, stage 1 is created: its record's round is the stage's.
    fx.approve();
    let (op, _) = fx.op("CreateStageBranch");
    fx.done(op, OpResult::StageCreated);
    assert!(fx.run().stage(1).is_some());
    fx.run_mut().stages[0].round = 3;
    assert_eq!(rounds(&fx), [(1, 3), (2, 2)]);
}
