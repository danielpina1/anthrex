//! Task M9.11.10 (decision 10, A1): the snapshot carries every live run and at most
//! `TERMINAL_RUNS_SHOWN` finished ones, and its last guard keeps it within one frame.
//! Pure.

use proto::{MAX_FRAME, RunState};

use super::*;
use crate::run::engine::EngineState;
use crate::run::model::Run;
use crate::run::orch::test_support::*;
use crate::run::snapshot::{TERMINAL_RUNS_SHOWN, snapshot};
use crate::run::test_support::{PROFILE, build_with, plan_with, show, task_toml};
use crate::run::validate::PLAN_BRIEFS_MAX_BYTES;

fn state_of(runs: Vec<Run>) -> EngineState {
    let mut state = EngineState::default();
    for run in runs {
        state.runs.insert(run.id.clone(), run);
    }
    state
}

fn run_at(id: &str, state: RunState, created_at: u64) -> Run {
    let mut run = run_of(1);
    run.id = id.into();
    run.state = state;
    run.created_at = created_at;
    run
}

const TERMINAL: [RunState; 3] = [RunState::Accepted, RunState::Discarded, RunState::Failed];

fn ids(snap: &RunsSnapshot) -> Vec<String> {
    snap.runs.iter().map(|r| r.run_id.clone()).collect()
}

fn encoded(snap: &RunsSnapshot) -> usize {
    match proto::encode(snap) {
        Ok(frame) => frame.len() - 4,
        Err(proto::CodecError::TooLarge(n)) => n,
        Err(e) => panic!("{e}"),
    }
}

/// 62 runs: the 2 live ones are created before every finished one, so a cut by
/// `created_at` alone, blind to state, would drop them; the 60 finished ones have
/// distinct `created_at`s, inserted in shuffled order (a stride coprime to 60).
#[test]
fn only_the_newest_terminal_runs_are_sent() {
    let mut runs = vec![
        run_at("live-a", RunState::Running, 10),
        run_at("live-b", RunState::AwaitingApproval, 11),
    ];
    for k in 0..60u64 {
        let n = (k * 37) % 60;
        let id = format!("done-{n:02}");
        runs.push(run_at(&id, TERMINAL[(n % 3) as usize], 1_000 + n));
    }
    let snap = snapshot(&state_of(runs), 5_000);
    let mut expected: Vec<String> = (10..60).rev().map(|n| format!("done-{n:02}")).collect();
    expected.extend(["live-b".to_string(), "live-a".to_string()]);
    assert_eq!(ids(&snap), expected);
    assert_eq!(TERMINAL_RUNS_SHOWN, 50);
}

/// The guard's order: the oldest finished run goes first, a live run never. `max` is the
/// encoded size of the live runs and the two newest finished runs together.
#[test]
fn fit_within_drops_the_oldest_terminal_runs_first() {
    let mut runs = vec![
        run_at("live-a", RunState::Running, 50),
        run_at("live-b", RunState::Paused, 10),
    ];
    for n in 0..5u64 {
        runs.push(run_at(
            &format!("done-{n}"),
            TERMINAL[(n % 3) as usize],
            20 + n,
        ));
    }
    let full = snapshot(&state_of(runs.clone()), 5_000);
    let kept: Vec<Run> = (runs.iter())
        .filter(|r| ["live-a", "live-b", "done-4", "done-3"].contains(&r.id.as_str()))
        .cloned()
        .collect();
    let max = encoded(&snapshot(&state_of(kept), 5_000));
    let fitted = fit_within(full, max);
    assert_eq!(ids(&fitted), ["live-a", "done-4", "done-3", "live-b"]);
    assert!(encoded(&fitted) <= max);
}

/// Decision 10's last rule: when only live runs are left and the snapshot is still too
/// large, it is sent as it is (and the largest run is logged).
#[test]
fn a_live_run_alone_over_the_frame_is_sent_and_logged() {
    let runs = vec![
        run_at("live-a", RunState::Running, 50),
        run_at("done-0", RunState::Accepted, 60),
    ];
    let fitted = fit_within(snapshot(&state_of(runs), 5_000), 16);
    assert_eq!(ids(&fitted), ["live-a"]);
}

/// The largest plan the daemon accepts, at its gate (briefs shown): eight ASCII briefs
/// of 524,288 bytes, each under the 1,000,000-character cap and together exactly
/// `PLAN_BRIEFS_MAX_BYTES` (unit 1 byte; 8 × 524,288 = 4,194,304), beside 50 finished
/// runs of 20 tasks, encodes under `MAX_FRAME` with nothing dropped.
#[test]
fn the_largest_accepted_plan_fits_one_frame() {
    let brief = "b".repeat(524_288);
    assert_eq!(8 * brief.len(), PLAN_BRIEFS_MAX_BYTES);
    let tasks: Vec<String> = (0..8)
        .map(|i| {
            task_toml(&format!("t{i}"), "S", &format!("[\"crates/m{i}/**\"]"), "").replace(
                &format!("brief = \"Brief t{i}\""),
                &format!("brief = \"{brief}\""),
            )
        })
        .collect();
    let mut gate = build_with(
        &plan_with(PROFILE, &tasks),
        &config::Orchestrator::default(),
    )
    .unwrap_or_else(|e| panic!("the plan is accepted: {}", show(&e)));
    gate.id = "gate-0001".into();
    gate.state = RunState::AwaitingApproval;
    gate.created_at = 9_000;
    let mut runs = vec![gate];
    for i in 0..50u64 {
        let mut run = run_of(20);
        run.id = format!("done-{i:02}");
        run.state = TERMINAL[(i % 3) as usize];
        run.created_at = 1_000 + i;
        runs.push(run);
    }
    let snap = snapshot(&state_of(runs), 5_000);
    assert_eq!(snap.runs.len(), 51);
    let shown: usize = snap.runs[0].tasks.iter().map(|t| t.brief.len()).sum();
    assert_eq!(shown, PLAN_BRIEFS_MAX_BYTES, "the gate shows every brief");
    let bytes = encoded(&snap);
    assert!(bytes < MAX_FRAME, "{bytes} bytes");
    assert_eq!(
        ids(&fit_frame(snap.clone())),
        ids(&snap),
        "nothing is dropped"
    );
}
