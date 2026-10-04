//! Milestone 9.5 task 20: racers, test writers and capped writers in the inspector
//! (decision 29, Interfaces "Run view"): the task's `race` and `pair` rows and state
//! word, a racer's `race` and a test writer's `test` field, and the run's `agents` row.

use super::run_patterns::{pair_row, race_field, race_row, test_field};
use super::run_tests::{app_of, inspect_node, pairs, value};
use crate::app::App;
use crate::tree::NodeKey;
use crate::tree::run_fixtures::{pair_fixture, race_fixture};
use proto::{
    AgentRole, LaneInfo, LaneState, PairInfo, PairPhase, RaceInfo, RaceLane, RunsSnapshot,
    TaskInfo, TaskState, WindowInfo,
};

fn task_key(id: &str) -> NodeKey {
    NodeKey::Task {
        run: "r1".into(),
        id: id.into(),
    }
}

fn round_key(task: &str, role: AgentRole, lane: Option<RaceLane>, session: u32) -> NodeKey {
    NodeKey::AgentRound {
        run: "r1".into(),
        task: task.into(),
        role,
        lane,
        session,
        round: 1,
    }
}

fn race(task: &mut TaskInfo) -> &mut RaceInfo {
    task.race.as_mut().expect("a race")
}

fn lane(task: &mut TaskInfo, lane: RaceLane) -> &mut LaneInfo {
    race(task)
        .lanes
        .iter_mut()
        .find(|info| info.lane == lane)
        .expect("the lane")
}

fn pair(task: &mut TaskInfo) -> &mut PairInfo {
    task.pair.as_mut().expect("a pair")
}

fn changed(
    (mut snapshot, windows): (RunsSnapshot, Vec<WindowInfo>),
    change: impl FnOnce(&mut TaskInfo),
) -> App {
    change(&mut snapshot.runs[0].tasks[0]);
    app_of((snapshot, windows))
}

/// The DETAIL section's labels, in order, and the value of `label`.
fn detail(app: &App, id: &str, label: &str) -> (Vec<&'static str>, Option<String>) {
    let inspection = inspect_node(app, &task_key(id));
    let detail = inspection
        .sections
        .iter()
        .find(|section| section.title == "DETAIL")
        .expect("a DETAIL section");
    let labels = detail.fields.iter().map(|field| field.label).collect();
    let value = detail
        .fields
        .iter()
        .find(|field| field.label == label)
        .map(|field| field.value.clone());
    (labels, value)
}

#[test]
fn task_race_row() {
    let (snapshot, _) = race_fixture();
    let t2 = &snapshot.runs[0].tasks[0];
    assert_eq!(
        race_row(t2, false).as_deref(),
        Some("a claude ● · b codex ◐")
    );
    assert_eq!(
        race_row(t2, true).as_deref(),
        Some("a claude * · b codex %")
    );
    let (snapshot, _) = pair_fixture();
    assert_eq!(race_row(&snapshot.runs[0].tasks[0], false), None);

    let app = app_of(race_fixture());
    let (labels, row) = detail(&app, "t2", "race");
    assert_eq!(row.as_deref(), Some("a claude ● · b codex ◐"));
    let route = labels.iter().position(|label| *label == "route");
    assert_eq!(
        route.map(|at| at + 1),
        labels.iter().position(|l| *l == "race")
    );
}

#[test]
fn task_race_row_for_each_lane_state_and_its_end() {
    let row = |a: LaneState, b: LaneState, winner: Option<RaceLane>, adopted: bool| {
        let (mut snapshot, _) = race_fixture();
        let t2 = &mut snapshot.runs[0].tasks[0];
        lane(t2, RaceLane::A).state = a;
        lane(t2, RaceLane::B).state = b;
        race(t2).winner = winner;
        race(t2).adopted = adopted;
        race_row(t2, false).expect("a race row")
    };
    let b = Some(RaceLane::B);
    assert_eq!(
        row(LaneState::Preparing, LaneState::Proof, None, false),
        "a claude ● · b codex ◇"
    );
    assert_eq!(
        row(LaneState::Check, LaneState::Working, None, false),
        "a claude ◇ · b codex ●"
    );
    assert_eq!(
        row(LaneState::Lost, LaneState::Won, b, false),
        "a claude – · b codex ✓ · won by b"
    );
    assert_eq!(
        row(LaneState::Out, LaneState::Adopted, b, true),
        "a claude – · b codex ✓ · b adopted"
    );
}

