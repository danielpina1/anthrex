//! M8b.15: run usage by role (decision 29) and the orchestrator's OTLP usage
//! (decision 30).

use proto::{AgentRole, TokenUsage};

use super::fixture::*;
use super::turns::working;
use crate::run::engine::{Effect, EventKind};
use crate::run::snapshot::snapshot;

fn usage(n: u64) -> TokenUsage {
    TokenUsage {
        input: n,
        output: 10 * n,
        cache_read: 100 * n,
        cache_write: 1000 * n,
    }
}

#[test]
fn run_usage_sums_roles() {
    let (mut fx, _window) = working();
    {
        let run = fx.run_mut();
        let task = &mut run.tasks[0];
        task.rounds[0].usage = usage(1);
        let mut second = task.rounds[0].clone();
        second.usage = usage(2);
        let mut review = task.rounds[0].clone();
        review.role = AgentRole::Reviewer;
        review.usage = usage(4);
        task.rounds.push(second);
        task.rounds.push(review);
        // A task's decider usage is already in the run's; it is not counted twice.
        task.decider_usage = usage(8);
        run.decider_usage = usage(8);
        run.triage_usage = usage(16);
        run.scout_usage = usage(32);
        run.orchestrator_usage = usage(64);
        // Milestone 9 decision 32: sub-planners' sessions.
        run.orch.planner_usage = usage(128);
        run.decider_calls = 3;
        run.decider_fallbacks = 1;
    }
    let info = snapshot(&fx.state, fx.now).runs.remove(0);
    let got = info.usage.expect("RunInfo.usage is filled");
    let by_role: Vec<(&str, TokenUsage)> =
        got.by_role.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    assert_eq!(
        by_role,
        vec![
            ("decider", usage(8 + 16)),
            ("orchestrator", usage(64)),
            ("planner", usage(128)),
            ("reviewer", usage(4)),
            ("scout", usage(32)),
            ("worker", usage(1 + 2)),
        ]
    );
    assert_eq!(got.total, usage(1 + 2 + 4 + 8 + 16 + 32 + 64 + 128));
    assert_eq!((got.decider_calls, got.decider_fallbacks), (3, 1));
}

#[test]
fn a_new_run_shows_every_role_at_zero() {
    let (fx, _window) = working();
    let got = snapshot(&fx.state, fx.now).runs[0].usage.clone().unwrap();
    let roles: Vec<&str> = got.by_role.keys().map(String::as_str).collect();
    assert_eq!(
        roles,
        [
            "decider",
            "orchestrator",
            "planner",
            "reviewer",
            "scout",
            "worker"
        ]
    );
    assert_eq!(got.total, TokenUsage::default());
}

#[test]
fn usage_totals_saturate() {
    let (mut fx, _window) = working();
    let max = TokenUsage {
        input: u64::MAX,
        output: u64::MAX,
        cache_read: u64::MAX,
        cache_write: u64::MAX,
    };
    fx.run_mut().orchestrator_usage = max;
    fx.run_mut().tasks[0].rounds[0].usage = usage(1);
    let got = snapshot(&fx.state, fx.now).runs[0].usage.clone().unwrap();
    assert_eq!(got.total, max);
}

#[test]
fn orchestrator_usage_is_stored_as_a_counter() {
    let (mut fx, _window) = working();
    let now = fx.now + 1;
    let effects = fx.send(
        now,
        EventKind::OrchestratorUsage {
            run_id: RUN_ID.into(),
            usage: usage(5),
        },
    );
    assert_eq!(fx.run().orchestrator_usage, usage(5));
    // A counter: persisted lazily and published as a counter update (decision 47).
    assert!(
        effects.contains(&Effect::Persist {
            run_id: RUN_ID.into(),
            urgent: false
        }),
        "{effects:?}"
    );
    // The ledger sends its whole total; the run keeps the latest.
    fx.send(
        now + 1,
        EventKind::OrchestratorUsage {
            run_id: RUN_ID.into(),
            usage: usage(7),
        },
    );
    assert_eq!(fx.run().orchestrator_usage, usage(7));
    // A run the engine does not know is ignored.
    let before = fx.state.revision;
    let effects = fx.send(
        now + 2,
        EventKind::OrchestratorUsage {
            run_id: "r-unknown".into(),
            usage: usage(9),
        },
    );
    assert_eq!(fx.state.revision, before);
    assert!(!effects.iter().any(|e| matches!(e, Effect::Persist { .. })));
}

