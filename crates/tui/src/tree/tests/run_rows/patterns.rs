//! Milestone 9.5 task 20: racers and test writers in the run view's tree (decision 29,
//! Interfaces "Run view"): their labels, their order and their splitting.

use super::*;
use crate::tree::run_fixtures::{pair_fixture, race_fixture};
use proto::RaceLane;

fn task_rounds(info: &RunInfo, windows: &[WindowInfo], id: &str) -> Vec<(NodeKey, String)> {
    let rows = run_rows(info, windows, &TreeState::default(), RunFilter::All);
    assert_unique(&rows);
    rows.iter()
        .filter_map(|row| match &row.kind {
            RowKind::AgentRound { task, round, .. } if task.id == id => Some((
                row.key.clone(),
                round_label_with_lane(
                    round.info.role,
                    round.info.lane,
                    round.info.session,
                    round.number,
                ),
            )),
            _ => None,
        })
        .collect()
}

fn key(task: &str, role: AgentRole, lane: Option<RaceLane>, session: u32, round: u32) -> NodeKey {
    NodeKey::AgentRound {
        run: "r1".into(),
        task: task.into(),
        role,
        lane,
        session,
        round,
    }
}

#[test]
fn race_rounds_are_ordered_and_labelled() {
    let (snapshot, windows) = race_fixture();
    assert_eq!(
        task_rounds(&snapshot.runs[0], &windows, "t2"),
        [
            (
                key("t2", AgentRole::Racer, Some(RaceLane::A), 1, 1),
                "racer a".to_owned()
            ),
            (
                key("t2", AgentRole::Racer, Some(RaceLane::B), 2, 1),
                "racer b".to_owned()
            ),
            (
                key("t2", AgentRole::Reviewer, Some(RaceLane::B), 2, 1),
                "review b#1".to_owned()
            ),
        ]
    );
}

/// Ties: test writer, worker, racer a, racer b, then reviewers, lane a's first.
#[test]
fn rounds_of_one_start_order_by_role_then_lane() {
    let (mut snapshot, windows) = race_fixture();
    let t2 = &mut snapshot.runs[0].tasks[0];
    for round in &mut t2.rounds {
        round.started_at = 500;
    }
    let mut review_a = reviewer(1, None, Runtime::Codex, 500);
    review_a.session = 1;
    review_a.lane = Some(RaceLane::A);
    t2.rounds.insert(0, review_a);
    t2.rounds.push(worker(3, None, Runtime::Claude, 500));
    let labels: Vec<String> = task_rounds(&snapshot.runs[0], &windows, "t2")
        .into_iter()
        .map(|(_, label)| label)
        .collect();
    assert_eq!(
        labels,
        [
            "worker #3",
            "racer a",
            "racer b",
            "review a#1",
            "review b#1"
        ]
    );
}

#[test]
fn pair_rounds_are_ordered_and_labelled() {
    let (snapshot, windows) = pair_fixture();
    assert_eq!(
        task_rounds(&snapshot.runs[0], &windows, "t3"),
        [
            (
                key("t3", AgentRole::TestWriter, None, 1, 1),
                "test writer #1".to_owned()
            ),
            (
                key("t3", AgentRole::Worker, None, 2, 1),
                "worker #2".to_owned()
            ),
        ]
    );
}

#[test]
fn a_test_writer_sorts_before_a_worker_on_a_tie() {
    let (mut snapshot, windows) = pair_fixture();
    for round in &mut snapshot.runs[0].tasks[0].rounds {
        round.started_at = 500;
    }
    let labels: Vec<String> = task_rounds(&snapshot.runs[0], &windows, "t3")
        .into_iter()
        .map(|(_, label)| label)
        .collect();
    assert_eq!(labels, ["test writer #1", "worker #2"]);
}