#[test]
fn task_pair_row() {
    let (snapshot, _) = pair_fixture();
    let t3 = &snapshot.runs[0].tasks[0];
    assert_eq!(
        pair_row(t3, false).as_deref(),
        Some("test writer ✓ red a1b2c3d → implementer ●")
    );
    assert_eq!(
        pair_row(t3, true).as_deref(),
        Some("test writer + red a1b2c3d -> implementer *")
    );

    let app = app_of(pair_fixture());
    let (labels, row) = detail(&app, "t3", "pair");
    assert_eq!(
        row.as_deref(),
        Some("test writer ✓ red a1b2c3d → implementer ●")
    );
    let route = labels.iter().position(|label| *label == "route");
    assert_eq!(
        route.map(|at| at + 1),
        labels.iter().position(|l| *l == "pair")
    );
}

#[test]
fn task_pair_row_while_writing_failed_and_merged() {
    let row = |change: &dyn Fn(&mut TaskInfo)| {
        let (mut snapshot, _) = pair_fixture();
        let t3 = &mut snapshot.runs[0].tasks[0];
        change(t3);
        pair_row(t3, false)
    };
    let writing = |t3: &mut TaskInfo| {
        t3.rounds
            .retain(|round| round.role == AgentRole::TestWriter);
        t3.rounds[0].ended_at = None;
        let pair = pair(t3);
        pair.phase = PairPhase::Writing;
        pair.red = None;
        pair.red_checked = None;
    };
    assert_eq!(
        row(&writing).as_deref(),
        Some("test writer ● → implementer ◌")
    );
    let failed = |t3: &mut TaskInfo| {
        writing(t3);
        t3.rounds[0].ended_at = Some(t3.rounds[0].started_at + 10);
        t3.state = TaskState::Blocked;
    };
    assert_eq!(
        row(&failed).as_deref(),
        Some("test writer ✗ → implementer ◌")
    );
    let merged = |t3: &mut TaskInfo| {
        t3.state = TaskState::Merged;
        for round in &mut t3.rounds {
            round.ended_at.get_or_insert(round.started_at + 10);
        }
    };
    assert_eq!(
        row(&merged).as_deref(),
        Some("test writer ✓ red a1b2c3d → implementer ✓")
    );
    // An unpaired task has no row.
    let (snapshot, _) = race_fixture();
    assert_eq!(pair_row(&snapshot.runs[0].tasks[0], false), None);
}

#[test]
fn racer_round_race_field() {
    let field = |change: &dyn Fn(&mut TaskInfo), which: RaceLane| {
        let (mut snapshot, _) = race_fixture();
        let t2 = &mut snapshot.runs[0].tasks[0];
        change(t2);
        race_field(t2, Some(which))
    };
    let (a, b) = (RaceLane::A, RaceLane::B);
    assert_eq!(field(&|_| {}, a).as_deref(), Some("lane a · racing"));
    assert_eq!(field(&|_| {}, b).as_deref(), Some("lane b · racing"));
    let won = |t2: &mut TaskInfo| {
        lane(t2, RaceLane::A).state = LaneState::Lost;
        lane(t2, RaceLane::B).state = LaneState::Won;
        race(t2).winner = Some(RaceLane::B);
    };
    assert_eq!(field(&won, b).as_deref(), Some("lane b · won"));
    assert_eq!(
        field(&won, a).as_deref(),
        Some("lane a · lost to b · salvage pending")
    );
    let salvaged = |t2: &mut TaskInfo| {
        won(t2);
        lane(t2, RaceLane::A).salvage_ref = Some("refs/anthrex/salvage/t2-1".into());
    };
    assert_eq!(
        field(&salvaged, a).as_deref(),
        Some("lane a · lost to b · salvaged refs/anthrex/salvage/t2-1")
    );
    let adopted = |t2: &mut TaskInfo| {
        let out = lane(t2, RaceLane::A);
        out.state = LaneState::Out;
        out.reason = Some("budget exceeded".into());
        lane(t2, RaceLane::B).state = LaneState::Adopted;
        race(t2).winner = Some(RaceLane::B);
        race(t2).adopted = true;
    };
    assert_eq!(
        field(&adopted, b).as_deref(),
        Some("lane b · adopted after lane a went out")
    );
    assert_eq!(
        field(&adopted, a).as_deref(),
        Some("lane a · out: budget exceeded · salvage pending")
    );
    let out_salvaged = |t2: &mut TaskInfo| {
        adopted(t2);
        lane(t2, RaceLane::A).salvage_ref = Some("refs/anthrex/salvage/t2-2".into());
    };
    assert_eq!(
        field(&out_salvaged, a).as_deref(),
        Some("lane a · out: budget exceeded · salvaged refs/anthrex/salvage/t2-2")
    );
    // A round with no lane, or a task with no race, has no field.
    let (snapshot, _) = race_fixture();
    assert_eq!(race_field(&snapshot.runs[0].tasks[0], None), None);
    let (snapshot, _) = pair_fixture();
    assert_eq!(race_field(&snapshot.runs[0].tasks[0], Some(a)), None);
}

