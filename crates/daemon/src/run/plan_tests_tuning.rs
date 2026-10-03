//! Milestone 9.5 task 9: what a start freezes from history (decision 12) reaches the
//! built run: budgets by decision 6's precedence, the hub budget, the class routes, and
//! `Tuned::default()` changing nothing.

use proto::{Budget, ClassBudget, ClassRoute, Effort, SizeThresholds, Strength, TuningFile};

use super::*;
use crate::run::model::{ClassRoutes, Run};
use crate::run::refit::{Tuned, tuned};

fn budget(tool_calls: u32, minutes: u32) -> Budget {
    Budget {
        tool_calls,
        minutes,
        tokens: None,
    }
}

/// A tuning file whose `class` budget is `calls`/`minutes`.
fn refit_file(budgets: &[(&str, u32, u32)]) -> TuningFile {
    let mut file = TuningFile::default();
    for &(class, tool_calls, minutes) in budgets {
        file.budgets.insert(
            class.into(),
            ClassBudget {
                tool_calls,
                minutes,
                tokens: None,
                samples: 34,
                at: 1_790_500_000,
            },
        );
    }
    file
}

fn plan(tasks: &[String]) -> String {
    plan_with(PROFILE, tasks)
}

fn built(text: &str, config: &config::Orchestrator, tuning: Tuned) -> Run {
    build_tuned(text, config, tuning).unwrap_or_else(|e| panic!("{}", show(&e)))
}

#[test]
fn tuned_budgets_win_over_defaults_and_lose_to_the_plan_and_config() {
    let config = config::Orchestrator::default();
    let file = refit_file(&[("s", 55, 18)]);
    let text = plan(&[
        task_toml("t1", "S", "[\"crates/a/src/x.rs\"]", ""),
        task_toml(
            "t2",
            "S",
            "[\"crates/b/src/y.rs\"]",
            "[task.budget]\ntool_calls = 12\nminutes = 7",
        ),
    ]);
    let run = built(&text, &config, tuned(&file, &config));
    assert_eq!(task(&run, "t1").budget, budget(55, 18), "the refit");
    assert_eq!(task(&run, "t2").budget, budget(12, 7), "the plan's own");
    assert_eq!(run.limits.budget_s, budget(55, 18));
    assert_eq!(run.limits.budget_m, config.budget_m);
    let log: Vec<&str> = run.log.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(log, ["tuning: budget S 55 calls 18m from 34 samples"]);
    assert!(run.log.iter().all(|e| e.at == 1_000));

    // Ruling RH-5: an explicit `[orchestrator.budget.s]` beats the refit.
    let mut configured = config::Orchestrator {
        budget_s: budget(30, 10),
        ..Default::default()
    };
    configured.tuning.configured.s = true;
    let run = built(&text, &configured, tuned(&file, &configured));
    assert_eq!(task(&run, "t1").budget, budget(30, 10));
    assert_eq!(run.limits.budget_s, budget(30, 10));
    assert!(run.limits.budget_configured.s && !run.limits.budget_configured.m);
    assert_eq!(
        run.log[0].text,
        "tuning: budget S 30 calls 10m configured (refit would be 55 calls 18m)"
    );
}

#[test]
fn hub_uses_its_own_budget_else_m() {
    let config = config::Orchestrator::default();
    let text = plan(&[
        task_toml("h", "M", "[\"crates/proto/src/wire.rs\"]", ""),
        task_toml("m", "M", "[\"crates/a/src/x.rs\"]", ""),
    ]);
    let both = refit_file(&[("m", 200, 90), ("hub", 160, 70)]);
    let run = built(&text, &config, tuned(&both, &config));
    assert!(task(&run, "h").hub);
    assert_eq!(task(&run, "h").budget, budget(160, 70));
    assert_eq!(task(&run, "m").budget, budget(200, 90));
    assert_eq!(run.limits.budget_hub, Some(budget(160, 70)));

    let m_only = refit_file(&[("m", 200, 90)]);
    let run = built(&text, &config, tuned(&m_only, &config));
    assert_eq!(run.limits.budget_hub, None);
    assert_eq!(
        task(&run, "h").budget,
        budget(200, 90),
        "M's effective budget"
    );

    // An explicit M holds hub tasks too, whatever the hub refit says.
    let mut configured = config::Orchestrator::default();
    configured.tuning.configured.m = true;
    let run = built(&text, &configured, tuned(&both, &configured));
    assert_eq!(task(&run, "h").budget, configured.budget_m);
    assert_eq!(task(&run, "m").budget, configured.budget_m);
}

