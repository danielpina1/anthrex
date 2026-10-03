//! Milestone 9.3 task 6b fix round 3 (re-review I1): a delivered run's chain, once it
//! has left the table (the project's older idle chain), stays out. `chains::pass`
//! re-inserts a chain only for a run that is not finished, so a restart keeps the
//! project's newest idle chain, two delivered chains never swap places, and a later
//! accept does not bring the delivered chain back. Iterating the delivered run puts its
//! chain back, active, with that run alone (ruled).

use proto::{FinishAction, RunState};

use super::chains::ended;
use super::fixture::*;
use super::goal_rounds_pr::landed;
use super::goal_rounds_start::{iterate, reply, started};
use super::orch::ORCH;
use crate::run::chain::ChainState;
use crate::run::engine::{Effect, EngineState, EventKind};
use crate::run::model::{LogEntry, Run};

const A: &str = "o-3f9a";
const B: &str = "o-4c1d";
const B_RUN: &str = "engine-test-4c1d";

/// The fixture run delivered (a `pr` run complete with its PR landed), its last
/// `complete: ` entry at `at`.
fn delivered_at(at: u64) -> Run {
    let mut run = landed().run().clone();
    run.log.push(LogEntry {
        at,
        text: "complete: 1 merged, 0 cancelled".into(),
    });
    run
}

/// `run` as run [`B_RUN`] of chain [`B`], with a last entry `text` at `at`.
fn as_b(mut run: Run, text: &str, at: u64) -> Run {
    run.id = B_RUN.into();
    run.chain = Some(B.into());
    run.created_at += 1;
    run.log.push(LogEntry {
        at,
        text: text.into(),
    });
    run
}

/// A fixture whose state is `runs`, restored (`EventKind::Restore`, its `pass`
/// included).
fn restored(runs: Vec<Run>) -> Fixture {
    let mut fx = landed();
    fx.state = EngineState::default();
    fx.next(EventKind::Restore {
        runs,
        replay: Vec::new(),
        held: Vec::new(),
    });
    fx
}

fn table(fx: &Fixture) -> Vec<(String, ChainState, bool)> {
    let chains = fx.state.chains.values();
    chains.map(|c| (c.id.clone(), c.state, c.ended)).collect()
}

fn structural(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::Publish { structural: true }))
}

/// (a) A restart with an older delivered chain and a newer accepted one keeps the
/// newer one only, through the restore step's own `pass` and the steps after it.
#[test]
fn a_restart_keeps_the_newer_idle_chain_over_an_older_delivered_one() {
    let newer = as_b(
        ended(FinishAction::Accept).run().clone(),
        "accepted: merged",
        1_000_000,
    );
    let mut fx = restored(vec![delivered_at(10), newer]);
    let only_b = vec![(B.to_string(), ChainState::Idle, true)];
    assert_eq!(table(&fx), only_b);
    fx.tick();
    fx.tick();
    assert_eq!(table(&fx), only_b);
}

/// (b) Two delivered chains of one project never swap places, and a quiet step
/// publishes no structural snapshot.
#[test]
fn two_delivered_chains_never_swap() {
    let mut b = as_b(
        delivered_at(10),
        "complete: 1 merged, 0 cancelled",
        1_000_000,
    );
    b.state = RunState::Complete;
    let mut fx = restored(vec![delivered_at(10), b]);
    let only_b = vec![(B.to_string(), ChainState::Idle, true)];
    assert_eq!(table(&fx), only_b);
    for _ in 0..3 {
        let effects = fx.tick();
        assert_eq!(table(&fx), only_b);
        assert!(!structural(&effects), "{effects:#?}");
    }
}

/// (c) A is delivered and idle; B, another chain of the project, is accepted later:
/// A leaves the table and does not come back.
#[test]
fn accepting_another_chain_after_a_delivered_one_does_not_bring_it_back() {
    let mut fx = landed();
    fx.tick();
    assert_eq!(fx.state.chains[A].state, ChainState::Idle);
    let mut b = as_b(fx.run().clone(), "running", fx.now);
    b.state = RunState::Running;
    b.delivery.mode = proto::DeliveryMode::Local;
    fx.state.runs.insert(B_RUN.into(), b);
    fx.tick();
    assert_eq!(fx.state.chains[B].state, ChainState::Active);
    let b = fx.state.runs.get_mut(B_RUN).unwrap();
    b.state = RunState::Accepted;
    fx.tick();
    for _ in 0..3 {
        assert_eq!(table(&fx), vec![(B.to_string(), ChainState::Idle, false)]);
        fx.tick();
    }
}

/// Ruled with I1: iterating a delivered run whose chain has left the table puts its
/// chain back, active, with that run alone and the run's own orchestrator window.
#[test]
fn iterating_a_delivered_run_whose_chain_left_the_table_restarts_it() {
    let newer = as_b(
        ended(FinishAction::Accept).run().clone(),
        "accepted: merged",
        1_000_000,
    );
    let mut fx = restored(vec![delivered_at(10), newer]);
    assert!(!fx.state.chains.contains_key(A));
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    let chain = &fx.state.chains[A];
    assert_eq!(chain.state, ChainState::Active);
    assert_eq!(chain.runs, [RUN_ID]);
    assert_eq!(chain.window_id, ORCH);
    assert_eq!(fx.state.chains[B].state, ChainState::Idle);
}
