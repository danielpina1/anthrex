//! Task M9.5.3b: the race, pair and per-runtime concurrency model persists through
//! `run.json` (`journal::save_run`, the driver's serializer, and `journal::load_all`),
//! and a protocol-15 (9.3) `run.json` loads with none of it and writes none of it.

use proto::{GateCounts, LaneState, PairPhase, RaceLane, Route, Spend};

use super::*;
use crate::run::journal::{self, RUN_FILE};
use crate::run::model::{DoneClaim, Run};

/// A `run.json` written by milestone 9.3's code (`c8d77428`), captured in task M9.5.2:
/// `t1` in the merge queue with a worker and a reviewer round, a check, a proof and a
/// review; `t2` working; a `MergeCandidate` and a `VerifyDone` pending.
const M93_RUN: &str = include_str!("../../tests/fixtures/run/m93-run.json");

/// Every key milestone 9.5 adds to the persisted run. `lane` is also an older key, a
/// routing decision's (milestone 9), so it is counted rather than looked for.
const NEW_KEYS: [&str; 8] = [
    "race",
    "pair",
    "race_wait_since",
    "concurrency",
    "route_lists",
    "list_pick",
    "list_escalation",
    "environment_failed",
];

fn old_run() -> Run {
    serde_json::from_str(M93_RUN).expect("m93-run.json parses")
}

fn tmp() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("ax-model-tuning-")
        .tempdir_in("/tmp")
        .expect("temp dir")
}

/// Writes `run` as `run.json` under `data` and reads it back the way a daemon start does.
fn save_and_load(run: &mut Run, data: &std::path::Path) -> (Run, String) {
    run.data_dir = journal::runs_dir(data).join(&run.id);
    journal::save_run(run).expect("save_run");
    let text = std::fs::read_to_string(run.data_dir.join(RUN_FILE)).expect("run.json");
    let (mut runs, problems) = journal::load_all(data);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(runs.len(), 1);
    (runs.remove(0).0, text)
}

/// The object keys anywhere in `value`, with repeats.
fn keys(value: &serde_json::Value) -> Vec<String> {
    fn walk(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::Object(map) => {
                for (key, inner) in map {
                    out.push(key.clone());
                    walk(inner, out);
                }
            }
            serde_json::Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            _ => {}
        }
    }
    let mut out = Vec::new();
    walk(value, &mut out);
    out
}

fn count(keys: &[String], key: &str) -> usize {
    keys.iter().filter(|k| *k == key).count()
}