#[test]
fn class_routes_fill_the_policy() {
    let config = config::Orchestrator::default();
    let route = |strength, effort| ClassRoute { strength, effort };
    let mut file = TuningFile::default();
    file.routes
        .insert("s".into(), route(Strength::Standard, Effort::Medium));
    file.routes
        .insert("m".into(), route(Strength::Frontier, Effort::Medium));
    let text = plan(&[
        task_toml("s1", "S", "[\"crates/a/src/x.rs\"]", ""),
        task_toml("m1", "M", "[\"crates/b/src/y.rs\"]", ""),
        task_toml("h1", "M", "[\"crates/proto/src/wire.rs\"]", ""),
        // An explicit route keeps its own effort.
        task_toml(
            "e1",
            "S",
            "[\"crates/c/src/z.rs\"]",
            "[task.route]\nruntime = \"claude\"\neffort = \"high\"",
        ),
    ]);
    let run = built(&text, &config, tuned(&file, &config));
    let of = |id: &str| {
        let r = &task(&run, id).route;
        (r.strength, r.effort)
    };
    assert_eq!(of("s1"), (Strength::Standard, Effort::Medium));
    assert_eq!(of("m1"), (Strength::Frontier, Effort::Medium));
    assert_eq!(
        of("h1"),
        (Strength::Frontier, Effort::High),
        "hub is never tuned"
    );
    assert_eq!(of("e1").1, Effort::High);
    assert_eq!(
        run.limits.class_routes,
        ClassRoutes {
            s: route(Strength::Standard, Effort::Medium),
            m: route(Strength::Frontier, Effort::Medium),
            ..ClassRoutes::default()
        }
    );
    let log: Vec<&str> = run.log.iter().map(|e| e.text.as_str()).collect();
    assert_eq!(
        log,
        [
            "tuning: route S standard/medium (applied)",
            "tuning: route M frontier/medium (applied)"
        ]
    );
}

#[test]
fn default_tuning_reproduces_today() {
    let config = config::Orchestrator::default();
    let today = build_with(EXAMPLE_PLAN, &config).unwrap_or_else(|e| panic!("{}", show(&e)));
    // `build_with` builds with `Tuned::default()` too, so the pin is the limits below,
    // `none == today` and `the_brief_example_builds_a_run`, not a comparison of the two.
    let run = built(EXAMPLE_PLAN, &config, Tuned::default());
    assert!(run.log.is_empty());
    // What a start with nothing learned freezes is the same run, but for its log line.
    let mut none = built(
        EXAMPLE_PLAN,
        &config,
        tuned(&TuningFile::default(), &config),
    );
    let log: Vec<String> = none.log.drain(..).map(|e| e.text).collect();
    assert_eq!(
        log,
        ["tuning: none (history has fewer than 30 samples per class)"]
    );
    assert_eq!(none, today);
    let limits = &run.limits;
    assert_eq!(
        (limits.budget_s, limits.budget_m, limits.budget_l),
        (config.budget_s, config.budget_m, config.budget_l)
    );
    assert_eq!(limits.budget_hub, None);
    assert_eq!(limits.class_routes, ClassRoutes::default());
    assert_eq!(limits.path_weights, None);
    assert_eq!(limits.thresholds, SizeThresholds::default());
    // A run with nothing tuned writes none of the new limit keys.
    let json = serde_json::to_value(limits).unwrap();
    for key in [
        "budget_hub",
        "budget_configured",
        "class_routes",
        "path_weights",
        "thresholds",
    ] {
        assert!(json.get(key).is_none(), "{key}: {json}");
    }
}
