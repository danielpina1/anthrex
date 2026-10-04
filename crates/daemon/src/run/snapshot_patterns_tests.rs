//! Task 20b: the snapshot's race, pair and lane fields, from the daemon's model, for a
//! racing task in each lane state and a paired task in each phase. Pure.

use proto::{AgentRole, LaneState, PairPhase, RaceLane, Runtime, TokenUsage, Verdict};

use super::*;
use crate::run::engine::EngineState;
use crate::run::model::{Pair, Run};
use crate::run::orch::test_support::{review, round, run_of, task_mut};
use crate::run::test_support::race_of;

/// Run `r1` with S tasks `t0` and `t1`; `t0` races with its lanes in `states`.
fn racing(states: [LaneState; 2]) -> Run {
    let mut run = run_of(2);
    run.id = "r1".into();
    let race = race_of(&run.tasks[0], states);
    run.tasks[0].race = Some(race);
    run
}

fn pair(phase: PairPhase) -> Pair {
    let implementing = phase == PairPhase::Implementing;
    let mut writer_route = crate::run::orch::test_support::route();
    writer_route.runtime = Runtime::Codex;
    Pair {
        phase,
        writer_route,
        test: implementing.then(|| "t1::works".into()),
        red: implementing.then(|| "a1b2c3d4e5f6".into()),
        red_checked: implementing.then_some(true),
        writer_failures: u8::from(implementing),
        writer_sessions: 1,
        escalated_from: None,
        writer_signals: Vec::new(),
        writer_signals_more: 0,
        signals_read: false,
    }
}

#[test]
fn a_running_race_shows_both_lanes_and_no_winner() {
    for states in [
        [LaneState::Preparing, LaneState::Working],
        [LaneState::Proof, LaneState::Check],
        [LaneState::Review, LaneState::Working],
    ] {
        let run = racing(states);
        let task = &run.tasks[0];
        let info = race_info(task).expect("a race");
        assert_eq!((info.winner, info.adopted), (None, false), "{states:?}");
        let lanes: Vec<_> = (info.lanes.iter())
            .map(|l| (l.lane, l.state, l.checkout.as_str(), l.route.runtime))
            .collect();
        assert_eq!(
            lanes,
            [
                (RaceLane::A, states[0], "t0.a", task.route.runtime),
                (RaceLane::B, states[1], "t0.b", Runtime::Codex),
            ]
        );
        assert!(
            info.lanes
                .iter()
                .all(|l| !l.kept && l.salvage_ref.is_none())
        );
    }
}

#[test]
fn a_won_and_a_crowned_race_name_the_winner() {
    let mut run = racing([LaneState::Working, LaneState::Won]);
    let info = race_info(&run.tasks[0]).unwrap();
    assert_eq!((info.winner, info.adopted), (Some(RaceLane::B), false));
    // Crowned: the winner is the task; its lane's own fields stay as the crown left them.
    let race = run.tasks[0].race.as_mut().unwrap();
    race.crowned = true;
    race.lanes[0].state = LaneState::Lost;
    race.lanes[1].head = Some("feedbeef".into());
    let info = race_info(&run.tasks[0]).unwrap();
    assert_eq!(info.winner, Some(RaceLane::B));
    assert_eq!(info.lanes[0].state, LaneState::Lost);
    assert_eq!(info.lanes[1].state, LaneState::Won);
    assert_eq!(info.lanes[1].head.as_deref(), Some("feedbeef"));
}