fn lane(lane: RaceLane, route: Route) -> Lane {
    Lane {
        lane,
        route,
        review_route: None,
        checkout: format!("t1.{}", lane.label()),
        state: LaneState::Preparing,
        session: 0,
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

#[test]
fn a_run_with_race_pair_and_caps_round_trips() {
    let mut run = old_run();
    let route = run.tasks[0].route.clone();
    let mut peer = route.clone();
    peer.runtime = proto::Runtime::Codex;
    peer.model = "gpt-5.5-codex".into();

    let mut a = lane(RaceLane::A, route.clone());
    a.review_route = Some(peer.clone());
    a.state = LaneState::Review;
    a.session = 1;
    a.start_commit = Some("b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0b0".into());
    a.head = Some("d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1".into());
    a.done = Some(DoneClaim {
        summary: "did it".into(),
        test: Some("a::works".into()),
        red: Some("abcdef1".into()),
        signal: proto::DoneSignal::TaskDone,
        session: Some(1),
    });
    a.failures = 1;
    a.bounces.proof = 1;
    a.spent = Spend {
        tool_calls: 12,
        secs: 300,
        tokens: 4000,
    };
    let mut b = lane(RaceLane::B, peer.clone());
    b.state = LaneState::Out;
    b.session = 2;
    b.stalls = 1;
    b.budget_exceeded = 1;
    b.reason = Some("a second gate failure".into());
    b.salvage_ref = Some(format!("refs/anthrex/salvage/{}/t1/1", run.id));
    b.cleared_locks = vec!["index.lock".into()];
    b.kill_sent_at = Some(400);
    b.exited = true;
    b.removed = true;
    b.kept = true;
    run.tasks[0].race = Some(Race {
        lanes: vec![a, b],
        winner: Some(RaceLane::A),
        adopted: false,
        started_at: 100,
    });
    run.tasks[0].race_wait_since = Some(90);
    run.tasks[1].pair = Some(Pair {
        phase: PairPhase::Implementing,
        writer_route: peer,
        test: Some("b::fails_first".into()),
        red: Some("e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2e2".into()),
        red_checked: Some(true),
        writer_failures: 1,
        writer_sessions: 2,
    });
    run.concurrency.insert(
        "codex".into(),
        RuntimeConcurrency {
            cap: 1,
            last_rate_limit_at: Some(200),
            last_change_at: Some(200),
            halvings: 2,
            ..Default::default()
        },
    );
    let t1 = &mut run.tasks[0];
    t1.rounds[0].lane = Some(RaceLane::B);
    t1.checks[0].lane = Some(RaceLane::B);
    t1.proofs[0].lane = Some(RaceLane::B);
    t1.reviews[0].lane = Some(RaceLane::B);
    let op = run
        .pending_ops
        .values_mut()
        .find(|op| op.task_id.as_deref() == Some("t1"))
        .expect("t1 has a pending op");
    op.lane = Some(RaceLane::B);
    // Task M9.5.10a: a frozen model list and the picks it made.
    let candidate = proto::RoutingCandidate {
        route: route.clone(),
        skipped_reason: None,
    };
    let pick = ListPick {
        candidates: vec![candidate],
        chosen: Some(0),
        pick: ListPolicy::Spread,
        slot: Some(0),
    };
    run.limits.route_lists.m = FrozenList {
        candidates: vec![ListCandidate {
            runtime: route.runtime,
            model: route.model.clone(),
            strength: route.strength,
            effort: None,
        }],
        pick: ListPolicy::Spread,
    };
    run.tasks[0].list_pick = Some(pick.clone());
    run.tasks[0].list_escalation = Some(pick);
    // Task M9.5.10b (ruling RL-1): a session that failed for an environment reason.
    run.tasks[0].rounds[0].environment_failed = true;

    let dir = tmp();
    let (loaded, text) = save_and_load(&mut run, dir.path());
    let written = keys(&serde_json::from_str(&text).expect("run.json is JSON"));
    let before = keys(&serde_json::from_str(M93_RUN).expect("fixture"));
    for key in NEW_KEYS {
        assert!(count(&written, key) > 0, "run.json lacks {key}");
    }
    // The two lanes' own, then a round, a check, a proof, a review and a pending op.
    assert_eq!(count(&written, "lane"), count(&before, "lane") + 2 + 5);
    assert_eq!(loaded, run);
}

#[test]
fn an_old_run_json_still_loads() {
    let mut run = old_run();
    assert!(run.concurrency.is_empty());
    assert!(run.pending_ops.values().all(|op| op.lane.is_none()));
    assert_eq!(run.tasks.len(), 2);
    for task in &run.tasks {
        assert_eq!(task.race, None, "{}", task.id());
        assert_eq!(task.pair, None, "{}", task.id());
        assert_eq!(task.race_wait_since, None, "{}", task.id());
        assert!(task.rounds.iter().all(|r| r.lane.is_none()));
        assert!(task.checks.iter().all(|c| c.lane.is_none()));
        assert!(task.proofs.iter().all(|p| p.lane.is_none()));
        assert!(task.reviews.iter().all(|r| r.lane.is_none()));
    }

    // Written again, it is the JSON 9.3 wrote, so it has none of the new keys.
    let dir = tmp();
    let (loaded, text) = save_and_load(&mut run, dir.path());
    assert_eq!(loaded, run);
    let mut again: serde_json::Value = serde_json::from_str(&text).expect("run.json is JSON");
    let captured: serde_json::Value = serde_json::from_str(M93_RUN).expect("fixture");
    let (written, before) = (keys(&again), keys(&captured));
    for key in NEW_KEYS {
        assert_eq!(count(&written, key), 0, "run.json gained {key}");
    }
    assert_eq!(count(&written, "lane"), count(&before, "lane"));
    // `save_and_load` moved the run's data directory under the temp dir.
    again["data_dir"] = captured["data_dir"].clone();
    assert_eq!(again, captured);
}
