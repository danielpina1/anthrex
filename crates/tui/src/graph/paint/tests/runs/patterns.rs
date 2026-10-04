//! Milestone 9.5 task 20: racer and test-writer nodes (decision 29, Interfaces "Run
//! view"): their text, and their glyphs through 9.0.7's kit.

use super::*;
use crate::graph::paint::style::node_glyph;
use crate::graph::run_text::round_text;
use crate::theme::Role;
use crate::tree::display_rounds;
use crate::tree::run_fixtures::{pair_fixture, race_fixture};
use proto::{LaneState, PairPhase, RaceLane};

fn round_key(
    task: &str,
    role: AgentRole,
    lane: Option<RaceLane>,
    session: u32,
    round: u32,
) -> NodeKey {
    NodeKey::AgentRound {
        run: "r1".into(),
        task: task.into(),
        role,
        lane,
        session,
        round,
    }
}

/// The glyph and role of the run-view row `key`.
fn look(app: &App, key: &NodeKey) -> (&'static str, Role) {
    let rows = view_rows(app);
    let row = rows
        .iter()
        .find(|row| &row.key == key)
        .unwrap_or_else(|| panic!("{key:?} is a row"));
    node_glyph(row, app)
}

fn lane_mut(snapshot: &mut RunsSnapshot, lane: RaceLane) -> &mut proto::LaneInfo {
    let race = snapshot.runs[0].tasks[0].race.as_mut().expect("a race");
    race.lanes
        .iter_mut()
        .find(|info| info.lane == lane)
        .expect("the lane")
}

#[test]
fn node_text_for_new_rounds() {
    let (mut snapshot, windows) = race_fixture();
    let racer_a = snapshot.runs[0].tasks[0]
        .rounds
        .iter_mut()
        .find(|round| round.role == AgentRole::Racer && round.lane == Some(RaceLane::A))
        .expect("racer a");
    racer_a.sent_back_at = vec![racer_a.started_at + 60];
    let mut texts: Vec<String> = display_rounds(&snapshot.runs[0].tasks[0], &windows)
        .iter()
        .map(round_text)
        .collect();
    let (mut pair, pair_windows) = pair_fixture();
    let writer = pair.runs[0].tasks[0]
        .rounds
        .iter_mut()
        .find(|round| round.role == AgentRole::TestWriter)
        .expect("the test writer");
    writer.sent_back_at = vec![writer.started_at + 30];
    texts.extend(
        display_rounds(&pair.runs[0].tasks[0], &pair_windows)
            .iter()
            .map(round_text),
    );
    assert_eq!(
        texts,
        [
            "racer a claude",
            "racer b codex",
            "racer a r2 claude",
            "review b#1 claude",
            "test writer #1 codex",
            "test writer #1 r2 codex",
            "worker #2 claude",
        ]
    );
    for text in &texts {
        assert!(UnicodeWidthStr::width(text.as_str()) <= 24, "{text:?}");
    }
}

#[test]
fn a_live_racer_and_test_writer_follow_a_workers_rules() {
    let app = app_of(race_fixture());
    let racer_a = round_key("t2", AgentRole::Racer, Some(RaceLane::A), 1, 1);
    assert_eq!(look(&app, &racer_a), ("●", Role::Working));
    let (mut snapshot, windows) = race_fixture();
    let round = &mut snapshot.runs[0].tasks[0].rounds[2];
    assert_eq!(round.lane, Some(RaceLane::A));
    round.rate_limited_until = Some(snapshot.now + 60);
    let app = app_of((snapshot, windows));
    assert_eq!(look(&app, &racer_a), ("⊘", Role::Paused));

    let (mut snapshot, windows) = pair_fixture();
    let t3 = &mut snapshot.runs[0].tasks[0];
    t3.rounds
        .retain(|round| round.role == AgentRole::TestWriter);
    t3.rounds[0].ended_at = None;
    t3.pair.as_mut().expect("a pair").phase = PairPhase::Writing;
    let app = app_of((snapshot, windows));
    let writer = round_key("t3", AgentRole::TestWriter, None, 1, 1);
    assert_eq!(look(&app, &writer), ("●", Role::Working));
}

