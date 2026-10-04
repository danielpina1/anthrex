//! Milestone 9.5 task 20: a racing task and a paired task (decision 29, Interfaces
//! "Run view"), for the tree, canvas, inspector and render-audit tests.

use super::{PROJECT, headless, route_json, run, run_ref, snapshot, task, worker};
use proto::{
    AgentRole, LaneInfo, LaneState, PairInfo, PairPhase, RaceInfo, RaceLane, ReviewInfo, Route,
    RunState, RunsSnapshot, Runtime, Size, TaskState, WindowInfo,
};

const NOW: u64 = 10_000;

fn route(runtime: Runtime) -> Route {
    serde_json::from_value(route_json(runtime)).expect("a Route")
}

fn lane(lane: RaceLane, runtime: Runtime, state: LaneState) -> LaneInfo {
    LaneInfo {
        lane,
        route: route(runtime),
        state,
        checkout: format!("t2.{}", lane.label()),
        head: None,
        reason: None,
        salvage_ref: None,
    }
}

/// Run `r1`, `running`, `max_writers = 3`, `writer_caps = {"codex": 1}`, workers 2/3
/// and readers 1/3 busy. Task `t2`, M, `working`, on the critical path, racing with
/// no winner: lane a Claude `working`, its racer (session 1) live on window 7; lane b
/// Codex `review`, its racer (session 2) ended, and its Claude reviewer `review b#1`
/// (session 2, round 1) live on window 9.
pub(crate) fn race_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    let mut r1 = run("r1", PROJECT, RunState::Running);
    r1.created_at = NOW - 600;
    r1.writers_busy = 2;
    r1.readers_busy = 1;
    r1.writer_caps = [("codex".to_owned(), 1)].into();
    r1.critical_path = vec!["t2".into()];

    let mut t2 = task("t2", "reset endpoint", Size::M, TaskState::Working);
    t2.on_critical_path = true;
    let mut racer_a = super::round(AgentRole::Racer, 1, Some(7), Runtime::Claude, NOW - 300);
    racer_a.lane = Some(RaceLane::A);
    let mut racer_b = super::round(AgentRole::Racer, 2, Some(8), Runtime::Codex, NOW - 300);
    racer_b.lane = Some(RaceLane::B);
    racer_b.ended_at = Some(NOW - 120);
    let mut review_b = super::round(AgentRole::Reviewer, 2, Some(9), Runtime::Claude, NOW - 100);
    review_b.round = 1;
    review_b.lane = Some(RaceLane::B);
    t2.review_route = Some(review_b.route.clone());
    t2.reviews = vec![ReviewInfo {
        round: 1,
        route: review_b.route.clone(),
        verdict: None,
        summary: String::new(),
        findings: Vec::new(),
        blocking: false,
        lane: Some(RaceLane::B),
    }];
    // Listed out of start order: the view orders them.
    t2.rounds = vec![review_b, racer_b, racer_a];
    t2.race = Some(RaceInfo {
        lanes: vec![
            lane(RaceLane::A, Runtime::Claude, LaneState::Working),
            LaneInfo {
                head: Some("b0b1b2b3b4b5b6b7b8b9b0b1b2b3b4b5b6b7b8b9".into()),
                ..lane(RaceLane::B, Runtime::Codex, LaneState::Review)
            },
        ],
        winner: None,
        adopted: false,
    });
    r1.tasks = vec![t2];

    let mut racer_ref = run_ref("r1", Some("t2"), AgentRole::Racer, 1);
    racer_ref.lane = Some(RaceLane::A);
    let mut review_ref = run_ref("r1", Some("t2"), AgentRole::Reviewer, 2);
    review_ref.lane = Some(RaceLane::B);
    (
        snapshot(NOW, vec![r1]),
        vec![
            headless(7, "1a2b/t2.aw1", PROJECT, Some(racer_ref)),
            headless(9, "1a2b/t2.b.review1", PROJECT, Some(review_ref)),
        ],
    )
}

/// The red commit of [`pair_fixture`]'s test writer.
const PAIR_RED: &str = "a1b2c3d4e5f60718293a4b5c6d7e8f9012345678";

/// Run `r1`, `running`. Task `t3`, M, hub, `working`, paired in phase `implementing`:
/// its Codex test writer (session 1) ended with `red = a1b2c3d…` checked failing
/// (`red_checked = Some(true)`), and its Claude implementer, worker session 2, live on
/// window 11.
pub(crate) fn pair_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    let mut r1 = run("r1", PROJECT, RunState::Running);
    r1.created_at = NOW - 600;
    r1.writers_busy = 1;

    let mut t3 = task("t3", "token expiry", Size::M, TaskState::Working);
    t3.hub = true;
    let mut writer = super::round(
        AgentRole::TestWriter,
        1,
        Some(10),
        Runtime::Codex,
        NOW - 400,
    );
    writer.ended_at = Some(NOW - 250);
    t3.rounds = vec![worker(2, Some(11), Runtime::Claude, NOW - 200), writer];
    t3.pair = Some(PairInfo {
        phase: PairPhase::Implementing,
        writer_route: route(Runtime::Codex),
        test: Some("reset_token_expires".into()),
        red: Some(PAIR_RED.into()),
        red_checked: Some(true),
        writer_failures: 0,
    });
    r1.tasks = vec![t3];
    (
        snapshot(NOW, vec![r1]),
        vec![headless(
            11,
            "1a2b/t3.w2",
            PROJECT,
            Some(run_ref("r1", Some("t3"), AgentRole::Worker, 2)),
        )],
    )
}
