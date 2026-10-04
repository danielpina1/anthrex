//! Task M9.5.20's review fix round (ruling T20-2, folded into task 20b): a lane
//! reviewer's own verdict and findings (I1), a racer's `fixing` from its own lane, a
//! reviewer with no lane judging a racer (m2), the `agents` row's runtimes routed only
//! by a lane or a test writer (m3), the pair row's implementer mark (m4) and a racing
//! task's review count (m5).

use super::run_patterns::pair_row;
use super::run_task_outcome::review_row;
use super::run_task_sections::check_line;
use super::run_tests::{app_of, inspect_node, value};
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{lane_reviews_fixture, pair_fixture, race_fixture};
use proto::{AgentRole, CheckInfo, LaneState, RaceLane, ReviewInfo, TaskInfo, TaskState, Verdict};

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

/// A failed check of the race fixture's `t2`, made in `lane`.
fn failed_check(lane: Option<RaceLane>, at: u64) -> CheckInfo {
    CheckInfo {
        at,
        ok: false,
        code: Some(1),
        timed_out: false,
        secs: 3,
        summary: "the expiry test failed".into(),
        on_candidate: false,
        decider_summary: None,
        summary_source: None,
        tier: None,
        lane,
    }
}

/// A verdict of lane `lane` (or the task's own), `blocking` or approving.
fn judged(lane: Option<RaceLane>, round: u32, blocking: bool) -> ReviewInfo {
    let verdict = if blocking {
        Verdict::Changes
    } else {
        Verdict::Approve
    };
    ReviewInfo {
        verdict: Some(verdict),
        summary: "fix the expiry".into(),
        blocking,
        ..review(lane, round)
    }
}

/// Task 20b's carry with its review's m2: before a race has a winner, the check row and
/// the review row name the lane they come from; with a winner they are the winner's
/// (the daemon sends its check) and name none; a race over with no winner counts only
/// the task's own reviews.
#[test]
fn a_race_names_the_lane_of_its_check_and_review_until_it_has_a_winner() {
    let (mut snapshot, _) = race_fixture();
    let run = snapshot.runs[0].clone();
    let t2 = &mut snapshot.runs[0].tasks[0];
    t2.last_check = Some(failed_check(Some(RaceLane::B), 9_950));
    assert_eq!(
        check_line(&run, t2),
        "racer b · ✗ failed · the expiry test failed → bounced (check 0/2)"
    );
    t2.reviews = vec![judged(Some(RaceLane::B), 1, true)];
    lane_state(t2, RaceLane::B, LaneState::Working);
    assert_eq!(review_row(t2), "racer b · r1 ✗ changes · fix the expiry");

    // Lane a won: its check, and no lane review of its own yet.
    t2.race.as_mut().unwrap().winner = Some(RaceLane::A);
    lane_state(t2, RaceLane::A, LaneState::Won);
    lane_state(t2, RaceLane::B, LaneState::Lost);
    t2.last_check = Some(failed_check(Some(RaceLane::A), 9_960));
    assert_eq!(
        check_line(&run, t2),
        "✗ failed · the expiry test failed → bounced (check 0/2)"
    );
    assert_eq!(review_row(t2), "not yet");

    // Both lanes out, no winner, then retried as one worker and reviewed on its own.
    t2.race.as_mut().unwrap().winner = None;
    lane_state(t2, RaceLane::A, LaneState::Out);
    lane_state(t2, RaceLane::B, LaneState::Out);
    t2.reviews.push(judged(None, 1, false));
    assert_eq!(review_row(t2), "r1 ✓ approve · fix the expiry");
    t2.state = TaskState::Review;
    assert_eq!(review_row(t2), "in review · r1");
}

/// Task 20b's carry: a racer is never `fixing` another lane's failed check, only its
/// own lane's.
#[test]
fn a_racer_is_not_fixing_another_lanes_check() {
    let fixing_of_a = |lane: RaceLane| {
        let (mut snapshot, windows) = lane_reviews_fixture();
        let now = snapshot.now;
        let t2 = &mut snapshot.runs[0].tasks[0];
        t2.last_check = Some(failed_check(Some(lane), now - 50));
        for round in &mut t2.rounds {
            if round.role == AgentRole::Racer {
                round.ended_at = None;
                round.sent_back_at = vec![now - 30];
            }
        }
        let app = app_of((snapshot, windows));
        let a = inspect_node(&app, &round_key(AgentRole::Racer, Some(RaceLane::A), 1, 2));
        value(&a, "fixing").map(str::to_owned)
    };
    assert_eq!(
        fixing_of_a(RaceLane::B),
        None,
        "lane b's check is not lane a's"
    );
    assert_eq!(
        fixing_of_a(RaceLane::A).as_deref(),
        Some("check failed: the expiry test failed")
    );
}

/// Review D, M-2: the acceptance marks follow the review the task counts: after the
/// crown the winner's, even when the loser's review came last.
#[test]
fn the_accept_marks_follow_the_winners_review() {
    let (mut snapshot, _) = race_fixture();
    let t2 = &mut snapshot.runs[0].tasks[0];
    t2.race.as_mut().unwrap().winner = Some(RaceLane::B);
    t2.reviews = vec![
        judged(Some(RaceLane::B), 1, false),
        judged(Some(RaceLane::A), 1, true),
    ];
    let review = super::run_task_outcome::accept_review(t2).expect("a review");
    assert_eq!((review.lane, review.blocking), (Some(RaceLane::B), false));
}
