//! Milestone 9.7 decision 16 (DH §4.2): an engine-made fix task for a culprit (9.1's
//! bisect fix and 9.2's CI culprit fix) escalates from the culprit's route as its
//! worker's rung 2 would (`route_pick::escalate_for`): past a runtime the run's start
//! recorded as not installed, and past a route that failed in the culprit's task.

use proto::{CiCategory, Effort, ModelEntry, Route, Runtime, Strength, TaskState};

use super::bisect::{TEST, answer, merge_next, merged, red_full, with_orchestrator};
use super::delivery_ci::ci_fixes;
use super::delivery_ci_repro::{red_probe, red_summarised, tiered_watched};
use super::fixture::*;
use super::full::{full_job, outcome, tier};
use super::merge::{commit, doc_task, start_on};
use crate::run::engine::EventKind;
use crate::run::proof::proof_command;
use crate::run::roster::escalate;
use crate::run::route_pick::{Mover, escalate_for};
use crate::run::test_support::task_toml;

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
        effort: Effort::HIGH,
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

/// Makes `id` the one unfinished task that overlaps `culprit`'s `owns`: planned on the
/// culprit's runtime (its spec names none, so the default, Claude), but moved to Codex
/// by rung 2 or `run retry` ([`codex_step`]). The overlap skip reads current routes, so
/// it leaves Codex open; rule 9 reads planned runtimes, so it refuses a Codex fix task.
fn moved_overlap(fx: &mut Fixture, id: &str, culprit: &str) {
    let owns = (fx.task(culprit).spec.owns.iter())
        .map(|g| g.replace("**", "more/**"))
        .collect();
    let t = fx.task_mut(id);
    t.spec.owns = owns;
    assert_eq!(t.spec.route.runtime, None, "planned on the default runtime");
    assert!(t.list_pick.is_none());
    t.route = codex_step();
    if t.state.is_finished() {
        t.state = TaskState::Working;
    }
    assert_eq!(fx.run().limits.default_runtime, Runtime::Claude);
    let at = (fx.run().tasks.iter())
        .position(|t| t.id() == culprit)
        .unwrap();
    assert_eq!(
        escalate_for(fx.run(), at, &culprit_route(), Mover::Worker),
        codex_step(),
        "the premise: the overlap skip leaves Codex open"
    );
}

/// Review of task M9.7.13 (Important 1): the fix task is refused on the step
/// `escalate_for` gives, and falls back to the culprit's own route.
#[test]
fn a_bisect_fix_task_refused_by_rule_9_falls_back_to_the_culprits_own_route() {
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", ""),
        task_toml(
            "t3",
            "S",
            "[\"docs/t3/**\"]",
            "test_mode = \"check\"\ntest_mode_reason = \"glue code\"",
        ),
    ];
    let (mut fx, mut windows) = start_on(&super::full::profile(), &tasks);
    fx.run_mut().roster = roster();
    fx.task_mut("t2").route = culprit_route();
    merge_next(&mut fx, &mut windows, "t1", &commit(1));
    merge_next(&mut fx, &mut windows, "t2", &commit(2));
    moved_overlap(&mut fx, "t3", "t2");
    assert!(!fx.task("t3").state.is_finished());
    let since = fx.run().queue_idle_since.expect("idle");
    fx.send(since + 120, EventKind::Tick);
    let (op, _) = full_job(&fx);
    fx.done(op, tier(outcome(3, &[TEST])));
    answer(&mut fx, 2);
    assert_eq!(
        fx.task("fix1").route,
        culprit_route(),
        "the culprit's own route"
    );
}

/// The same through `ci_culprit`: `t4`, still open, overlaps the culprit `t3`.
#[test]
fn a_ci_culprit_fix_task_refused_by_rule_9_falls_back_to_the_culprits_own_route() {
    let mut fx = tiered_watched(&["t1", "t2", "t3", "t4"]);
    with_orchestrator(&mut fx);
    fx.run_mut().roster = roster();
    fx.task_mut("t3").route = culprit_route();
    fx.run_mut().stages[0].full.green_at = Some(commit(1));
    red_summarised(&mut fx, &commit(4), &[TEST], CiCategory::Test);
    moved_overlap(&mut fx, "t4", "t3");
    let (op, _) = super::bisect::probe(&fx);
    fx.done(op, red_probe(&proof_command(SINGLE, TEST)));
    answer(&mut fx, 3);
    let fixes = ci_fixes(&fx);
    assert_eq!(fixes.len(), 1, "{fixes:?}");
    // FW-10 (review A m1): the spec names the culprit's runtime, model, strength and
    // effort, so the last resort (`RouteSpec::default()`, which resolves to the same
    // runtime and model) can never pass for the fallback, whatever its effort.
    let culprit = culprit_route();
    let spec = proto::RouteSpec {
        runtime: Some(culprit.runtime),
        model: Some(culprit.model.clone()),
        strength: Some(culprit.strength),
        effort: Some(culprit.effort.clone()),
    };
    assert_eq!(
        fx.task(&fixes[0]).spec.route,
        spec,
        "the culprit's own route"
    );
    assert_eq!(fx.task(&fixes[0]).route, culprit, "the culprit's own route");
}
