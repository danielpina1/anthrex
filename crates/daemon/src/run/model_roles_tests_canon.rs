//! Gate fix (2026-10-09), ruling B2: every same-model rule uses the run's
//! canonical model identity (`RunModels::same_model`), so a Claude alias the real CLI
//! lists (`opus[1m]`) is the row's `claude-opus-5-5`: the row's own model keeps the
//! row's effort, a failed alias rules out its full id, and the routing record does not
//! call the row's model another one.

use proto::models::{ModelRef, ModelTable, Role, RoleChoice};
use proto::{Effort, OrchestratorChoice, PlanEdit, RouteSpec, Runtime};

use super::tests_real::real_catalogs;
use super::*;
use crate::run::edits::apply_edits;
use crate::run::orch::EditSource;
use crate::run::test_support::{build_with, plan_with, show, task_toml};
use crate::run::validate::EditScope;

const PROFILE: &str = "goal = \"g\"\n\n[profile]\ncheck = \"true\"\nhub = [\"hub/**\"]\n";
const CHECK: &str = "test_mode = \"check\"\ntest_mode_reason = \"r\"";

fn opus_at_max(role: Role) -> ModelTable {
    let row = RoleChoice {
        model: ModelRef::parse("claude:claude-opus-5-5").unwrap(),
        effort: Some("max".into()),
        fallback: None,
    };
    ModelTable {
        rows: [(role, row)].into(),
        brainstorm: None,
    }
}

/// A run whose medium row is Opus at `max`, its table validated against the real
/// catalogs (as a run's start does), with one M task `m`.
fn run_on_opus() -> Run {
    let cfg = config::Orchestrator {
        roles: opus_at_max(Role::ImplementerMedium),
        ..config::Orchestrator::default()
    };
    let mut run = build_with(
        &plan_with(PROFILE, &[task_toml("m", "M", "[\"b/**\"]", CHECK)]),
        &cfg,
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    let models = run.limits.models.as_mut().expect("a frozen table");
    models.validate(&real_catalogs());
    run
}

fn amend_to(model: &str) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: "m".into(),
        brief: None,
        acceptance: None,
        route: Some(RouteSpec {
            runtime: Some(Runtime::Claude),
            model: Some(model.into()),
            strength: None,
            effort: None,
        }),
        test_mode: None,
        test_mode_reason: None,
        priority: None,
        size: None,
        deps: None,
        stage: None,
        race: None,
        pair: None,
    }
}

/// B2: a user's pick of the alias of the row's model, with no effort, keeps the row's.
#[test]
fn picking_the_rows_alias_keeps_the_rows_effort() {
    let run = run_on_opus();
    let (edited, _) = apply_edits(
        &run,
        &[amend_to("opus[1m]")],
        &EditScope::Run,
        &EditSource::User,
        5_000,
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    let route = &edited.tasks[0].route;
    assert_eq!(
        (route.model.as_str(), route.effort.as_str()),
        ("opus[1m]", "max")
    );
    // Another model still runs at its own default.
    let (edited, _) = apply_edits(
        &run,
        &[amend_to("sonnet")],
        &EditScope::Run,
        &EditSource::User,
        5_000,
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(edited.tasks[0].route.effort, Effort::DEFAULT);
}

/// B2: the goal form's choice of the orchestrator row's alias runs at the row's effort.
#[test]
fn the_orchestrator_choice_of_the_rows_alias_keeps_the_rows_effort() {
    let mut models = RunModels::resolve(&opus_at_max(Role::Orchestrator), None);
    models.validate(&real_catalogs());
    let choice = OrchestratorChoice {
        runtime: Runtime::Claude,
        model: Some("opus[1m]".into()),
        effort: None,
    };
    let resolved = crate::run::orch::launch::orchestrator_route(Some(&choice), &models);
    assert_eq!(
        (
            resolved.route.model.as_str(),
            resolved.route.effort.as_str()
        ),
        ("opus[1m]", "max")
    );
}

/// B2: a model that failed in the task under its alias failed under its full id too.
#[test]
fn failed_in_matches_an_alias_and_its_resolved_model() {
    let mut models = RunModels::resolve(&ModelTable::default(), None);
    models.validate(&real_catalogs());
    let at = |m: &str, e: Option<&str>| RunModels::route_of(&ModelRef::parse(m).unwrap(), e);
    let failed = [at("claude:opus[1m]", None)];
    assert!(failed_in(
        &models,
        &failed,
        &at("claude:claude-opus-5-5", Some("high"))
    ));
    assert!(!failed_in(&models, &failed, &at("claude:sonnet", None)));
}
