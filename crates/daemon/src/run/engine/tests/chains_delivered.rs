//! Milestone 9.3, D17: a `pr` run complete with every PR landed (delivered) is finished
//! for its chain. Its chain goes idle and a next goal adopts its window with the
//! `delivered` wake; a `pr` run still delivering keeps its chain active; iterating a
//! delivered run whose idle chain still has it last makes the chain active again, its
//! round wake pasted to the same window; once a next goal has started from the chain,
//! iterating the earlier run is refused.

use super::chains_continue::{adopted, continue_as, wakes};
use super::fixture::*;
use super::goal_rounds_pr::{delivering, landed};
use super::goal_rounds_start::{iterate, reply, round_wake, started};
use super::orch::ORCH;
use crate::run::chain::ChainState;
use crate::run::engine::actions::{ActionNode, available};
use proto::{ActionKind, RunState};

const CHAIN: &str = "o-3f9a";
const NEXT: &str = "engine-test-4c1d";
/// D17's wake after a delivered run (KG §3.3's `<accepted|discarded|delivered>`).
const DELIVERED_WAKE: &str =
    "a new goal, run 4c1d (your previous run 3f9a was delivered):\n```\nAdd a logout button\n```\n";
/// D17's refusal of an iterate once a next goal has started from the chain.
const SUPERSEDED: &str =
    "run 3f9a is no longer its orchestrator's current run; start a new goal instead";

/// [`landed`], stepped once: the chain sees the delivered run.
fn delivered() -> Fixture {
    let mut fx = landed();
    fx.tick();
    assert_eq!(fx.run().state, RunState::Complete);
    fx
}

#[test]
fn a_delivered_pr_runs_chain_goes_idle_and_continues_with_the_delivered_wake() {
    let mut fx = delivered();
    let chain = &fx.state.chains[CHAIN];
    assert_eq!((chain.state, chain.ended), (ChainState::Idle, false));
    let effects = continue_as(&mut fx, NEXT, None);
    assert_eq!(
        adopted(&effects),
        vec![(NEXT.to_string(), ORCH, "4c1d/orchestrator".to_string())]
    );
    assert_eq!(
        fx.state.runs[NEXT].orch.request_wake.as_deref(),
        Some(DELIVERED_WAKE)
    );
    // The delivered run has not ended, so its record is released here: the window is
    // the next run's now.
    let prev = fx.run();
    assert_eq!(prev.continued_by.as_deref(), Some(NEXT));
    assert!(!prev.orch.orchestrator.as_ref().unwrap().live);
    assert_eq!(fx.state.chains[CHAIN].state, ChainState::Active);
}

#[test]
fn a_delivering_pr_run_does_not_go_idle() {
    let mut fx = delivering();
    fx.tick();
    assert_eq!(fx.state.chains[CHAIN].state, ChainState::Active);
    let effects = continue_as(&mut fx, NEXT, None);
    assert_eq!(
        reply(&effects),
        Err("run 3f9a is still going; finish it before starting another goal".into())
    );
}

#[test]
fn iterating_a_delivered_pr_run_reactivates_its_chain_in_the_same_window() {
    let mut fx = delivered();
    assert_eq!(fx.state.chains[CHAIN].state, ChainState::Idle);
    let mark = fx.log.len();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    let chain = &fx.state.chains[CHAIN];
    assert_eq!(chain.state, ChainState::Active);
    assert_eq!(
        (chain.runs.as_slice(), chain.window_id),
        (&[RUN_ID.to_string()][..], ORCH)
    );
    fx.tick();
    let pasted: Vec<_> = fx.log[mark..]
        .iter()
        .filter_map(|e| match e {
            crate::run::engine::Effect::WakeOrchestrator {
                run_id,
                window_id,
                request,
                text,
                ..
            } if run_id == RUN_ID => Some((*window_id, *request, text.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(pasted.len(), 1, "{pasted:#?}");
    assert_eq!((pasted[0].0, pasted[0].1), (ORCH, Some(2)));
    assert!(
        pasted[0].2.starts_with(&round_wake(2, 1, "more")),
        "{pasted:#?}"
    );
}

#[test]
fn iterating_after_a_next_goal_is_refused() {
    let mut fx = delivered();
    continue_as(&mut fx, NEXT, None);
    assert_eq!(reply(&iterate(&mut fx, "more")), Err(SUPERSEDED.into()));
    assert_eq!(fx.run().rounds.len(), 1);
    assert_eq!(fx.run().state, RunState::Complete);
    // Decision 32: the action menu lists only what decision 9 allows.
    let kinds: Vec<_> = available(fx.run(), &ActionNode::Run)
        .into_iter()
        .map(|a| a.kind)
        .collect();
    assert!(!kinds.contains(&ActionKind::Iterate), "{kinds:?}");
    assert!(wakes(&fx.log, RUN_ID).iter().all(|(_, r)| *r != Some(2)));
}