#[test]
fn a_lost_and_an_out_lane_show_their_reason_salvage_and_kept() {
    let mut run = racing([LaneState::Won, LaneState::Lost]);
    let race = run.tasks[0].race.as_mut().unwrap();
    race.lanes[1].salvage_ref = Some("refs/anthrex/salvage/r1/t0/1".into());
    race.lanes[1].kept = true;
    let info = race_info(&run.tasks[0]).unwrap();
    let lost = &info.lanes[1];
    assert_eq!(lost.state, LaneState::Lost);
    assert_eq!(
        lost.salvage_ref.as_deref(),
        Some("refs/anthrex/salvage/r1/t0/1")
    );
    assert!(lost.kept);
    assert!(!info.lanes[0].kept);

    let mut run = racing([LaneState::Out, LaneState::Working]);
    let race = run.tasks[0].race.as_mut().unwrap();
    race.lanes[0].reason = Some("the check failed twice".into());
    let info = race_info(&run.tasks[0]).unwrap();
    assert_eq!(info.lanes[0].state, LaneState::Out);
    assert_eq!(
        info.lanes[0].reason.as_deref(),
        Some("the check failed twice")
    );
    assert_eq!((info.winner, info.adopted), (None, false));
}

#[test]
fn an_adopted_lane_is_the_winner_and_says_so() {
    let run = racing([LaneState::Out, LaneState::Adopted]);
    let info = race_info(&run.tasks[0]).unwrap();
    assert_eq!((info.winner, info.adopted), (Some(RaceLane::B), true));
    assert_eq!(info.lanes[1].state, LaneState::Adopted);
}

#[test]
fn a_paired_task_shows_its_phase() {
    let mut run = run_of(2);
    assert_eq!(pair_info(&run.tasks[1]), None);
    run.tasks[1].pair = Some(pair(PairPhase::Writing));
    let writing = pair_info(&run.tasks[1]).expect("a pair");
    assert_eq!(writing.phase, PairPhase::Writing);
    assert_eq!(writing.writer_route.runtime, Runtime::Codex);
    assert_eq!(
        (writing.test, writing.red, writing.red_checked),
        (None, None, None)
    );
    assert_eq!(writing.writer_failures, 0);
    // Whole-branch review D, M-1: before the red check the test is the spec's
    // `test_to_write`, so the run view reads `writing <test_to_write>`.
    run.tasks[1].spec.test_to_write = Some("t1::expires".into());
    let named = pair_info(&run.tasks[1]).unwrap();
    assert_eq!(
        (named.test.as_deref(), named.red),
        (Some("t1::expires"), None)
    );

    run.tasks[1].pair = Some(pair(PairPhase::Implementing));
    let implementing = pair_info(&run.tasks[1]).unwrap();
    assert_eq!(implementing.phase, PairPhase::Implementing);
    assert_eq!(implementing.test.as_deref(), Some("t1::works"));
    assert_eq!(implementing.red.as_deref(), Some("a1b2c3d4e5f6"));
    assert_eq!(implementing.red_checked, Some(true));
    assert_eq!(implementing.writer_failures, 1);
    assert_eq!(race_info(&run.tasks[1]), None);
}

#[test]
fn only_racer_and_reviewer_rounds_carry_a_lane() {
    let mut racer = round(1, 0, TokenUsage::default());
    racer.role = AgentRole::Racer;
    racer.lane = Some(RaceLane::A);
    assert_eq!(round_lane(&racer), Some(RaceLane::A));
    let mut reviewer = racer.clone();
    reviewer.role = AgentRole::Reviewer;
    reviewer.lane = Some(RaceLane::B);
    assert_eq!(round_lane(&reviewer), Some(RaceLane::B));
    let worker = round(2, 0, TokenUsage::default());
    assert_eq!(round_lane(&worker), None);
    let mut writer = racer.clone();
    writer.role = AgentRole::TestWriter;
    assert_eq!(round_lane(&writer), None, "a test writer has no lane");
    let mut record = review(1, Some(Verdict::Approve), &[]);
    assert_eq!(review_lane(&record), None);
    record.lane = Some(RaceLane::B);
    assert_eq!(review_lane(&record), Some(RaceLane::B));
}

