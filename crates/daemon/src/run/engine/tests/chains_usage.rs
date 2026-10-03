//! Milestone 9.5 decision 37 (FU-F40, F43): orchestrator usage across a chain. An
//! adopted session keeps posting under the chain's first run, and the driver credits
//! those posts to the chain's current run; the current run counts only what the session
//! spent after it adopted the window (`usage_at_adopt`), a run a later one continued is
//! credited no more, and a fresh session is metered as before.

use proto::{FinishAction, RunState, TokenUsage};

use super::chains::{ended, gone};
use super::chains_continue::continue_as;
use super::fixture::*;
use super::goal_rounds_pr::landed;
use super::orch::ORCH;
use crate::run::engine::{EngineState, EventKind, OpResult};

const NEXT: &str = "engine-test-4c1d";
const THIRD: &str = "engine-test-7e2b";

fn usage(n: u64) -> TokenUsage {
    TokenUsage {
        input: n,
        output: 2 * n,
        cache_read: 3 * n,
        cache_write: 4 * n,
    }
}

/// The OTLP ledger's total `usage` for `run`, as the driver's drain sends it.
fn post(fx: &mut Fixture, run: &str, usage: TokenUsage) {
    fx.next(EventKind::OrchestratorUsage {
        run_id: run.into(),
        usage,
    });
}

fn credited(fx: &Fixture, run: &str) -> TokenUsage {
    fx.state.runs[run].orchestrator_usage
}

fn at_adopt(fx: &Fixture, run: &str) -> TokenUsage {
    let o = fx.state.runs[run].orch.orchestrator.as_ref();
    o.expect("an orchestrator").usage_at_adopt
}

/// A `pr` run delivered with every PR landed (D17), its chain idle, credited 1000.
fn delivered_with_usage() -> Fixture {
    let mut fx = landed();
    fx.tick();
    assert_eq!(fx.run().state, RunState::Complete);
    post(&mut fx, RUN_ID, usage(1000));
    assert_eq!(credited(&fx, RUN_ID), usage(1000));
    fx
}

#[test]
fn a_delivered_previous_run_is_not_credited() {
    let mut fx = delivered_with_usage();
    continue_as(&mut fx, NEXT, None);
    assert_eq!(fx.run().continued_by.as_deref(), Some(NEXT));
    // A total that reaches the engine under the delivered run's id after the adoption
    // (one the driver coalesced before it) is dropped: the adoption counted it.
    post(&mut fx, RUN_ID, usage(1300));
    assert_eq!(credited(&fx, RUN_ID), usage(1000));
}

#[test]
fn the_new_run_counts_only_what_was_spent_after_adopting() {
    let mut fx = delivered_with_usage();
    continue_as(&mut fx, NEXT, None);
    assert_eq!(at_adopt(&fx, NEXT), usage(1000));
    post(&mut fx, NEXT, usage(1500));
    assert_eq!(credited(&fx, NEXT), usage(500));
    // The second run accepted, a third run adopts the window: the session's counter
    // is the second run's credit plus its own adoption point.
    fx.state.runs.get_mut(NEXT).unwrap().state = RunState::Accepted;
    fx.tick();
    continue_as(&mut fx, THIRD, None);
    assert_eq!(fx.state.runs[NEXT].continued_by.as_deref(), Some(THIRD));
    assert_eq!(at_adopt(&fx, THIRD), usage(1500));
    post(&mut fx, THIRD, usage(1800));
    assert_eq!(credited(&fx, THIRD), usage(300));
    // Per field, saturating at 0.
    let lower = TokenUsage {
        input: 2000,
        ..usage(1400)
    };
    post(&mut fx, THIRD, lower);
    let expected = TokenUsage {
        input: 500,
        ..TokenUsage::default()
    };
    assert_eq!(credited(&fx, THIRD), expected);
}

#[test]
fn a_fresh_session_is_metered_as_before() {
    let mut fx = ended(FinishAction::Accept);
    gone(&mut fx, ORCH);
    continue_as(&mut fx, NEXT, None);
    assert_eq!(at_adopt(&fx, NEXT), TokenUsage::default());
    post(&mut fx, NEXT, usage(700));
    assert_eq!(credited(&fx, NEXT), usage(700));
}

/// A session the run launches itself after its adoption was lost posts under its own
/// id from zero, so nothing is subtracted from it.
#[test]
fn a_launch_after_a_lost_adoption_counts_from_zero() {
    let mut fx = delivered_with_usage();
    continue_as(&mut fx, NEXT, None);
    fx.next(EventKind::Orch(crate::run::engine::OrchEvent::AdoptLost {
        run_id: NEXT.into(),
        window_id: ORCH,
        first_prompt: "the handoff of 4c1d".into(),
    }));
    let reply = fx.reply();
    let effects = fx.next(EventKind::Resume {
        reply,
        run_id: NEXT.into(),
        rebaseline: None,
    });
    let (op, _) = ops_in(&effects, "CreateOrchestrator")
        .pop()
        .expect("a fresh launch");
    fx.next(EventKind::OpDone {
        run_id: NEXT.into(),
        op,
        result: OpResult::Window {
            window_id: ORCH + 1,
            pid: None,
        },
    });
    assert_eq!(at_adopt(&fx, NEXT), TokenUsage::default());
    post(&mut fx, NEXT, usage(40));
    assert_eq!(credited(&fx, NEXT), usage(40));
}

/// A new daemon's ledger counts from zero again, and the restored usage is the base:
/// the adoption point no longer applies.
#[test]
fn a_restore_drops_the_adoption_point() {
    let mut fx = delivered_with_usage();
    continue_as(&mut fx, NEXT, None);
    post(&mut fx, NEXT, usage(1500));
    let runs: Vec<_> = fx.state.runs.values().cloned().collect();
    fx.state = EngineState::default();
    fx.next(EventKind::Restore {
        held: Vec::new(),
        runs,
        replay: Vec::new(),
    });
    assert_eq!(at_adopt(&fx, NEXT), TokenUsage::default());
    post(&mut fx, NEXT, usage(40));
    assert_eq!(credited(&fx, NEXT), usage(540));
}