#[test]
fn a_racer_rounds_race_field_follows_doing() {
    let app = app_of(race_fixture());
    let inspection = inspect_node(
        &app,
        &round_key("t2", AgentRole::Racer, Some(RaceLane::A), 1),
    );
    assert_eq!(inspection.name, "racer a  claude · standard · medium");
    let labels: Vec<&str> = pairs(&inspection).iter().map(|(label, _)| *label).collect();
    assert_eq!(labels[..2], ["doing", "race"]);
    assert_eq!(value(&inspection, "race"), Some("lane a · racing"));
    let inspection = inspect_node(
        &app,
        &round_key("t2", AgentRole::Reviewer, Some(RaceLane::B), 2),
    );
    assert!(
        inspection.name.starts_with("review b#1  claude"),
        "{}",
        inspection.name
    );
    assert_eq!(
        value(&inspection, "judging"),
        Some("racer b · codex · standard")
    );
}

#[test]
fn test_writer_round_test_field() {
    let field = |change: &dyn Fn(&mut PairInfo), ascii: bool| {
        let (mut snapshot, _) = pair_fixture();
        let t3 = &mut snapshot.runs[0].tasks[0];
        change(pair(t3));
        test_field(t3, ascii)
    };
    assert_eq!(
        field(&|_| {}, false).as_deref(),
        Some("reset_token_expires · red a1b2c3d · fails at red ✓")
    );
    assert_eq!(
        field(&|_| {}, true).as_deref(),
        Some("reset_token_expires · red a1b2c3d · fails at red +")
    );
    let passed = |pair: &mut PairInfo| pair.red_checked = Some(false);
    assert_eq!(
        field(&passed, false).as_deref(),
        Some("reset_token_expires · red a1b2c3d · passed at red ✗")
    );
    assert_eq!(
        field(&passed, true).as_deref(),
        Some("reset_token_expires · red a1b2c3d · passed at red x")
    );
    let unchecked = |pair: &mut PairInfo| pair.red_checked = None;
    assert_eq!(
        field(&unchecked, false).as_deref(),
        Some("reset_token_expires · red a1b2c3d")
    );
    let writing = |pair: &mut PairInfo| {
        pair.phase = PairPhase::Writing;
        pair.red = None;
        pair.red_checked = None;
    };
    assert_eq!(
        field(&writing, false).as_deref(),
        Some("writing reset_token_expires")
    );

    let app = app_of(pair_fixture());
    let inspection = inspect_node(&app, &round_key("t3", AgentRole::TestWriter, None, 1));
    assert_eq!(inspection.name, "test writer #1  codex · standard · medium");
    let labels: Vec<&str> = pairs(&inspection).iter().map(|(label, _)| *label).collect();
    assert_eq!(labels[..2], ["doing", "test"]);
}

#[test]
fn agents_row_shows_capped_writers() {
    let app = app_of(race_fixture());
    let inspection = inspect_node(&app, &NodeKey::Run("r1".into()));
    assert_eq!(
        value(&inspection, "agents"),
        Some("workers 2/3 · readers 1/3 · claude ok · codex ok (writers 1/3)")
    );
}