/// The snapshot fills the fields: both lanes' first reviewers are round 1, told apart
/// by their lane.
#[test]
fn the_snapshot_carries_race_pair_and_lanes() {
    let mut run = racing([LaneState::Review, LaneState::Review]);
    let task = task_mut(&mut run, "t0");
    for (session, lane) in [(1, RaceLane::A), (2, RaceLane::B)] {
        let mut racer = round(session, 0, TokenUsage::default());
        racer.role = AgentRole::Racer;
        racer.lane = Some(lane);
        task.rounds.push(racer);
    }
    for lane in [RaceLane::A, RaceLane::B] {
        let mut reviewer = round(1, 0, TokenUsage::default());
        reviewer.role = AgentRole::Reviewer;
        reviewer.lane = Some(lane);
        task.rounds.push(reviewer);
        let mut record = review(1, None, &[]);
        record.lane = Some(lane);
        task.reviews.push(record);
    }
    task_mut(&mut run, "t1").pair = Some(pair(PairPhase::Writing));
    let mut state = EngineState::default();
    state.runs.insert(run.id.clone(), run);
    let snap = crate::run::snapshot::snapshot(&state, 5_000);
    let tasks = &snap.runs[0].tasks;
    let t0 = &tasks[0];
    let race = t0.race.as_ref().expect("t0's race");
    assert_eq!(race.lanes.len(), 2);
    let lanes: Vec<_> = (t0.rounds.iter())
        .map(|r| (r.role, r.session, r.lane))
        .collect();
    assert_eq!(
        lanes,
        [
            (AgentRole::Racer, 1, Some(RaceLane::A)),
            (AgentRole::Racer, 2, Some(RaceLane::B)),
            (AgentRole::Reviewer, 1, Some(RaceLane::A)),
            (AgentRole::Reviewer, 1, Some(RaceLane::B)),
        ]
    );
    let reviews: Vec<_> = t0.reviews.iter().map(|r| (r.round, r.lane)).collect();
    assert_eq!(reviews, [(1, Some(RaceLane::A)), (1, Some(RaceLane::B))]);
    let t1 = &tasks[1];
    assert_eq!(t1.pair.as_ref().map(|p| p.phase), Some(PairPhase::Writing));
    assert_eq!(t1.race, None);
}

/// A failed check of task `t0`'s run at `at`, made in `lane`.
fn check(at: u64, lane: Option<RaceLane>) -> crate::run::model::CheckRecord {
    crate::run::model::CheckRecord {
        at,
        ok: false,
        code: Some(1),
        timed_out: false,
        tail: format!("failed at {at}"),
        secs: 1,
        on_candidate: false,
        summary: None,
        summary_source: None,
        tier: None,
        lane,
    }
}

/// Task 20b's carry: the snapshot's `last_check` (and `last_proof`, read the same way)
/// names its lane before the race has a winner; once it has, it is the winner's, and
/// once the race is over with no winner, the task's own.
#[test]
fn the_last_check_names_its_lane_then_is_the_winners() {
    let last = |run: &Run| {
        let mut state = EngineState::default();
        state.runs.insert(run.id.clone(), run.clone());
        let snap = crate::run::snapshot::snapshot(&state, 5_000);
        let check = snap.runs[0].tasks[0].last_check.clone();
        check.map(|c| (c.at, c.lane))
    };
    let mut run = racing([LaneState::Working, LaneState::Working]);
    let t0 = task_mut(&mut run, "t0");
    t0.checks = vec![check(10, Some(RaceLane::A)), check(20, Some(RaceLane::B))];
    assert_eq!(last(&run), Some((20, Some(RaceLane::B))));
    let won = |run: &mut Run, lane| {
        let race = task_mut(run, "t0").race.as_mut().unwrap();
        race.winner = Some(lane);
    };
    won(&mut run, RaceLane::A);
    assert_eq!(last(&run), Some((10, Some(RaceLane::A))), "the winner's");
    task_mut(&mut run, "t0").checks.push(check(30, None));
    assert_eq!(last(&run), Some((30, None)), "the crowned task's own");

    // Both lanes out, no winner, then the task retried single.
    let mut over = racing([LaneState::Out, LaneState::Out]);
    let t0 = task_mut(&mut over, "t0");
    t0.checks = vec![check(10, None), check(20, Some(RaceLane::B))];
    assert_eq!(last(&over), Some((10, None)));
}
