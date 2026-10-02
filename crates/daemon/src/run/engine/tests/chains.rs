//! Milestone 9.3 task 6a: the chain table inside `step` (decision 19). A run that gets
//! an orchestrator starts a chain; accepted or discarded, its chain is idle; failed,
//! its chain is dropped; an idle chain whose window is gone ends; the snapshot lists
//! idle orchestrators; a restart keeps active chains and ends idle ones.

use proto::{FinishAction, IdleOrchestrator, RunState};

use super::fixture::*;
use super::goal_rounds_start::complete;
use super::orch::{ORCH, launched, planned};
use super::promote::promoted;
use crate::run::chain::ChainState;
use crate::run::engine::{Effect, EngineState, EventKind, OpKind, OpResult, OrchEvent};
use crate::run::snapshot::snapshot;

/// The fixture run's chain, after its id's last four characters.
const CHAIN: &str = "o-3f9a";

/// `run accept` or `run discard` of the fixture run, its op answered.
fn finished(fx: &mut Fixture, action: FinishAction) {
    let reply = fx.reply();
    fx.next(EventKind::Finish {
        reply,
        run_id: RUN_ID.into(),
        action,
    });
    let (name, outcome) = match action {
        FinishAction::Accept => ("Accept", "accepted"),
        FinishAction::Discard => ("Discard", "discarded"),
    };
    let (op, _) = fx.op(name);
    fx.done(
        op,
        OpResult::Finished {
            outcome: outcome.into(),
            kept_branches: Vec::new(),
        },
    );
}

/// [`complete`], then finished with `action`.
fn ended(action: FinishAction) -> Fixture {
    let mut fx = complete();
    assert_eq!(fx.state.chains[CHAIN].state, ChainState::Active);
    finished(&mut fx, action);
    fx
}

fn gone(fx: &mut Fixture, window_id: u32) -> Vec<Effect> {
    fx.next(EventKind::Orch(OrchEvent::ChainWindowGone {
        chain: CHAIN.into(),
        window_id,
    }))
}

/// Whether the step published a structural snapshot.
fn published(effects: &[Effect]) -> bool {
    effects
        .iter()
        .any(|e| matches!(e, Effect::Publish { structural: true }))
}

#[test]
fn a_planned_run_starts_its_chain() {
    let fx = planned(false);
    assert_eq!(fx.run().chain.as_deref(), Some(CHAIN));
    let chain = &fx.state.chains[CHAIN];
    assert_eq!(chain.state, ChainState::Active);
    assert_eq!(chain.runs, [RUN_ID]);
    assert_eq!(chain.project, fx.run().project);
    // The orchestrator's MCP target names the chain (decision 20).
    let (_, kind) = fx.op("CreateOrchestrator");
    let OpKind::CreateOrchestrator { role, .. } = kind else {
        unreachable!()
    };
    assert_eq!(role.mcp.chain.as_deref(), Some(CHAIN));
    // Its window, once the launch answers.
    let fx = launched(false);
    assert_eq!(fx.state.chains[CHAIN].window_id, ORCH);
}

#[test]
fn a_promoted_run_starts_its_chain() {
    let fx = promoted();
    assert_eq!(fx.run().chain.as_deref(), Some(CHAIN));
    let chain = &fx.state.chains[CHAIN];
    assert_eq!((chain.state, chain.window_id), (ChainState::Active, ORCH));
    let (_, kind) = fx.op("CreateOrchestrator");
    let OpKind::CreateOrchestrator { role, .. } = kind else {
        unreachable!()
    };
    assert_eq!(role.mcp.chain.as_deref(), Some(CHAIN));
}

#[test]
fn a_run_with_no_orchestrator_has_no_chain() {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "auth", "")]));
    fx.ready(false);
    assert_eq!(fx.run().chain, None);
    assert!(fx.state.chains.is_empty());
}

#[test]
fn an_accepted_runs_orchestrator_becomes_idle() {
    let fx = ended(FinishAction::Accept);
    assert_eq!(fx.run().state, RunState::Accepted);
    let chain = &fx.state.chains[CHAIN];
    assert_eq!(chain.state, ChainState::Idle);
    assert!(!chain.ended);
    assert_eq!(
        (chain.runs.as_slice(), chain.window_id),
        (&[RUN_ID.to_string()][..], ORCH)
    );
    // Its window is released as a finished run's is (decision 19).
    assert!(!fx.run().orch.orchestrator.as_ref().unwrap().live);
}

