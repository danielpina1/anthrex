//! Milestone 9.7 decision 16 (DH §4.2): an engine-made fix task for a culprit (9.1's
//! bisect fix and 9.2's CI culprit fix) escalates from the culprit's route as its
//! worker's rung 2 would (`role_step::escalate_for`): past a runtime the run's start
//! recorded as not installed, and past a route that failed in the culprit's task.
//! Milestone 9.8 (task M9.8.8): along the culprit's row (decision 29).

use proto::models::ModelRef;
use proto::{CiCategory, Route, Runtime, TaskState};

use super::bisect::{TEST, answer, merge_next, merged, red_full, with_orchestrator};
use super::delivery_ci::ci_fixes;
use super::delivery_ci_repro::{red_probe, red_summarised, tiered_watched};
use super::fixture::*;
use super::full::{full_job, outcome, tier};
use super::merge::{commit, doc_task, start_on};
use crate::run::engine::EventKind;
use crate::run::model_roles::{Mover, RunModels};
use crate::run::proof::proof_command;
use crate::run::role_step::escalate_for;
use crate::run::test_support::{set_row, task_toml, with_efforts};

const SINGLE: &str = "cargo test -- --exact {test}";
const CODEX: &str = "codex:default";
const OPUS: &str = "claude:claude-opus-5-5";
const SONNET: &str = "claude:claude-sonnet-5";

fn at(model: &str, effort: &str) -> Route {
    let model = ModelRef::parse(model).expect("a model");
    RunModels::route_of(&model, (!effort.is_empty()).then_some(effort))
}

/// A route's runtime, model and effort: a fix task's route takes its model's roster
/// strength, a row's route says `standard` (removed in M9.8.14).
fn key(route: &Route) -> (Runtime, String, String) {
    let effort = route.effort.as_str().to_string();
    (route.runtime, route.model.clone(), effort)
}

/// Milestone 9.8: `culprit`'s row set to `model` falling back to `fallback`, every row
/// model reporting `low`, `medium`, `high`; the culprit on `route`.
fn row(fx: &mut Fixture, culprit: &str, (model, fallback): (&str, &str), route: Route) {
    let role = RunModels::task_role(fx.task(culprit));
    set_row(fx.run_mut(), role, model, None, Some(fallback));
    with_efforts(fx.run_mut());
    fx.task_mut(culprit).route = route;
}

#[derive(Clone, Copy)]
enum Skip {
    /// The culprit at the top of its Codex row: `run.orch.installed` records Claude,
    /// its fallback's runtime, as `false`; the culprit's own route is left.
    Uninstalled,
    /// The culprit on Codex at `medium`: a session of it on Codex ended for an
    /// environment reason, so Codex's `high` is passed by for the Opus fallback.
    Failed,
}

/// Gives `culprit` a Codex row falling back to Opus and `skip`'s reason to pass by the
/// row's own step.
fn arrange(fx: &mut Fixture, culprit: &str, skip: Skip) {
    let (route, step) = match skip {
        Skip::Uninstalled => (at(CODEX, "high"), at(OPUS, "")),
        Skip::Failed => (at(CODEX, "medium"), at(CODEX, "high")),
    };
    row(fx, culprit, (CODEX, OPUS), route.clone());
    assert_eq!(
        fx.escalated(culprit, &route),
        step,
        "the premise: the row's step"
    );
    let want = match skip {
        Skip::Uninstalled => {
            fx.run_mut().orch.installed = [("claude".to_string(), false)].into();
            route.clone()
        }
        Skip::Failed => {
            let mut r = crate::run::orch::test_support::round(1, 0, Default::default());
            r.route = route.clone();
            r.ended = true;
            r.environment_failed = true;
            fx.task_mut(culprit).rounds.push(r);
            at(OPUS, "")
        }
    };
    // Review minor 3: the skip itself, up front, not a refusal and a fallback later.
    let i = (fx.run().tasks.iter())
        .position(|t| t.id() == culprit)
        .unwrap();
    let up = escalate_for(fx.run(), i, &route, Mover::Worker);
    assert_eq!(up, want, "the premise: escalate_for skips the step");
}

/// No fix task of the run was refused on its first route (the bisect's line).
fn none_refused(fx: &Fixture) {
    let refused = (fx.run().log.iter()).any(|e| e.text.starts_with("bisect fix for "));
    assert!(!refused, "{:#?}", fx.run().log);
}

/// A tier-3 red bisected to `t2`: its fix task's route.
fn bisect_fix_route(skip: Skip) -> Route {
    let mut fx = merged(&["t1", "t2", "t3"], "");
    with_orchestrator(&mut fx);
    arrange(&mut fx, "t2", skip);
    red_full(&mut fx);
    answer(&mut fx, 2);
    none_refused(&fx);
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
    assert_eq!(
        key(&bisect_fix_route(Skip::Uninstalled)),
        key(&at(CODEX, "high"))
    );
}

