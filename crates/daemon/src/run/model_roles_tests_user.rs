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
    let mut cfg = config::Orchestrator {
        roles: ModelTable {
            rows: [(Role::ImplementerMedium, row)].into(),
            brainstorm: None,
        },
        ..config::Orchestrator::default()
    };
    if !cfg.models.iter().any(|m| m.model == "gpt-6-luna") {
        cfg.models.push(proto::ModelEntry {
            runtime: Runtime::Codex,
            model: "gpt-6-luna".into(),
            strength: proto::Strength::Fast,
            note: String::new(),
        });
    }
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
}
