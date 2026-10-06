//! Milestone 9.7 decision 16 (DH §4.2): an engine-made fix task for a culprit (9.1's
//! bisect fix and 9.2's CI culprit fix) escalates from the culprit's route as its
//! worker's rung 2 would (`route_pick::escalate_for`): past a runtime the run's start
//! recorded as not installed, and past a route that failed in the culprit's task.

use proto::{CiCategory, Effort, ModelEntry, Route, Runtime, Strength};

use super::bisect::{TEST, answer, merged, red_full, with_orchestrator};
use super::delivery_ci::ci_fixes;
use super::delivery_ci_repro::{red_probe, red_summarised, tiered_watched};
use super::fixture::*;
use super::merge::commit;
use crate::run::proof::proof_command;
use crate::run::roster::escalate;

const SINGLE: &str = "cargo test -- --exact {test}";

fn entry(runtime: Runtime, model: &str, strength: Strength) -> ModelEntry {
    ModelEntry {
        runtime,
        model: model.to_string(),
        strength,
        note: String::new(),
    }
}

fn route(runtime: Runtime, model: &str, strength: Strength) -> Route {
    Route {
        runtime,
        model: model.to_string(),
        strength,
        effort: Effort::High,
    }
}

/// Claude and Codex at `standard` and `frontier`.
fn roster() -> Vec<ModelEntry> {
    vec![
        entry(Runtime::Claude, "claude-std", Strength::Standard),
        entry(Runtime::Claude, "claude-top", Strength::Frontier),
        entry(Runtime::Codex, "codex-std", Strength::Standard),
        entry(Runtime::Codex, "codex-top", Strength::Frontier),
    ]
}

/// The culprit's route, `claude/standard` at `high`.
fn culprit_route() -> Route {
    route(Runtime::Claude, "claude-std", Strength::Standard)
}

/// The roster's step from the culprit's route: the peer runtime at the same strength.
fn codex_step() -> Route {
    route(Runtime::Codex, "codex-std", Strength::Standard)
}

/// The next installed, unfailed step: Claude one strength up.
fn claude_up() -> Route {
    route(Runtime::Claude, "claude-top", Strength::Frontier)
}

#[derive(Clone, Copy)]
enum Skip {
    /// `run.orch.installed` records Codex as `false`.
    Uninstalled,
    /// A session of the culprit ended for an environment reason on the Codex step.
    Failed,
}

/// Gives `fx` [`roster`], `culprit` [`culprit_route`], and `skip`'s reason to pass by
/// the roster's own step ([`codex_step`]).
fn arrange(fx: &mut Fixture, culprit: &str, skip: Skip) {
    fx.run_mut().roster = roster();
    fx.task_mut(culprit).route = culprit_route();
    assert_eq!(
        escalate(&fx.run().roster, &culprit_route()),
        codex_step(),
        "the premise: the roster's step is Codex"
    );
    match skip {
        Skip::Uninstalled => fx.run_mut().orch.installed = [("codex".to_string(), false)].into(),
        Skip::Failed => {
            let mut r = crate::run::orch::test_support::round(1, 0, Default::default());
            r.route = codex_step();
            r.ended = true;
            r.environment_failed = true;
            fx.task_mut(culprit).rounds.push(r);
        }
    }
}

/// A tier-3 red bisected to `t2`: its fix task's route.
fn bisect_fix_route(skip: Skip) -> Route {
    let mut fx = merged(&["t1", "t2", "t3"], "");
    with_orchestrator(&mut fx);
    arrange(&mut fx, "t2", skip);
    red_full(&mut fx);
    answer(&mut fx, 2);
    fx.task("fix1").route.clone()
}

/// A reproduced CI red bisected to `t3`: its CI fix task's route.
fn ci_fix_route(skip: Skip) -> Route {
    let mut fx = tiered_watched(&["t1", "t2", "t3", "t4"]);
    with_orchestrator(&mut fx);
    arrange(&mut fx, "t3", skip);
    fx.run_mut().stages[0].full.green_at = Some(commit(1));
    red_summarised(&mut fx, &commit(4), &[TEST], CiCategory::Test);
    let (op, _) = super::bisect::probe(&fx);
    fx.done(op, red_probe(&proof_command(SINGLE, TEST)));
    answer(&mut fx, 3);
    let fixes = ci_fixes(&fx);
    assert_eq!(fixes.len(), 1, "{fixes:?}");
    fx.task(&fixes[0]).route.clone()
}

#[test]
fn a_bisect_fix_task_skips_an_uninstalled_runtime() {
    assert_eq!(bisect_fix_route(Skip::Uninstalled), claude_up());
}

#[test]
fn a_ci_culprit_fix_task_skips_an_uninstalled_runtime() {
    assert_eq!(ci_fix_route(Skip::Uninstalled), claude_up());
}

#[test]
fn a_bisect_fix_task_skips_the_culprits_failed_route() {
    assert_eq!(bisect_fix_route(Skip::Failed), claude_up());
}

#[test]
fn a_ci_culprit_fix_task_skips_the_culprits_failed_route() {
    assert_eq!(ci_fix_route(Skip::Failed), claude_up());
}
