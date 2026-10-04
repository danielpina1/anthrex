//! Task M9.5.20's review fix round (ruling T20-2, folded into task 20b): a lane
//! reviewer's own verdict and findings (I1), a racer's `fixing` from its own lane, a
//! reviewer with no lane judging a racer (m2), the `agents` row's runtimes routed only
//! by a lane or a test writer (m3), the pair row's implementer mark (m4) and a racing
//! task's review count (m5).

use super::run_patterns::pair_row;
use super::run_task_outcome::review_row;
use super::run_tests::{app_of, inspect_node, value};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{lane_reviews_fixture, pair_fixture, race_fixture};
use proto::{AgentRole, LaneState, RaceLane, ReviewInfo, TaskInfo, TaskState};

fn round_key(role: AgentRole, lane: Option<RaceLane>, session: u32, round: u32) -> NodeKey {
    NodeKey::AgentRound {
        run: "r1".into(),
        task: "t2".into(),
        role,
        lane,
        session,
        round,
    }
}

fn lane_state(task: &mut TaskInfo, lane: RaceLane, state: LaneState) {
    let race = task.race.as_mut().expect("a race");
    let info = race.lanes.iter_mut().find(|l| l.lane == lane).unwrap();
    info.state = state;
}

/// I1: both lanes' first reviewers are review round 1; each shows its own lane's
/// verdict and findings.
#[test]
fn each_lane_reviewer_shows_its_own_verdict_and_findings() {
    let app = app_of(lane_reviews_fixture());
    let a = inspect_node(
        &app,
        &round_key(AgentRole::Reviewer, Some(RaceLane::A), 1, 1),
    );
    assert_eq!(value(&a, "verdict"), Some("reviewing"));
    assert_eq!(value(&a, "findings"), Some("none"));
    let b = inspect_node(
        &app,
        &round_key(AgentRole::Reviewer, Some(RaceLane::B), 1, 1),
    );
    assert_eq!(value(&b, "verdict"), Some("changes (blocking)"));
    let findings = value(&b, "findings").expect("findings");
    assert!(
        findings.contains("lane b's reset skips the expiry check"),
        "{findings}"
    );
}

/// Beyond T20-2's two sites, the same lookup: a racer sent back after its own lane's
/// review is `fixing` that review's finding, never the other lane's.
#[test]
fn a_racer_is_fixing_its_own_lanes_review_only() {
    let (mut snapshot, windows) = lane_reviews_fixture();
    let t2 = &mut snapshot.runs[0].tasks[0];
    let now = snapshot.now;
    for round in &mut t2.rounds {
        if round.role == AgentRole::Racer {
            round.ended_at = None;
            round.sent_back_at = vec![now - 30];
        }
    }
    let app = app_of((snapshot, windows));
    let b = inspect_node(&app, &round_key(AgentRole::Racer, Some(RaceLane::B), 2, 2));
    let fixing = value(&b, "fixing").expect("lane b is fixing its review");
    assert!(
        fixing.contains("lane b's reset skips the expiry check"),
        "{fixing}"
    );
    let a = inspect_node(&app, &round_key(AgentRole::Racer, Some(RaceLane::A), 1, 2));
    assert_eq!(value(&a, "fixing"), None, "lane b's review is not lane a's");
}

/// m2: a reviewer with no lane (the crowned task's) judges the latest racer before it.
#[test]
fn a_reviewer_with_no_lane_judges_the_latest_racer() {
    let (mut snapshot, windows) = race_fixture();
    let t2 = &mut snapshot.runs[0].tasks[0];
    let now = snapshot.now;
    for round in &mut t2.rounds {
        match (round.role, round.lane) {
            (AgentRole::Racer, Some(RaceLane::A)) => round.started_at = now - 200,
            (AgentRole::Reviewer, _) => {
                round.lane = None;
                round.started_at = now - 100;
            }
            _ => {}
        }
    }
    t2.reviews[0].lane = None;
    let app = app_of((snapshot, windows));
    let reviewer = inspect_node(&app, &round_key(AgentRole::Reviewer, None, 2, 1));
    assert_eq!(
        value(&reviewer, "judging"),
        Some("racer a · claude · standard")
    );
}

/// m3: a runtime routed only by a race lane, or only by a test writer, is listed.
#[test]
fn a_runtime_routed_only_by_a_lane_or_a_test_writer_is_listed() {
    let (mut snapshot, windows) = race_fixture();
    snapshot.runs[0].writer_caps.clear();
    let app = app_of((snapshot, windows));
    let run = inspect_node(&app, &NodeKey::Run("r1".into()));
    assert_eq!(
        value(&run, "agents"),
        Some("workers 2/3 · readers 1/3 · claude ok · codex ok")
    );

    let (mut snapshot, windows) = pair_fixture();
    let t3 = &mut snapshot.runs[0].tasks[0];
    t3.rounds.retain(|round| round.role == AgentRole::Worker);
    let app = app_of((snapshot, windows));
    let run = inspect_node(&app, &NodeKey::Run("r1".into()));
    assert_eq!(
        value(&run, "agents"),
        Some("workers 1/3 · readers 0/3 · claude ok · codex ok")
    );
}

/// m4: the implementer is live only while its round is open; ended and not merged,
/// it is ended.
#[test]
fn the_pair_rows_implementer_is_live_only_while_its_round_is_open() {
    let (mut snapshot, _) = pair_fixture();
    let t3 = &mut snapshot.runs[0].tasks[0];
    assert_eq!(
        pair_row(t3, false).as_deref(),
        Some("test writer ✓ red a1b2c3d → implementer ●")
    );
    for round in &mut t3.rounds {
        round.ended_at.get_or_insert(round.started_at + 10);
    }
    t3.state = TaskState::Review;
    assert_eq!(
        pair_row(t3, false).as_deref(),
        Some("test writer ✓ red a1b2c3d → implementer –")
    );
    assert_eq!(
        pair_row(t3, true).as_deref(),
        Some("test writer + red a1b2c3d -> implementer _")
    );
}

fn review(lane: Option<RaceLane>, round: u32) -> ReviewInfo {
    let (snapshot, _) = race_fixture();
    ReviewInfo {
        lane,
        round,
        ..snapshot.runs[0].tasks[0].reviews[0].clone()
    }
}

/// m5: before the crown a racing task counts its reviews per lane; after it, only the
/// winner's (and the task's own, which carry no lane).
#[test]
fn a_racing_tasks_review_count_is_per_lane_then_the_winners() {
    let (mut snapshot, _) = race_fixture();
    let t2 = &mut snapshot.runs[0].tasks[0];
    let (a, b) = (Some(RaceLane::A), Some(RaceLane::B));
    t2.reviews = vec![review(b, 1), review(a, 1), review(b, 2)];
    lane_state(t2, RaceLane::A, LaneState::Review);
    assert_eq!(review_row(t2), "in review · a r1 · b r2");

    // Lane b won and was crowned: the task is in review again on its own (no lane).
    lane_state(t2, RaceLane::A, LaneState::Lost);
    lane_state(t2, RaceLane::B, LaneState::Won);
    t2.race.as_mut().unwrap().winner = b;
    t2.reviews.push(review(None, 3));
    t2.state = TaskState::Review;
    assert_eq!(review_row(t2), "in review · r3");
    let app = app_of((snapshot, Vec::new()));
    let task = NodeKey::Task {
        run: "r1".into(),
        id: "t2".into(),
    };
    assert_eq!(
        inspect_node(&app, &task).right.as_deref(),
        Some("in review · r3")
    );
}
