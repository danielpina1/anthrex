//! Milestone 9.3's final fix wave (W1), the chain table: a restart keeps the chains the
//! live table kept. An idle chain the table dropped (the project's older idle one,
//! evicted when a newer chain went idle), ended by a closed window or not, stays out
//! (review A, M1); a chain's runs keep their continue order whatever the clock did
//! (review A, M2); a restart whose window is gone launches a fresh session (A-M3).

use proto::{FinishAction, RunState};

use super::chains::{ended, gone};
use super::fixture::*;
use super::goal_rounds_pr::landed;
use super::goal_rounds_start::{iterate, reply, started};
use super::orch::ORCH;
use super::orch_restore::restart;
use crate::run::chain::ChainState;
use crate::run::engine::{EventKind, OpKind, OpResult, OrchEvent};
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

/// M2, the live table: a delivered run whose chain left the table, iterated, puts the
/// chain back with its runs in their continue order, though the clock stepped back
/// between them (`chains::members`).
#[test]
fn a_chain_back_on_an_iterate_keeps_its_continue_order() {
    let mut fx = landed();
    fx.tick();
    let current = fx.run().clone();
    let mut first = another(
        &current,
        ("engine-test-1111", A),
        RunState::Accepted,
        0,
        "x",
    );
    first.created_at = current.created_at + 20;
    first.continued_by = Some("engine-test-2222".into());
    let mut second = another(
        &current,
        ("engine-test-2222", A),
        RunState::Accepted,
        0,
        "x",
    );
    second.created_at = current.created_at + 10;
    second.continued_by = Some(RUN_ID.into());
    fx.state.runs.insert(first.id.clone(), first);
    fx.state.runs.insert(second.id.clone(), second);
    fx.state.chains.clear();
    fx.run_mut().chain_left = true;
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    let chain = &fx.state.chains[A];
    assert_eq!(chain.runs, ["engine-test-1111", "engine-test-2222", RUN_ID]);
    assert!(!fx.run().chain_left, "the chain is back");
}

/// A-M3, the engine's half: a delivered run iterated after its window was closed (I3
/// lets the user close it). The restart the iterate asked for finds no window: the
/// driver sends `AdoptLost` with the run's handoff, then the restart's failure. The run
/// launches a fresh session with that prompt at once, and the round's request waits
/// for it.
#[test]
fn a_restart_whose_window_is_gone_launches_a_fresh_session() {
    let mut fx = landed();
    fx.tick();
    let o = fx.run_mut().orch.orchestrator.as_mut().unwrap();
    o.live = false;
    o.exited_at = Some(1);
    let effects = iterate(&mut fx, "more");
    assert_eq!(reply(&effects), started(2));
    let restarts = ops_in(&effects, "RestartOrchestrator");
    assert_eq!(restarts.len(), 1, "{effects:#?}");
    fx.next(EventKind::Orch(OrchEvent::AdoptLost {
        run_id: RUN_ID.into(),
        window_id: ORCH,
        first_prompt: "the handoff of 3f9a".into(),
    }));
    let message = format!("window {ORCH} is gone");
    let effects = fx.done(restarts[0].0, OpResult::Failed { message });
    let launches = ops_in(&effects, "CreateOrchestrator");
    assert_eq!(launches.len(), 1, "{effects:#?}");
    let OpKind::CreateOrchestrator { spec, .. } = &launches[0].1 else {
        unreachable!()
    };
    assert_eq!(spec.initial_prompt.as_deref(), Some("the handoff of 3f9a"));
    let run = fx.run();
    assert!(run.orch.request_wake.is_some(), "round 2's request waits");
    assert!(
        run.log.iter().any(|e| e.text
            == format!("window {ORCH} is gone; run 3f9a's orchestrator starts a fresh session")),
        "{:#?}",
        run.log
    );
}
