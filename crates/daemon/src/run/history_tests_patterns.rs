//! Milestone 9.5 task M9.5.21 (decision 30): what a task's history record says of its
//! race, its pair and its round, and that the refit leaves out the race records it
//! wrote.

use proto::{
    AgentRole, HistoryLine, LaneState, PairPhase, RaceLane, Runtime, TaskOrigin, TaskOutcome,
    TaskPattern, TaskState,
};

use super::super::task_record;
use super::fixtures::*;
use crate::run::model::{Pair, Task};
use crate::run::refit::{SizeClass, budget_samples, threshold_samples};
use crate::run::test_support::race_of;

fn pair(phase: PairPhase, writer_failures: u8) -> Pair {
    Pair {
        phase,
        writer_route: route(Runtime::Codex, "", proto::Effort::MEDIUM),
        test: Some("t_feat".into()),
        red: Some("a".repeat(40)),
        red_checked: Some(phase == PairPhase::Implementing),
        writer_failures,
        writer_sessions: 1,
        escalated_from: None,
        writer_signals: Vec::new(),
        writer_signals_more: 0,
        signals_read: false,
    }
}

fn merged(task: &mut Task) {
    task.state = TaskState::Merged;
    task.session = 2;
}

#[test]
fn task_record_marks_the_pattern_and_round() {
    let mut run = run_of(&["t1", "t2", "t3", "t4", "t5", "t6"]);
    // t1: lane b (Codex) won, its crown not back yet; the task's route is still lane a's.
    let raced = &mut run.tasks[0];
    merged(raced);
    raced.race = Some(race_of(raced, [LaneState::Lost, LaneState::Won]));
    raced.race.as_mut().unwrap().crowned = false;
    let winner_route = raced.race.as_ref().unwrap().lanes[1].route.clone();
    assert_ne!(raced.route, winner_route, "the fixture's lanes differ");
    // t2: lane a went out, lane b was adopted.
    let adopted = &mut run.tasks[1];
    adopted.state = TaskState::Blocked;
    adopted.race = Some(race_of(adopted, [LaneState::Out, LaneState::Adopted]));
    // t3: paired, implemented after two failed red checks.
    let paired = &mut run.tasks[2];
    merged(paired);
    paired.failures = 1;
    paired.pair = Some(pair(PairPhase::Implementing, 2));
    // t4: paired, still writing its test after one failure.
    let writing = &mut run.tasks[3];
    writing.state = TaskState::Blocked;
    writing.failures = 1;
    writing.pair = Some(pair(PairPhase::Writing, 0));
    // t5: added by round 2; t6 a plain round-1 task.
    run.tasks[4].round = 2;
    merged(&mut run.tasks[4]);
    merged(&mut run.tasks[5]);

    let record = |i: usize| {
        let task = run.tasks[i].clone();
        let outcome = super::super::outcome(&task);
        task_record(&run, &task, outcome, 1_000)
    };

    let r = record(0);
    assert_eq!(r.pattern, Some(TaskPattern::Race));
    assert_eq!(r.race_winner, Some(RaceLane::B));
    assert!(!r.race_adopted);
    assert_eq!(
        r.route, winner_route,
        "before the crown, a raced task records the winner's route"
    );
    assert_eq!(r.writer_failures, 0);
    // HISTORY_VERSION stays 5: the line it writes reads back unchanged.
    let line = serde_json::to_string(&HistoryLine::Task(r.clone())).unwrap();
    let back: HistoryLine = serde_json::from_str(&line).unwrap();
    assert_eq!(back, HistoryLine::Task(r));

    let r = record(1);
    assert_eq!(r.pattern, Some(TaskPattern::Race));
    assert_eq!(r.race_winner, Some(RaceLane::B));
    assert!(r.race_adopted);

    let r = record(2);
    assert_eq!(r.pattern, Some(TaskPattern::Pair));
    assert_eq!((r.writer_failures, r.failures), (2, 1));
    assert_eq!((r.race_winner, r.race_adopted), (None, false));

    let r = record(3);
    assert_eq!(r.pattern, Some(TaskPattern::Pair));
    assert_eq!(
        r.writer_failures, 1,
        "while writing, the task's failures are the test writer's"
    );

    let r = record(4);
    assert_eq!((r.pattern, r.round), (None, 2));
    let r = record(5);
    assert_eq!((r.pattern, r.round), (None, 1));
    assert_eq!(r.route, run.tasks[5].route);
}

