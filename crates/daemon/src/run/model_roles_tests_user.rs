//! Milestone 9.8 M9.8.11 fix round 1 (controller ruling I2): a user's route naming a
//! model other than its row's, with no effort, runs at that model's default effort,
//! never at the row's (which that model may not offer); naming the row's own model it
//! keeps the row's effort, as the goal form's orchestrator choice does.

use proto::models::{ModelRef, ModelTable, Role, RoleChoice};
use proto::{Effort, PlanEdit, RouteSpec, Runtime};

use crate::run::edits::apply_edits;
use crate::run::orch::EditSource;
use crate::run::test_support::{build_with, plan_with, show, task_toml};
use crate::run::validate::EditScope;

const PROFILE: &str = "goal = \"g\"\n\n[profile]\ncheck = \"true\"\nhub = [\"hub/**\"]\n";
const CHECK: &str = "test_mode = \"check\"\ntest_mode_reason = \"r\"";

fn amend(runtime: Runtime, model: &str) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: "m".into(),
        brief: None,
        acceptance: None,
        route: Some(RouteSpec {
            runtime: Some(runtime),
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

#[test]
fn a_users_model_without_effort_does_not_take_the_rows_effort() {
    let row = RoleChoice {
        model: ModelRef::parse("claude:claude-opus-5-5").unwrap(),
        effort: Some("max".into()),
        fallback: None,
    };
    let cfg = config::Orchestrator {
        roles: ModelTable {
            rows: [(Role::ImplementerMedium, row)].into(),
            brainstorm: None,
        },
        ..config::Orchestrator::default()
    };
    let run = build_with(
        &plan_with(PROFILE, &[task_toml("m", "M", "[\"b/**\"]", CHECK)]),
        &cfg,
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(run.tasks[0].route.effort.as_str(), "max", "the row's");
    let edit = |runtime, model| {
        let (edited, _) = apply_edits(
            &run,
            &[amend(runtime, model)],
            &EditScope::Run,
            &EditSource::User,
            5_000,
        )
        .unwrap_or_else(|e| panic!("{}", show(&e)));
        edited.tasks[0].route.clone()
    };
    let luna = edit(Runtime::Codex, "gpt-6-luna");
    assert_eq!(luna.model, "gpt-6-luna");
    assert_eq!(luna.effort, Effort::DEFAULT, "max never reaches luna");
    let own = edit(Runtime::Claude, "claude-opus-5-5");
    assert_eq!(
        own.effort.as_str(),
        "max",
        "the row's own model keeps its effort"
    );

    // At the gate the snapshot names the task's row, so the task edit form can say
    // which effort a pick of the row's model runs at.
    assert_eq!(run.state, proto::RunState::AwaitingApproval);
    let mut state = crate::run::engine::EngineState::default();
    state.runs.insert(run.id.clone(), run.clone());
    let info = &crate::run::snapshot::snapshot(&state, 0).runs[0].tasks[0];
    let row = info.row.as_ref().expect("the task's row");
    assert_eq!(
        (row.runtime, row.model.as_str(), row.effort.as_str()),
        (Runtime::Claude, "claude-opus-5-5", "max")
    );
}

/// Milestone 9.8 task M9.8.13 (the follow-up from M9.8.8): with `Run.roster` gone, a
/// user's route naming a model no roster lists is accepted as given: no roster
/// membership, and no roster strength checked against it. No catalog is consulted
/// either; the only check is the name rule (fix round 1, I1).
#[test]
fn a_users_route_to_a_model_no_roster_lists_is_accepted() {
    let cfg = config::Orchestrator::default();
    let run = build_with(
        &plan_with(PROFILE, &[task_toml("m", "M", "[\"b/**\"]", CHECK)]),
        &cfg,
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    let mut edit = amend(Runtime::Codex, "gpt-9-new");
    if let PlanEdit::AmendTask { route: Some(r), .. } = &mut edit {
        r.effort = Some(Effort::HIGH);
    }
    let (edited, _) = apply_edits(&run, &[edit], &EditScope::Run, &EditSource::User, 5_000)
        .unwrap_or_else(|e| panic!("{}", show(&e)));
    let route = &edited.tasks[0].route;
    assert_eq!(
        (route.runtime, route.model.as_str(), route.effort.as_str()),
        (Runtime::Codex, "gpt-9-new", "high")
    );
}

/// The user's amend of task `m` to Codex model `model`, from the built-in table's run:
/// the run's refusal, or the route it took.
fn amended_to(model: &str) -> Result<proto::Route, String> {
    let run = build_with(
        &plan_with(PROFILE, &[task_toml("m", "M", "[\"b/**\"]", CHECK)]),
        &config::Orchestrator::default(),
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    apply_edits(
        &run,
        &[amend(Runtime::Codex, model)],
        &EditScope::Run,
        &EditSource::User,
        5_000,
    )
    .map(|(edited, _)| edited.tasks[0].route.clone())
    .map_err(|e| show(&e))
}

/// M9.8.13 fix round 1 (I1): a user's model must be a model name (`ModelRef`'s rule:
/// 1 to 100 visible characters, no whitespace, control or hidden-format character, no
/// leading `-`), refused with `route.model` named, so it never reaches `-m`/`--model`.
fn refused(model: &str) {
    let error = amended_to(model).expect_err(model);
    assert!(error.contains("route.model"), "{model:?}: {error}");
    assert!(error.contains("is not a model name"), "{model:?}: {error}");
}

#[test]
fn a_users_model_with_whitespace_is_refused() {
    refused("gpt 6");
    refused("   ");
}

#[test]
fn a_users_model_with_a_control_character_is_refused() {
    refused("gpt-6\n--x");
    refused("gpt\u{7}6");
}

#[test]
fn a_users_model_with_a_hidden_format_character_is_refused() {
    refused("gpt\u{200b}6");
}

#[test]
fn a_users_model_over_100_characters_is_refused() {
    refused(&"g".repeat(101));
    assert!(amended_to(&"g".repeat(100)).is_ok(), "100 is allowed");
}

#[test]
fn a_users_model_with_a_leading_dash_is_refused() {
    refused("-gpt-6");
    refused("--model");
}

/// `""` is the runtime's default model, as a route spells it.
#[test]
fn a_users_empty_model_is_the_runtimes_default() {
    let route = amended_to("").expect("the default model");
    assert_eq!((route.runtime, route.model.as_str()), (Runtime::Codex, ""));
}