#[test]
fn a_ci_culprit_fix_task_skips_an_uninstalled_runtime() {
    assert_eq!(
        key(&ci_fix_route(Skip::Uninstalled)),
        key(&at(CODEX, "high"))
    );
}

#[test]
fn a_bisect_fix_task_skips_the_culprits_failed_route() {
    assert_eq!(key(&bisect_fix_route(Skip::Failed)), key(&at(OPUS, "")));
}

#[test]
fn a_ci_culprit_fix_task_skips_the_culprits_failed_route() {
    assert_eq!(key(&ci_fix_route(Skip::Failed)), key(&at(OPUS, "")));
}

/// The culprit on Sonnet at the top of its row, which falls back to Codex.
fn culprit_route() -> Route {
    at(SONNET, "high")
}

/// Makes `id` the one unfinished task that overlaps `culprit`'s `owns`: planned on the
/// culprit's runtime (its row's, Claude), but moved to Codex by rung 2 or `run retry`.
/// The overlap skip reads current routes, so it leaves Codex open; rule 9 reads planned
/// runtimes, so it refuses a Codex fix task.
fn moved_overlap(fx: &mut Fixture, id: &str, culprit: &str) {
    let owns = (fx.task(culprit).spec.owns.iter())
        .map(|g| g.replace("**", "more/**"))
        .collect();
    let t = fx.task_mut(id);
    t.spec.owns = owns;
    assert_eq!(t.spec.route.runtime, None, "planned on the default runtime");
    t.route = at(CODEX, "");
    if t.state.is_finished() {
        t.state = TaskState::Working;
    }
    let planned = RunModels::task_role(fx.task(id));
    assert_eq!(
        fx.run().limits.models().route(planned).runtime,
        Runtime::Claude
    );
    let i = (fx.run().tasks.iter())
        .position(|t| t.id() == culprit)
        .unwrap();
    assert_eq!(
        escalate_for(fx.run(), i, &culprit_route(), Mover::Worker),
        at(CODEX, ""),
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
    row(&mut fx, "t2", (SONNET, CODEX), culprit_route());
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
        key(&fx.task("fix1").route),
        key(&culprit_route()),
        "the culprit's own route"
    );
}

/// The same through `ci_culprit`: `t4`, still open, overlaps the culprit `t3`.
#[test]
fn a_ci_culprit_fix_task_refused_by_rule_9_falls_back_to_the_culprits_own_route() {
    let mut fx = tiered_watched(&["t1", "t2", "t3", "t4"]);
    with_orchestrator(&mut fx);
    row(&mut fx, "t3", (SONNET, CODEX), culprit_route());
    fx.run_mut().stages[0].full.green_at = Some(commit(1));
    red_summarised(&mut fx, &commit(4), &[TEST], CiCategory::Test);
    moved_overlap(&mut fx, "t4", "t3");
    let (op, _) = super::bisect::probe(&fx);
    fx.done(op, red_probe(&proof_command(SINGLE, TEST)));
    answer(&mut fx, 3);
    let fixes = ci_fixes(&fx);
    assert_eq!(fixes.len(), 1, "{fixes:?}");
    // FW-10 (review A m1): the spec names the culprit's runtime, model and effort (its
    // strength the roster's, M9.8.8), so the last resort (`RouteSpec::default()`, which
    // resolves to the row) can never pass for the fallback, whatever its effort.
    let culprit = culprit_route();
    let spec = proto::RouteSpec {
        runtime: Some(culprit.runtime),
        model: Some(culprit.model.clone()),
        strength: None,
        effort: Some(culprit.effort.clone()),
    };
    assert_eq!(
        fx.task(&fixes[0]).spec.route,
        spec,
        "the culprit's own route"
    );
    assert_eq!(
        key(&fx.task(&fixes[0]).route),
        key(&culprit),
        "the culprit's own route"
    );
}

/// M9.8.8 fix round 1 (review minor 2), then M9.8.13 (the follow-up from M9.8.8): a
/// culprit on a catalog model the old roster never listed. With `Run.roster` gone,
/// `validate::resolve_route` checks no roster membership, so the bisect fix takes the
/// step up along the culprit's row, and the run log names no refusal.
#[test]
fn a_bisect_fix_task_off_the_roster_takes_its_step_up() {
    let mut fx = merged(&["t1", "t2", "t3"], "");
    with_orchestrator(&mut fx);
    let sol = "codex:gpt-6-sol";
    row(&mut fx, "t2", (sol, "codex:gpt-6-luna"), at(sol, "medium"));
    red_full(&mut fx);
    answer(&mut fx, 2);
    assert_eq!(
        key(&fx.task("fix1").route),
        key(&at(sol, "high")),
        "the step up"
    );
    let refused = (fx.run().log.iter()).find(|e| e.text.starts_with("bisect fix for t2: "));
    assert!(refused.is_none(), "{:#?}", fx.run().log);
}