/// Fix round 1 (review m1): once crowned, the task is the lane's, and its route is the
/// task's own, a later escalation (an adopted lane's rung 2) included.
#[test]
fn a_crowned_race_records_the_tasks_route() {
    let mut run = run_of(&["t1"]);
    let raced = &mut run.tasks[0];
    raced.race = Some(race_of(raced, [LaneState::Out, LaneState::Adopted]));
    assert!(raced.race.as_ref().unwrap().crowned);
    raced.route = route(Runtime::Codex, "gpt-6.1-sol", proto::Effort::HIGH);
    let lane = raced.race.as_ref().unwrap().lanes[1].route.clone();
    assert_ne!(raced.route, lane, "the escalation moved the task's route");
    let r = task_record(&run, &run.tasks[0], TaskOutcome::Blocked, 1_000);
    assert_eq!(r.route, run.tasks[0].route);
}

/// Ruling RH-1 reads a record's origin: an engine-made fix task's record says so.
#[test]
fn task_record_keeps_the_tasks_origin() {
    let mut run = run_of(&["t1", "t2"]);
    run.tasks[1].origin = TaskOrigin::Bisect;
    let plan = task_record(&run, &run.tasks[0], TaskOutcome::Merged, 1_000);
    let fix = task_record(&run, &run.tasks[1], TaskOutcome::Merged, 1_000);
    assert_eq!(plan.origin, TaskOrigin::Plan);
    assert_eq!(fix.origin, TaskOrigin::Bisect);
}

/// Decision 4: a race's `tool_calls` add both lanes, and a pair's add its test writer:
/// the record counts every session that wrote the task.
#[test]
fn task_record_counts_every_writing_session() {
    let mut run = run_of(&["t1", "t2"]);
    let raced = &mut run.tasks[0];
    raced.race = Some(race_of(raced, [LaneState::Won, LaneState::Lost]));
    let mut a = round(AgentRole::Racer, 1, 5, usage(10));
    a.lane = Some(RaceLane::A);
    let mut b = round(AgentRole::Racer, 2, 7, usage(20));
    b.lane = Some(RaceLane::B);
    let mut review = round(AgentRole::Reviewer, 1, 3, usage(100));
    review.lane = Some(RaceLane::A);
    raced.rounds = vec![a, b, review];
    let paired = &mut run.tasks[1];
    paired.pair = Some(pair(PairPhase::Implementing, 0));
    paired.rounds = vec![
        round(AgentRole::TestWriter, 1, 4, usage(1)),
        round(AgentRole::Worker, 2, 6, usage(2)),
    ];

    let r = task_record(&run, &run.tasks[0], TaskOutcome::Merged, 1_000);
    assert_eq!(r.tool_calls, 12);
    assert_eq!(r.worker_usage.input, 30);
    assert_eq!(r.reviewer_usage.input, 100);
    let r = task_record(&run, &run.tasks[1], TaskOutcome::Merged, 1_000);
    assert_eq!(r.tool_calls, 10);
    assert_eq!(r.worker_usage.input, 3);
}

#[test]
fn refit_excludes_race_records_it_wrote() {
    let mut run = run_of(&["t1", "t2"]);
    for task in &mut run.tasks {
        merged(task);
        task.diff = Some(proto::DiffStats {
            files: 1,
            hunks: 1,
            added: 10,
            removed: 0,
        });
    }
    for i in 0..run.tasks.len() {
        crate::run::routing::record_worker(&mut run, i, 10);
    }
    let raced = &mut run.tasks[0];
    raced.race = Some(race_of(raced, [LaneState::Won, LaneState::Lost]));
    let lines: Vec<HistoryLine> = run
        .tasks
        .iter()
        .map(|t| HistoryLine::Task(task_record(&run, t, TaskOutcome::Merged, 1_000)))
        .collect();
    let t = config::Tuning::default();
    let ids = |records: Vec<&proto::TaskRecord>| -> Vec<String> {
        records.iter().map(|r| r.task_id.clone()).collect()
    };
    let single = vec!["t2".to_string()];
    assert_eq!(ids(budget_samples(&lines, SizeClass::S, &t)), single);
    assert_eq!(ids(threshold_samples(&lines, SizeClass::S, &t)), single);
}
