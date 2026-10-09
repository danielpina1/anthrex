//! Route resolution at plan build (decisions 23, 39): the roster policy by class, the
//! default runtime, a given model's strength, and route errors. Split out of
//! `plan_tests.rs` to keep that file under the 600-line rule (F4).

use super::*;

fn route(runtime: Runtime, model: &str, effort: Effort) -> Route {
    Route {
        runtime,
        model: model.to_string(),
        effort,
    }
}

/// Milestone 9.8 decision 31: a plan file's route is ignored, so the routes these tests
/// resolve are a user's: `tasks` added by `run edit`'s `add_task` (`EditSource::User`,
/// decision 10) to a run of one other task.
fn users_tasks(tasks: &[String]) -> Result<crate::run::model::Run, Vec<PlanError>> {
    use crate::run::edits::apply_edits;
    use crate::run::orch::EditSource;
    use crate::run::validate::EditScope;
    let run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("base", "S", r#"["crates/z/src/lib.rs"]"#, "")],
    ));
    let plan = parse_plan(&plan_with(PROFILE, tasks)).expect("the tasks parse");
    let edits: Vec<proto::PlanEdit> = (plan.tasks.into_iter())
        .map(|task| proto::PlanEdit::AddTask { task })
        .collect();
    apply_edits(&run, &edits, &EditScope::Run, &EditSource::User, 2_000).map(|(run, _)| run)
}