/// M8b.15 review (I5), decision 30 clarified: within one daemon the ledger's total
/// replaces the stored one; across a restart the stored usage is a base the new
/// daemon's totals add to, so a restart never lowers it.
#[test]
fn a_restart_never_lowers_the_orchestrator_usage() {
    let (mut fx, _window) = working();
    fx.run_mut().orchestrator_usage = usage(999);
    let run = fx.run().clone();
    let mut fresh = Fixture::new(&fx.plan);
    fresh.next(EventKind::Restore {
        held: Vec::new(),
        runs: vec![run],
        replay: vec![],
    });
    assert_eq!(fresh.run().orchestrator_usage, usage(999));
    // Any local process may post a zero-valued point; the stored usage is unchanged.
    fresh.next(EventKind::OrchestratorUsage {
        run_id: RUN_ID.into(),
        usage: TokenUsage::default(),
    });
    assert_eq!(fresh.run().orchestrator_usage, usage(999));
    // The new session's totals add to the base, and still replace one another.
    fresh.next(EventKind::OrchestratorUsage {
        run_id: RUN_ID.into(),
        usage: usage(1),
    });
    assert_eq!(fresh.run().orchestrator_usage, usage(1000));
    fresh.next(EventKind::OrchestratorUsage {
        run_id: RUN_ID.into(),
        usage: usage(3),
    });
    assert_eq!(fresh.run().orchestrator_usage, usage(1002));
    // A second restart takes the stored usage as its base again.
    let run = fresh.run().clone();
    let mut again = Fixture::new(&fx.plan);
    again.next(EventKind::Restore {
        held: Vec::new(),
        runs: vec![run],
        replay: vec![],
    });
    again.next(EventKind::OrchestratorUsage {
        run_id: RUN_ID.into(),
        usage: usage(2),
    });
    assert_eq!(again.run().orchestrator_usage, usage(1004));
}

/// M8b.15 review (I5): base + total saturates.
#[test]
fn the_base_and_the_new_total_saturate() {
    let (mut fx, _window) = working();
    fx.run_mut().orchestrator_usage = usage(u64::MAX / 1000);
    let run = fx.run().clone();
    let mut fresh = Fixture::new(&fx.plan);
    fresh.next(EventKind::Restore {
        held: Vec::new(),
        runs: vec![run],
        replay: vec![],
    });
    let max = TokenUsage {
        input: u64::MAX,
        output: u64::MAX,
        cache_read: u64::MAX,
        cache_write: u64::MAX,
    };
    fresh.next(EventKind::OrchestratorUsage {
        run_id: RUN_ID.into(),
        usage: max,
    });
    assert_eq!(fresh.run().orchestrator_usage, max);
}

/// M8b.15 re-review (Important 1, defence in depth): a run that ended keeps the usage
/// it ended with; a late total, from a request that raced the end, changes nothing.
#[test]
fn a_run_that_ended_ignores_orchestrator_usage() {
    for ended in [
        proto::RunState::Accepted,
        proto::RunState::Discarded,
        proto::RunState::Failed,
    ] {
        let (mut fx, _window) = working();
        fx.next(EventKind::OrchestratorUsage {
            run_id: RUN_ID.into(),
            usage: usage(100),
        });
        assert_eq!(fx.run().orchestrator_usage, usage(100));
        fx.run_mut().state = ended;
        fx.next(EventKind::OrchestratorUsage {
            run_id: RUN_ID.into(),
            usage: usage(5),
        });
        assert_eq!(fx.run().orchestrator_usage, usage(100), "{ended:?}");
    }
    // A run that has not ended still takes the latest total.
    let (mut fx, _window) = working();
    fx.run_mut().state = proto::RunState::Complete;
    fx.next(EventKind::OrchestratorUsage {
        run_id: RUN_ID.into(),
        usage: usage(5),
    });
    assert_eq!(fx.run().orchestrator_usage, usage(5));
}