#[test]
fn the_task_state_word_says_racing_and_writing_the_test() {
    let app = app_of(race_fixture());
    let right = inspect_node(&app, &task_key("t2")).right;
    assert_eq!(right.as_deref(), Some("racing"));
    // Once a lane wins, the race is over: the task's own word.
    let app = changed(race_fixture(), |t2| race(t2).winner = Some(RaceLane::B));
    let right = inspect_node(&app, &task_key("t2")).right;
    assert_eq!(right.as_deref(), Some("working"));

    let app = changed(pair_fixture(), |t3| pair(t3).phase = PairPhase::Writing);
    let right = inspect_node(&app, &task_key("t3")).right;
    assert_eq!(right.as_deref(), Some("writing the test"));
    let app = app_of(pair_fixture());
    let right = inspect_node(&app, &task_key("t3")).right;
    assert_eq!(right.as_deref(), Some("working"));
    // A blocked writer's task says why it is blocked.
    let app = changed(pair_fixture(), |t3| {
        pair(t3).phase = PairPhase::Writing;
        t3.state = TaskState::Blocked;
    });
    let right = inspect_node(&app, &task_key("t3")).right;
    assert_eq!(right.as_deref(), Some("blocked"));
}

/// Task 20b: decision 22's kept checkout (`LaneInfo.kept`) follows the salvage.
#[test]
fn a_kept_checkout_is_named_after_its_salvage() {
    let field = |change: &dyn Fn(&mut TaskInfo)| {
        let (mut snapshot, _) = race_fixture();
        let t2 = &mut snapshot.runs[0].tasks[0];
        lane(t2, RaceLane::B).state = LaneState::Won;
        race(t2).winner = Some(RaceLane::B);
        let a = lane(t2, RaceLane::A);
        a.state = LaneState::Lost;
        a.salvage_ref = Some("refs/anthrex/salvage/t2-1".into());
        a.kept = true;
        change(t2);
        race_field(t2, Some(RaceLane::A))
    };
    assert_eq!(
        field(&|_| {}).as_deref(),
        Some("lane a · lost to b · salvaged refs/anthrex/salvage/t2-1 · checkout kept")
    );
    let out = |t2: &mut TaskInfo| {
        let a = lane(t2, RaceLane::A);
        a.state = LaneState::Out;
        a.reason = Some("its racer stalled".into());
    };
    assert_eq!(
        field(&out).as_deref(),
        Some(
            "lane a · out: its racer stalled · salvaged refs/anthrex/salvage/t2-1 · checkout kept"
        )
    );
    let pending = |t2: &mut TaskInfo| lane(t2, RaceLane::A).salvage_ref = None;
    assert_eq!(
        field(&pending).as_deref(),
        Some("lane a · lost to b · salvage pending · checkout kept")
    );
    let not_kept = |t2: &mut TaskInfo| lane(t2, RaceLane::A).kept = false;
    assert_eq!(
        field(&not_kept).as_deref(),
        Some("lane a · lost to b · salvaged refs/anthrex/salvage/t2-1")
    );
}

/// Task 20b: the snapshot as the daemon fills it (both lanes' first reviewers on
/// session 1): each lane reviewer is inspected on its own and judges its own racer,
/// and the task shows its race row.
#[test]
fn each_lane_reviewer_is_inspected_on_its_own() {
    let app = app_of(crate::tree::run_fixtures::lane_reviews_fixture());
    for (lane, racer) in [
        (RaceLane::A, "racer a · claude · standard"),
        (RaceLane::B, "racer b · codex · standard"),
    ] {
        let key = round_key("t2", AgentRole::Reviewer, Some(lane), 1);
        let inspection = inspect_node(&app, &key);
        let name = format!("review {}#1  claude", lane.label());
        assert!(inspection.name.starts_with(&name), "{}", inspection.name);
        assert_eq!(value(&inspection, "judging"), Some(racer));
    }
    let (_, row) = detail(&app, "t2", "race");
    let review = crate::theme::glyph(crate::theme::Glyph::Review, false);
    let live = crate::theme::glyph(crate::theme::Glyph::Live, false);
    assert_eq!(row, Some(format!("a claude {review} · b codex {live}")));
}

/// A race with no lane left in it (both out, the task retried as one worker:
/// `Race.ended`) is no longer `racing`.
#[test]
fn a_race_with_no_lane_left_is_not_racing() {
    let app = changed(race_fixture(), |t2| {
        lane(t2, RaceLane::A).state = LaneState::Out;
        lane(t2, RaceLane::B).state = LaneState::Out;
    });
    let right = inspect_node(&app, &task_key("t2")).right;
    assert_eq!(right.as_deref(), Some("working"));
}