#[test]
fn loser_and_out_rounds_are_ended_and_muted() {
    let racer_a = round_key("t2", AgentRole::Racer, Some(RaceLane::A), 1, 1);
    let racer_b = round_key("t2", AgentRole::Racer, Some(RaceLane::B), 2, 1);
    let end_a = |snapshot: &mut RunsSnapshot| {
        let t2 = &mut snapshot.runs[0].tasks[0];
        for round in &mut t2.rounds {
            round.ended_at.get_or_insert(snapshot.now - 10);
        }
    };

    // Lane b won, lane a lost.
    let (mut snapshot, windows) = race_fixture();
    end_a(&mut snapshot);
    lane_mut(&mut snapshot, RaceLane::A).state = LaneState::Lost;
    lane_mut(&mut snapshot, RaceLane::B).state = LaneState::Won;
    snapshot.runs[0].tasks[0]
        .race
        .as_mut()
        .expect("a race")
        .winner = Some(RaceLane::B);
    let app = app_of((snapshot, windows));
    assert_eq!(look(&app, &racer_a), ("–", Role::Muted));
    assert_eq!(look(&app, &racer_b), ("✓", Role::Done));

    // Lane a out, lane b adopted.
    let (mut snapshot, windows) = race_fixture();
    end_a(&mut snapshot);
    lane_mut(&mut snapshot, RaceLane::A).state = LaneState::Out;
    lane_mut(&mut snapshot, RaceLane::B).state = LaneState::Adopted;
    let app = app_of((snapshot, windows));
    assert_eq!(look(&app, &racer_a), ("–", Role::Muted));
    assert_eq!(look(&app, &racer_b), ("✓", Role::Done));
}

#[test]
fn an_ended_test_writer_passed_unless_its_task_is_blocked_writing() {
    let writer = round_key("t3", AgentRole::TestWriter, None, 1, 1);
    let app = app_of(pair_fixture());
    assert_eq!(look(&app, &writer), ("✓", Role::Done));

    let (mut snapshot, windows) = pair_fixture();
    let t3 = &mut snapshot.runs[0].tasks[0];
    t3.state = TaskState::Blocked;
    t3.rounds
        .retain(|round| round.role == AgentRole::TestWriter);
    let pair = t3.pair.as_mut().expect("a pair");
    pair.phase = PairPhase::Writing;
    pair.red = None;
    pair.red_checked = None;
    let app = app_of((snapshot, windows));
    assert_eq!(look(&app, &writer), ("✗", Role::Failed));

    // Only the last test-writer round of the task fails.
    let (mut snapshot, windows) = pair_fixture();
    let t3 = &mut snapshot.runs[0].tasks[0];
    t3.state = TaskState::Blocked;
    t3.rounds
        .retain(|round| round.role == AgentRole::TestWriter);
    let mut second = t3.rounds[0].clone();
    second.session = 2;
    second.round = 2;
    second.started_at += 200;
    second.ended_at = Some(second.started_at + 10);
    t3.rounds.push(second);
    t3.pair.as_mut().expect("a pair").phase = PairPhase::Writing;
    let app = app_of((snapshot, windows));
    assert_eq!(look(&app, &writer), ("✓", Role::Done));
    let last = round_key("t3", AgentRole::TestWriter, None, 2, 1);
    assert_eq!(look(&app, &last), ("✗", Role::Failed));
}

#[test]
fn a_live_racer_animates_its_task() {
    let (snapshot, mut windows) = race_fixture();
    windows[0].status = Status::Working;
    let app = app_of((snapshot, windows));
    let task = NodeKey::Task {
        run: "r1".into(),
        id: "t2".into(),
    };
    let (glyph, role) = look(&app, &task);
    assert_eq!(role, Role::Working);
    assert_eq!(glyph, theme::spinner(app.spinner_frame, false));
}

/// Ruling T20-2 (I1): both lanes' first reviewers are review round 1; each reviewer's
/// glyph is its own lane's verdict. Lane a's is live with none yet, lane b's blocked.
#[test]
fn each_lane_reviewer_shows_its_own_lanes_verdict() {
    let app = app_of(crate::tree::run_fixtures::lane_reviews_fixture());
    let a = round_key("t2", AgentRole::Reviewer, Some(RaceLane::A), 1, 1);
    let b = round_key("t2", AgentRole::Reviewer, Some(RaceLane::B), 1, 1);
    assert_eq!(look(&app, &a), ("●", Role::Working));
    assert_eq!(look(&app, &b), ("✗", Role::Failed));
}

/// Ruling T20-2 (m2): a working paired task animates while its live test writer's
/// window is `Working`, with no worker round at all.
#[test]
fn a_live_test_writer_animates_its_task() {
    use crate::tree::run_fixtures::{PROJECT, headless, run_ref};
    let (mut snapshot, _) = pair_fixture();
    let t3 = &mut snapshot.runs[0].tasks[0];
    t3.rounds
        .retain(|round| round.role == AgentRole::TestWriter);
    t3.rounds[0].ended_at = None;
    t3.pair.as_mut().expect("a pair").phase = PairPhase::Writing;
    let reference = run_ref("r1", Some("t3"), AgentRole::TestWriter, 1);
    let mut window = headless(10, "1a2b/t3.t1", PROJECT, Some(reference));
    window.status = Status::Working;
    let app = app_of((snapshot, vec![window]));
    let task = NodeKey::Task {
        run: "r1".into(),
        id: "t3".into(),
    };
    let (glyph, role) = look(&app, &task);
    assert_eq!(role, Role::Working);
    assert_eq!(glyph, theme::spinner(app.spinner_frame, false));
}
