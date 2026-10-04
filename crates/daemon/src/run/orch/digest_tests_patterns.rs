//! Milestone 9.5 decision 32: the digest's and `task_result`'s `race` and `pair`, the
//! fingerprint over them, and the trim dropping them with their task. Pure.

use proto::{
    Effort, GateCounts, LaneState, PairPhase, RaceLane, Route, Runtime, Spend, Strength, TaskState,
};

use super::*;
use crate::run::model::{Lane, Pair, Race};
use crate::run::orch::result::task_result;
use crate::run::orch::test_support::*;

fn on(runtime: Runtime) -> Route {
    Route {
        runtime,
        model: String::new(),
        strength: Strength::Standard,
        effort: Effort::Medium,
    }
}

fn lane(lane: RaceLane, runtime: Runtime, state: LaneState) -> Lane {
    Lane {
        lane,
        route: on(runtime),
        review_route: None,
        checkout: format!("t1.{}", lane.label()),
        state,
        session: 1,
        start_commit: None,
        head: None,
        done: None,
        failures: 0,
        bounces: GateCounts::default(),
        stalls: 0,
        budget_exceeded: 0,
        spent: Spend::default(),
        reason: None,
        salvage_ref: None,
        cleared_locks: Vec::new(),
        kill_sent_at: None,
        exited: false,
        removed: false,
        kept: false,
    }
}

fn racing() -> Race {
    Race {
        lanes: vec![
            lane(RaceLane::A, Runtime::Claude, LaneState::Working),
            lane(RaceLane::B, Runtime::Codex, LaneState::Review),
        ],
        winner: None,
        adopted: false,
        started_at: 0,
    }
}

fn pair(phase: PairPhase, red: Option<&str>) -> Pair {
    Pair {
        phase,
        writer_route: on(Runtime::Codex),
        test: Some("t2::works".into()),
        red: red.map(str::to_string),
        red_checked: red.map(|_| true),
        writer_failures: 0,
        writer_sessions: 1,
    }
}

fn entry<'a>(d: &'a Value, id: &str) -> &'a Value {
    (d["tasks"].as_array().unwrap().iter())
        .find(|t| t["id"] == id)
        .unwrap_or_else(|| panic!("no task {id}"))
}

fn two() -> Run {
    run_with(&[
        task_toml("t1", "S", "[\"crates/a/**\"]", ""),
        task_toml("t2", "S", "[\"crates/b/**\"]", ""),
    ])
}

#[test]
fn digest_shows_race_and_pair() {
    let mut run = two();
    let d = digest(&run, 0);
    for id in ["t1", "t2"] {
        assert_eq!(entry(&d, id)["race"], Value::Null, "{id}");
        assert_eq!(entry(&d, id)["pair"], Value::Null, "{id}");
    }
    task_mut(&mut run, "t1").race = Some(racing());
    task_mut(&mut run, "t2").pair = Some(pair(PairPhase::Writing, None));
    let d = digest(&run, 0);
    assert_eq!(entry(&d, "t1")["race"], "a claude working · b codex review");
    assert_eq!(entry(&d, "t2")["pair"], "writing the test");

    // A lane's state change moves the fingerprint; so does the winner.
    let before = fingerprint(&run);
    let race = task_mut(&mut run, "t1").race.as_mut().unwrap();
    race.lanes[1].state = LaneState::Won;
    let lane_moved = fingerprint(&run);
    assert_ne!(before, lane_moved, "a lane state change");
    task_mut(&mut run, "t1").race.as_mut().unwrap().winner = Some(RaceLane::B);
    assert_ne!(fingerprint(&run), lane_moved, "the winner");
    task_mut(&mut run, "t2").pair = Some(pair(PairPhase::Implementing, Some(&"a1b2c3d".repeat(6))));
    let d = digest(&run, 0);
    assert_eq!(entry(&d, "t1")["race"], "won by b");
    assert_eq!(entry(&d, "t2")["pair"], "implementing, red a1b2c3d");

    // `task_result` carries the same two texts.
    let result = task_result(&run, run.task("t1").unwrap(), None);
    assert_eq!(result["task"]["race"], "won by b");
    assert_eq!(result["task"]["pair"], Value::Null);
    let result = task_result(&run, run.task("t2").unwrap(), None);
    assert_eq!(result["task"]["pair"], "implementing, red a1b2c3d");
}

/// `digest_trim` drops a trimmed task's `race` and `pair` with the rest of its entry.
#[test]
fn a_trimmed_task_takes_its_race_and_pair_with_it() {
    let mut run = run_of(60);
    for task in run.tasks.iter_mut() {
        task.spec.title = "t".repeat(120);
        task.spec.deps = (0..20).map(|i| format!("dep-{i:012}")).collect();
        event(task, 1, &"h".repeat(4_000));
    }
    let finished = task_mut(&mut run, "t0");
    finished.state = TaskState::Merged;
    finished.race = Some(Race {
        winner: Some(RaceLane::A),
        ..racing()
    });
    task_mut(&mut run, "t1").state = TaskState::Merged;
    task_mut(&mut run, "t1").pair = Some(pair(PairPhase::Implementing, None));
    let d = digest(&run, 0);
    assert!(
        d["omitted_tasks"].as_u64().unwrap() >= 2,
        "{}",
        d["omitted_tasks"]
    );
    let shown: Vec<&str> = (d["tasks"].as_array().unwrap().iter())
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert!(
        !shown.contains(&"t0") && !shown.contains(&"t1"),
        "{shown:?}"
    );
    let text = d.to_string();
    assert!(
        !text.contains("won by") && !text.contains("implementing"),
        "{text}"
    );
    // Every task still shown keeps both keys.
    for task in d["tasks"].as_array().unwrap() {
        let keys = task.as_object().unwrap();
        assert!(
            keys.contains_key("race") && keys.contains_key("pair"),
            "{task}"
        );
    }
}