#[test]
fn a_discarded_one_too() {
    let fx = ended(FinishAction::Discard);
    assert_eq!(fx.run().state, RunState::Discarded);
    let chain = &fx.state.chains[CHAIN];
    assert_eq!(chain.state, ChainState::Idle);
    assert!(!chain.ended);
}

#[test]
fn a_failed_runs_chain_ends() {
    let mut fx = planned(false);
    assert!(fx.state.chains.contains_key(CHAIN));
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(
        op,
        OpResult::Failed {
            message: "no space left".into(),
        },
    );
    assert_eq!(fx.run().state, RunState::Failed);
    assert!(fx.state.chains.is_empty(), "{:#?}", fx.state.chains);
    assert!(snapshot(&fx.state, fx.now).idle_orchestrators.is_empty());
}

#[test]
fn a_closed_window_ends_the_chain() {
    let mut fx = ended(FinishAction::Accept);
    // Another window's report changes nothing.
    let revision = fx.state.revision;
    let effects = gone(&mut fx, ORCH + 1);
    assert!(!fx.state.chains[CHAIN].ended);
    assert!(
        !published(&effects) && fx.state.revision == revision,
        "{effects:#?}"
    );
    // The table's change alone is published: the snapshot's idle list changed.
    let effects = gone(&mut fx, ORCH);
    assert!(published(&effects), "{effects:#?}");
    assert_eq!(fx.state.revision, revision + 1);
    let chain = &fx.state.chains[CHAIN];
    assert!(chain.ended);
    assert_eq!(
        chain.state,
        ChainState::Idle,
        "still the project's idle chain"
    );

    // An active chain's window is decision 13's: the report is ignored.
    let mut fx = complete();
    gone(&mut fx, ORCH);
    assert!(!fx.state.chains[CHAIN].ended);
}

#[test]
fn the_snapshot_lists_idle_orchestrators() {
    let mut fx = complete();
    let snap = snapshot(&fx.state, fx.now);
    assert!(
        snap.idle_orchestrators.is_empty(),
        "an active chain is not idle"
    );
    assert_eq!(snap.runs[0].chain.as_deref(), Some(CHAIN));

    finished(&mut fx, FinishAction::Accept);
    let route = fx.run().orch.orchestrator.as_ref().unwrap().route.clone();
    let expected = IdleOrchestrator {
        chain: CHAIN.into(),
        project: fx.run().project.clone(),
        after_run: RUN_ID.into(),
        outcome: RunState::Accepted,
        runtime: route.runtime,
        model: route.model,
        window_id: Some(ORCH),
        fresh: false,
        runs: 1,
    };
    let snap = snapshot(&fx.state, fx.now);
    assert_eq!(snap.idle_orchestrators, vec![expected.clone()]);

    gone(&mut fx, ORCH);
    let snap = snapshot(&fx.state, fx.now);
    let fresh = IdleOrchestrator {
        window_id: None,
        fresh: true,
        ..expected
    };
    assert_eq!(snap.idle_orchestrators, vec![fresh]);
}

#[test]
fn an_active_chain_stays_with_its_run_and_an_idle_one_ends() {
    let idle = ended(FinishAction::Accept).run().clone();
    // A second, active planned run of another chain.
    let mut active = launched(false).run().clone();
    active.id = "engine-test-4c1d".into();
    active.chain = Some("o-4c1d".into());
    let mut fx = complete();
    fx.state = EngineState::default();
    fx.next(EventKind::Restore {
        runs: vec![idle, active],
        replay: Vec::new(),
        held: Vec::new(),
    });
    let chains = &fx.state.chains;
    assert_eq!(chains.len(), 2, "{chains:#?}");
    let restored = &chains["o-4c1d"];
    assert_eq!(restored.state, ChainState::Active);
    assert!(!restored.ended);
    assert_eq!(restored.runs, ["engine-test-4c1d"]);
    assert_eq!(restored.window_id, ORCH);
    let ended = &chains[CHAIN];
    assert_eq!(ended.state, ChainState::Idle);
    assert!(ended.ended, "an idle orchestrator is not relaunched");
    let snap = snapshot(&fx.state, fx.now);
    assert_eq!(snap.idle_orchestrators.len(), 1);
    assert!(snap.idle_orchestrators[0].fresh);
}
