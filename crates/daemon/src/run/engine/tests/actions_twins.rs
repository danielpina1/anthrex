//! Milestone 9.0.6 decision 42: the pure twins of two mutating guards agree with them
//! (`full::retryable` with `full::retry`, `orch_window::relaunchable` with
//! `orch_window::relaunch`), and the mutating functions behave as before.

use proto::RunState;

use super::actions_rules::running;
use crate::run::engine::{full, orch_window};
use crate::run::model::{InfraFailures, Run};

/// Stage 1 of `run` held by three executor failures on its head (ruling C-18), as
/// `full_fixes.rs::three_failures_hold_the_run_and_resume_retries` reaches it through
/// the executor; built in place here because only the record matters.
pub(super) fn hold_stage(run: &mut Run) {
    let stage = &mut run.stages[0];
    stage.full.infra = Some(InfraFailures {
        commit: stage.head.clone(),
        count: 3,
        at: 0,
        line: String::new(),
    });
}

#[test]
fn retryable_matches_retry() {
    let fx = running();
    let now = fx.now;
    let mut free = fx.run().clone();
    assert!(!full::retryable(&free));
    assert!(!full::retry(&mut free, now));
    assert_eq!(&free, fx.run(), "nothing to retry changes nothing");
    let mut held = fx.run().clone();
    hold_stage(&mut held);
    assert!(full::retryable(&held));
    assert!(full::retry(&mut held, now));
    assert_eq!(
        held.stages[0].full.infra, None,
        "retry forgets the failures"
    );
    assert!(!full::retryable(&held));
}

#[test]
fn relaunchable_matches_relaunch() {
    let now = 0;
    // No orchestrator.
    let mut plain = running().run().clone();
    assert!(!orch_window::relaunchable(&plain));
    assert!(!orch_window::relaunch(&mut plain, now, &mut Vec::new()));
    // A live orchestrator, then a dormant one (a daemon restart).
    let fx = super::orch::launched(false);
    let mut live = fx.run().clone();
    assert!(!orch_window::relaunchable(&live));
    assert!(!orch_window::relaunch(&mut live, now, &mut Vec::new()));
    assert_eq!(&live, fx.run(), "a refused relaunch changes nothing");
    let mut dormant = fx.run().clone();
    orch_window::restored(&mut dormant);
    assert!(orch_window::relaunchable(&dormant));
    let mut effects = Vec::new();
    assert!(orch_window::relaunch(&mut dormant, now, &mut effects));
    assert!(
        !orch_window::relaunchable(&dormant),
        "its restart is in flight"
    );
    // A launch still in flight.
    let mut launching = super::orch::planned(false).run().clone();
    assert!(!orch_window::relaunchable(&launching));
    assert!(!orch_window::relaunch(&mut launching, now, &mut Vec::new()));
    // A run that ended.
    let mut ended = fx.run().clone();
    orch_window::restored(&mut ended);
    ended.state = RunState::Failed;
    assert!(!orch_window::relaunchable(&ended));
    assert!(!orch_window::relaunch(&mut ended, now, &mut Vec::new()));
}
