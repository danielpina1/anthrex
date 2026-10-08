//! Milestone 9.5 task M9.5.21 (decision 30): `run status`'s race and pair lines under
//! their task, and the header's writer caps.

use proto::{
    Effort, LaneInfo, LaneState, PairInfo, PairPhase, RaceInfo, RaceLane, Route, Runtime, TaskState,
};

use super::run_block;
use super::tests::example;

fn route(runtime: Runtime) -> Route {
    Route {
        runtime,
        model: String::new(),
        effort: Effort::MEDIUM,
    }
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
        kept: false,
    }
}

fn pair(phase: PairPhase, red: Option<&str>) -> PairInfo {
    PairInfo {
        phase,
        writer_route: route(Runtime::Codex),
        test: Some("t_feat".into()),
        red: red.map(str::to_string),
        red_checked: red.map(|_| true),
        writer_failures: 0,
    }
}

#[test]
fn status_shows_race_pair_and_caps() {
    let mut run = example();
    run.tasks[1].race = Some(RaceInfo {
        lanes: vec![
            lane(RaceLane::A, Runtime::Claude, LaneState::Working),
            lane(RaceLane::B, Runtime::Codex, LaneState::Review),
        ],
        winner: None,
        adopted: false,
    });
    run.tasks[2].state = TaskState::Working;
    run.tasks[2].pair = Some(pair(PairPhase::Writing, None));
    run.tasks[3].pair = Some(pair(PairPhase::Implementing, Some(&"a1b2c3d".repeat(6))));
    run.writer_caps.insert("codex".into(), 1);
    let expected = "\
add-reset-3f9a  running  1/4 merged  base main@1a2b3c4  writers 2/3 (codex cap 1)  readers 1/3  rev 57
  goal: Add password reset
  report: /Users/me/Library/Application Support/anthrex/runs/add-reset-3f9a/REPORT.md
  ID   SIZE MODE   STATE          RUNG BOUNCES        ROUTE                          WINDOWS
  t1   M◆   tdd    merged         0    -              claude claude-opus-5 high      4 5
  t2   M    tdd    review         1    review 1       codex (default) medium         6 9
    race: a claude working · b codex review
  t3   S    check  working        0    -              claude claude-sonnet-5 low
    pair: test writer working; implementer not started
  t4   S    none   blocked        3    check 3        claude claude-sonnet-5 low     7
    pair: test writer done, red a1b2c3d; implementer blocked
  attention: t4 blocked (mis_sized): check failed 3 times
";
    assert_eq!(run_block(&run, 0), expected);
}

/// Each capped runtime adds its own parenthesis; a won and a lost lane show their
/// states.
#[test]
fn every_capped_runtime_and_a_finished_race() {
    let mut run = example();
    run.tasks[1].race = Some(RaceInfo {
        lanes: vec![
            lane(RaceLane::A, Runtime::Claude, LaneState::Lost),
            lane(RaceLane::B, Runtime::Codex, LaneState::Won),
        ],
        winner: Some(RaceLane::B),
        adopted: false,
    });
    run.writer_caps.insert("codex".into(), 1);
    run.writer_caps.insert("claude".into(), 2);
    let text = run_block(&run, 0);
    assert!(
        text.starts_with(
            "add-reset-3f9a  running  1/4 merged  base main@1a2b3c4  writers 2/3 (claude cap 2) (codex cap 1)  readers 1/3  rev 57\n"
        ),
        "{text}"
    );
    assert!(
        text.contains("\n    race: a claude lost · b codex won\n"),
        "{text}"
    );
}
