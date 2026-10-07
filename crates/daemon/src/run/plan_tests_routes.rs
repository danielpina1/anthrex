//! Route resolution at plan build (decisions 23, 39): the roster policy by class, the
//! default runtime, a given model's strength, and route errors. Split out of
//! `plan_tests.rs` to keep that file under the 600-line rule (F4).

use super::*;

fn route(runtime: Runtime, model: &str, strength: Strength, effort: Effort) -> Route {
    Route {
        runtime,
        model: model.to_string(),
        strength,
        effort,
    }
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
        route(
            Runtime::Claude,
            "claude-sonnet-5",
            Strength::Standard,
            Effort::LOW
        )
    );
    assert_eq!(
        task(&run, "m").route,
        route(
            Runtime::Claude,
            "claude-sonnet-5",
            Strength::Standard,
            Effort::MEDIUM
        )
    );
    assert_eq!(
        task(&run, "hub").route,
        route(
            Runtime::Claude,
            "claude-opus-5-5",
            Strength::Standard,
            Effort::HIGH
        )
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
        route(Runtime::Codex, "", Strength::Standard, Effort::LOW)
    );
}

#[test]
fn a_given_model_fixes_the_strength() {
    let text = plan_with(
        PROFILE,
        &[task_toml(
            "s",
            "S",
            r#"["crates/a/src/lib.rs"]"#,
            "[task.route]\nmodel = \"claude-haiku-4-5\"",
        )],
    );
    let run = run_ok(&text);
    // S policy would give standard; the model is fast in the roster, so fast it is.
    assert_eq!(
        task(&run, "s").route,
        route(
            Runtime::Claude,
            "claude-haiku-4-5",
            Strength::Fast,
            Effort::LOW
        )
    );
}

#[test]
fn a_contradicting_strength_is_an_error() {
    let text = plan_with(
        PROFILE,
        &[task_toml(
            "s",
            "S",
            r#"["crates/a/src/lib.rs"]"#,
            "[task.route]\nmodel = \"claude-sonnet-5\"\nstrength = \"frontier\"",
        )],
    );
    assert_eq!(
        errors_of(&text),
        vec![err(
            Some("s"),
            "route.strength",
            "route",
            "claude-sonnet-5 is standard in the roster, not frontier"
        )]
    );
}

#[test]
fn route_problems_are_errors() {
    let text = plan_with(
        PROFILE,
        &[
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
        ],
    );
    assert_eq!(
        errors_of(&text),
        vec![
            err(
                Some("a"),
                "route.model",
                "route",
                "gpt-9 is not in the roster for claude"
            ),
            err(
                Some("b"),
                "route.runtime",
                "route",
                "must be claude or codex"
            ),
        ]
    );
}
