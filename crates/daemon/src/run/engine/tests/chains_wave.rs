//! Milestone 9.3's final fix wave (W1), the chain table: a restart keeps the chains the
//! live table kept. An idle chain the table dropped (the project's older idle one,
//! evicted when a newer chain went idle), ended by a closed window or not, stays out
//! (review A, M1).

use proto::{FinishAction, RunState};

use super::chains::{ended, gone};
use super::fixture::*;
use super::orch::ORCH;
use super::orch_restore::restart;
use crate::run::chain::ChainState;
use crate::run::model::{LogEntry, Run};

const A: &str = "o-3f9a";
const B: &str = "o-4c1d";
const B_RUN: &str = "engine-test-4c1d";
const C_RUN: &str = "engine-test-7e2b";

/// `from` as run `id` of chain `chain`, in `state`, created `later` seconds after it,
/// with a last log entry `text` then.
fn another(from: &Run, (id, chain): (&str, &str), state: RunState, later: u64, text: &str) -> Run {
    let mut run = from.clone();
    run.id = id.into();
    run.chain = Some(chain.into());
    run.state = state;
    run.created_at += later;
    run.continued_by = None;
    let at = run.created_at;
    run.log.push(LogEntry {
        at,
        text: text.into(),
    });
    run
}

fn table(fx: &Fixture) -> Vec<(String, ChainState, Vec<String>)> {
    let chains = fx.state.chains.values();
    chains
        .map(|c| (c.id.clone(), c.state, c.runs.clone()))
        .collect()
}

/// M1: chain A is idle (its run accepted); B, a newer chain of the project, goes idle
/// too, so A leaves the table; B is continued by run C. After a restart the table
/// holds B alone, active on C: A, which the user had moved past, does not come back.
fn a_dropped_chain_stays_out_after_a_restart(end_a_first: bool) {
    let mut fx = ended(FinishAction::Accept);
    if end_a_first {
        gone(&mut fx, ORCH);
        assert!(fx.state.chains[A].ended);
    }
    let a = fx.run().clone();
    let b = another(&a, (B_RUN, B), RunState::Running, 100, "running");
    fx.state.runs.insert(B_RUN.into(), b);
    fx.tick();
    assert_eq!(fx.state.chains[B].state, ChainState::Active);
    assert_eq!(
        fx.state.chains[A].state,
        ChainState::Idle,
        "an active B leaves A"
    );
    let at = fx.now + 200;
    let b = fx.state.runs.get_mut(B_RUN).unwrap();
    b.state = RunState::Accepted;
    b.log.push(LogEntry {
        at,
        text: "accepted: merged".into(),
    });
    fx.tick();
    assert!(!fx.state.chains.contains_key(A), "{:?}", table(&fx));
    // C continues B (decision 23's join, as `requests::start` records it).
    let b = fx.state.runs[B_RUN].clone();
    let c = another(&b, (C_RUN, B), RunState::Planning, 300, "planning");
    fx.state.runs.get_mut(B_RUN).unwrap().continued_by = Some(C_RUN.into());
    fx.state.runs.insert(C_RUN.into(), c);
    restart(&mut fx);
    let only_b = vec![(
        B.to_string(),
        ChainState::Active,
        vec![B_RUN.to_string(), C_RUN.to_string()],
    )];
    assert_eq!(table(&fx), only_b);
    fx.tick();
    assert_eq!(table(&fx), only_b);
}

#[test]
fn an_evicted_idle_chain_stays_out_after_a_restart() {
    a_dropped_chain_stays_out_after_a_restart(false);
}

#[test]
fn an_evicted_ended_chain_stays_out_after_a_restart() {
    a_dropped_chain_stays_out_after_a_restart(true);
}