/// Milestone 9.8 decision 10: each task's row (the built-ins here); every route a row
/// gives is `standard` until M9.8.14 removes the strength.
#[test]
fn policy_fills_routes_by_class() {
    let text = plan_with(
        PROFILE,
        &[
            task_toml("s", "S", r#"["crates/a/src/lib.rs"]"#, ""),
            task_toml("m", "M", r#"["crates/b/src/lib.rs"]"#, ""),
            task_toml("hub", "M", r#"["crates/proto/src/lib.rs"]"#, ""),
        ],
    );
    let run = run_ok(&text);

    assert_eq!(
        task(&run, "s").route,
        route(Runtime::Claude, "claude-sonnet-5", Effort::LOW)
    );
    assert_eq!(
        task(&run, "m").route,
        route(Runtime::Claude, "claude-sonnet-5", Effort::MEDIUM)
    );
    assert_eq!(
        task(&run, "hub").route,
        route(Runtime::Claude, "claude-opus-5-5", Effort::HIGH)
    );
    // Budgets by class: S the S budget, M and hub the M budget.
    let config = config::Orchestrator::default();
    assert_eq!(task(&run, "s").budget, config.budget_s);
    assert_eq!(task(&run, "m").budget, config.budget_m);
    assert_eq!(task(&run, "hub").budget, config.budget_m);
}

/// Milestone 9.8: `default_runtime` reaches a route through the role table its
/// migration writes (decision 14), so the config is read as a file is.
#[test]
fn default_runtime_comes_from_config() {
    let text = plan_with(
        PROFILE,
        &[task_toml("s", "S", r#"["crates/a/src/lib.rs"]"#, "")],
    );
    let (parsed, problems) = config::parse("[orchestrator]\ndefault_runtime = \"codex\"\n");
    assert!(problems.is_empty(), "{problems:?}");
    let config = parsed.orchestrator;
    let run = build_with(&text, &config).unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(
        task(&run, "s").route,
        route(Runtime::Codex, "", Effort::LOW)
    );
}

/// A user's route (milestone 9.8 decision 31; a plan's before). Task M9.8.13: the run
/// has no roster, so the named model is taken as named; no roster strength is looked up
/// for it, and none contradicts the route's (strength itself goes in M9.8.14).
#[test]
fn a_given_model_is_taken_as_named() {
    for extra in ["", "\nstrength = \"frontier\""] {
        let run = users_tasks(&[task_toml(
            "s",
            "S",
            r#"["crates/a/src/lib.rs"]"#,
            &format!("[task.route]\nmodel = \"claude-haiku-4-5\"{extra}"),
        )])
        .unwrap_or_else(|e| panic!("{}", show(&e)));
        // M9.8.11 fix round 1: no effort given on another model than the row's runs at
        // that model's default, not the row's `low`.
        let r = &task(&run, "s").route;
        assert_eq!(
            (r.runtime, r.model.as_str(), r.effort.clone()),
            (Runtime::Claude, "claude-haiku-4-5", Effort::DEFAULT),
            "{extra}"
        );
    }
}

/// A user's routes (milestone 9.8 decision 31; a plan's before). Task M9.8.13: a model
/// no roster lists (`a`) is no longer an error.
#[test]
fn route_problems_are_errors() {
    let errors = users_tasks(&[
        task_toml(
            "a",
            "S",
            r#"["crates/a/src/lib.rs"]"#,
            "[task.route]\nmodel = \"gpt-9\"",
        ),
        task_toml(
            "b",
            "S",
            r#"["crates/b/src/lib.rs"]"#,
            "[task.route]\nruntime = \"shell\"",
        ),
        // Milestone 9.8 decision 10: a route that names no model takes the row, so
        // a strength with no roster model at it is no longer an error.
        task_toml(
            "c",
            "S",
            r#"["crates/c/src/lib.rs"]"#,
            "[task.route]\nruntime = \"codex\"\nstrength = \"frontier\"",
        ),
    ])
    .unwrap_err();
    assert_eq!(
        errors,
        vec![err(
            Some("b"),
            "route.runtime",
            "route",
            "must be claude or codex"
        ),]
    );
}

/// Milestone 9.8 decision 31: a plan file's route is cleared before resolution, so each
/// task takes its row, and the run log says so once; a plan with no route logs nothing.
#[test]
fn a_plan_files_route_is_ignored_and_noted() {
    let text = plan_with(
        PROFILE,
        &[
            task_toml(
                "s",
                "S",
                r#"["crates/a/src/lib.rs"]"#,
                "[task.route]\nruntime = \"codex\"\nmodel = \"gpt-6-sol\"",
            ),
            task_toml(
                "m",
                "M",
                r#"["crates/b/src/lib.rs"]"#,
                "[task.route]\neffort = \"high\"",
            ),
        ],
    );
    let run = run_ok(&text);
    let models = run.limits.models();
    for id in ["s", "m"] {
        let t = task(&run, id);
        assert_eq!(t.spec.route, proto::RouteSpec::default(), "{id}");
        let row = models.route(crate::run::model_roles::RunModels::task_role(t));
        assert_eq!(t.route, row, "{id}");
    }
    let ignored: Vec<_> = (run.log.iter())
        .filter(|l| l.text == crate::run::orch::contract::ROUTE_IGNORED)
        .collect();
    assert_eq!(ignored.len(), 1, "{:?}", run.log);
    assert_eq!(
        run.log.last().unwrap().text,
        "route model ignored: models come from the role table"
    );

    let plain = plan_with(
        PROFILE,
        &[task_toml("s", "S", r#"["crates/a/src/lib.rs"]"#, "")],
    );
    let run = run_ok(&plain);
    assert!(
        !(run.log.iter()).any(|l| l.text == crate::run::orch::contract::ROUTE_IGNORED),
        "{:?}",
        run.log
    );
}

/// Fix round 1 (controller ruling): a plan file's route is ignored, so its values never
/// refuse the plan, even ones no route could hold. `run edit --file`'s, a user's, are
/// still read as before.
#[test]
fn a_plan_files_unknown_route_values_are_ignored_not_refused() {
    let text = plan_with(
        PROFILE,
        &[task_toml(
            "s",
            "S",
            r#"["crates/a/src/lib.rs"]"#,
            "[task.route]\nruntime = \"openai\"\nstrength = \"ultra\"\neffort = \"xhigh\"",
        )],
    );
    let plan = parse_plan_file(&text).unwrap_or_else(|e| panic!("{e}"));
    let run = build_with_plan(plan);
    assert_eq!(task(&run, "s").spec.route, proto::RouteSpec::default());
    assert_eq!(
        run.log.last().unwrap().text,
        crate::run::orch::contract::ROUTE_IGNORED
    );
    // A plan whose route is valid reads as before (the route cleared at build).
    let valid = text
        .replace("\"openai\"", "\"codex\"")
        .replace("\"ultra\"", "\"fast\"");
    assert_eq!(parse_plan_file(&valid), parse_plan(&valid));
    // A user's edit file is not lenient.
    let edit =
        "[[edit]]\nop = \"amend_task\"\ntask_id = \"s\"\n[edit.route]\nruntime = \"openai\"\n";
    assert!(parse_edits(edit).is_err());
}