#[test]
fn a_racer_sent_back_splits_like_a_worker() {
    let (mut snapshot, windows) = race_fixture();
    let t2 = &mut snapshot.runs[0].tasks[0];
    let racer_a = t2
        .rounds
        .iter_mut()
        .find(|round| round.lane == Some(RaceLane::A) && round.role == AgentRole::Racer)
        .expect("racer a");
    racer_a.sent_back_at = vec![racer_a.started_at + 60];
    let rounds = task_rounds(&snapshot.runs[0], &windows, "t2");
    assert_eq!(
        rounds[..2],
        [
            (
                key("t2", AgentRole::Racer, Some(RaceLane::A), 1, 1),
                "racer a".to_owned()
            ),
            (
                key("t2", AgentRole::Racer, Some(RaceLane::B), 2, 1),
                "racer b".to_owned()
            ),
        ]
    );
    assert!(rounds.contains(&(
        key("t2", AgentRole::Racer, Some(RaceLane::A), 1, 2),
        "racer a r2".to_owned()
    )));
    let shown = display_rounds(&snapshot.runs[0].tasks[0], &windows);
    let pieces: Vec<(u32, bool)> = shown
        .iter()
        .filter(|round| round.info.lane == Some(RaceLane::A) && round.info.role == AgentRole::Racer)
        .map(|round| (round.number, round.last))
        .collect();
    assert_eq!(pieces, [(1, false), (2, true)]);
}

#[test]
fn a_test_writer_sent_back_splits_like_a_worker() {
    let (mut snapshot, windows) = pair_fixture();
    let writer = snapshot.runs[0].tasks[0]
        .rounds
        .iter_mut()
        .find(|round| round.role == AgentRole::TestWriter)
        .expect("the test writer");
    writer.sent_back_at = vec![writer.started_at + 30];
    let labels: Vec<String> = task_rounds(&snapshot.runs[0], &windows, "t3")
        .into_iter()
        .map(|(_, label)| label)
        .collect();
    assert_eq!(labels, ["test writer #1", "test writer #1 r2", "worker #2"]);
}

/// Interfaces "Run view": the lane is part of a racer's and a lane reviewer's label;
/// `round_label` is the label with no lane.
#[test]
fn round_labels_with_and_without_a_lane() {
    let a = Some(RaceLane::A);
    let b = Some(RaceLane::B);
    assert_eq!(round_label_with_lane(AgentRole::Racer, a, 1, 1), "racer a");
    assert_eq!(
        round_label_with_lane(AgentRole::Racer, a, 1, 2),
        "racer a r2"
    );
    assert_eq!(
        round_label_with_lane(AgentRole::Reviewer, b, 2, 1),
        "review b#1"
    );
    assert_eq!(
        round_label_with_lane(AgentRole::Reviewer, None, 2, 1),
        "review #1"
    );
    assert_eq!(
        round_label_with_lane(AgentRole::TestWriter, None, 1, 2),
        "test writer #1 r2"
    );
    assert_eq!(
        round_label_with_lane(AgentRole::Worker, a, 2, 1),
        "worker #2"
    );
    assert_eq!(round_label(AgentRole::Racer, 1, 1), "racer");
}

/// Ruling T20-1 (c): the daemon numbers each lane's reviewers from 1, so both lanes'
/// first reviewers are `Reviewer` session 1, round 1. The lane in the key keeps them
/// two nodes.
#[test]
fn two_lanes_first_reviewers_are_two_nodes() {
    let (snapshot, windows) = crate::tree::run_fixtures::lane_reviews_fixture();
    let (a, b) = (Some(RaceLane::A), Some(RaceLane::B));
    assert_eq!(
        task_rounds(&snapshot.runs[0], &windows, "t2"),
        [
            (key("t2", AgentRole::Racer, a, 1, 1), "racer a".to_owned()),
            (key("t2", AgentRole::Racer, b, 2, 1), "racer b".to_owned()),
            (
                key("t2", AgentRole::Reviewer, b, 1, 1),
                "review b#1".to_owned()
            ),
            (
                key("t2", AgentRole::Reviewer, a, 1, 1),
                "review a#1".to_owned()
            ),
        ]
    );
}

/// Two racers with one session number (ruling T20-1 (c)) are two nodes too.
#[test]
fn two_racers_with_one_session_number_are_two_nodes() {
    let (mut snapshot, windows) = race_fixture();
    for round in &mut snapshot.runs[0].tasks[0].rounds {
        if round.role == AgentRole::Racer {
            round.session = 1;
            round.round = 1;
        }
    }
    let (a, b) = (Some(RaceLane::A), Some(RaceLane::B));
    let rounds = task_rounds(&snapshot.runs[0], &windows, "t2");
    assert_eq!(
        rounds[..2],
        [
            (key("t2", AgentRole::Racer, a, 1, 1), "racer a".to_owned()),
            (key("t2", AgentRole::Racer, b, 1, 1), "racer b".to_owned()),
        ]
    );
    assert_eq!(rounds.len(), 3);
}
