//! Milestone 9.3 task M9.3.7 (decision 28): the digest names the current round and its
//! first stage, so the orchestrator can tell earlier rounds' stages from its own, and
//! the fingerprint covers both. Pure.

use proto::{RoundOrigin, RoundOutcome, RunState};

use super::*;
use crate::run::model::Round;
use crate::run::orch::test_support::*;

/// A complete run of two tasks in stage 1, with round 1 recorded (decision 18).
fn complete_run() -> Run {
    let mut run = run_with(&[
        task_toml("t1", "S", "[\"crates/a/**\"]", ""),
        task_toml("t2", "S", "[\"crates/b/**\"]", ""),
    ]);
    run.state = RunState::Complete;
    run.orch.orchestrator = Some(orchestrator());
    run.rounds = vec![Round::first(&run)];
    run
}

/// Round `n` of `run`, started by the orchestrator, its tasks from stage `first_stage`.
fn add_round(run: &mut Run, n: u32, first_stage: u16) {
    if let Some(last) = run.rounds.last_mut() {
        last.outcome = Some(RoundOutcome::Completed);
        last.ended_at = Some(run.created_at + 60);
    }
    run.rounds.push(Round {
        n,
        goal: format!("round {n}'s request"),
        origin: RoundOrigin::Orchestrator,
        started_at: run.created_at + 120,
        ended_at: None,
        outcome: None,
        summary: None,
        first_stage,
        windows_before: 0,
        scouts_before: 0,
    });
    run.state = RunState::Planning;
}

#[test]
fn the_digest_names_the_round_and_its_first_stage() {
    let mut run = complete_run();
    let d = digest(&run, 0);
    assert_eq!(
        (&d["run"]["round"], &d["run"]["first_stage"]),
        (&json!(1), &json!(1))
    );

    add_round(&mut run, 2, 2);
    let d = digest(&run, 0);
    assert_eq!(
        (&d["run"]["round"], &d["run"]["first_stage"]),
        (&json!(2), &json!(2))
    );

    // A run loaded before 9.3 and not yet restored has no round recorded: round 1,
    // stage 1 (decision 18).
    run.rounds.clear();
    let d = digest(&run, 0);
    assert_eq!(
        (&d["run"]["round"], &d["run"]["first_stage"]),
        (&json!(1), &json!(1))
    );
}

#[test]
fn the_fingerprint_moves_when_the_round_does() {
    let mut run = complete_run();
    run.state = RunState::Planning;
    let one = fingerprint(&run);

    // The same state, the same tasks: only the round and its first stage move.
    add_round(&mut run, 2, 2);
    let two = fingerprint(&run);
    assert_ne!(one, two, "a new round moves the fingerprint");

    // The first stage alone moves it too.
    run.rounds.last_mut().unwrap().first_stage = 3;
    assert_ne!(fingerprint(&run), two, "the round's first stage moves it");

    // And a recorded round 1 fingerprints as an unrecorded one.
    let mut fresh = complete_run();
    let recorded = fingerprint(&fresh);
    fresh.rounds.clear();
    assert_eq!(fingerprint(&fresh), recorded);

    assert!(note_change(&mut run), "a round's start bumps digest_rev");
}
