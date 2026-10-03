//! `[orchestrator.tuning]`, the explicit budgets and the `[orchestrator.routes.*]` model
//! lists (milestone 9.5 task M9.5.7).

use proto::{Effort, Runtime, Strength};

use super::routes::{Candidate, Pick, RouteList};
use super::*;
use crate::settings::SHIPPED_CODEX;
use crate::{Problem, parse};

fn tuning_of(text: &str) -> (TuningConfig, Vec<Problem>) {
    let (config, problems) = parse(text);
    (config.orchestrator.tuning, problems)
}

fn problem(key: &str, message: &str, default: &str) -> Problem {
    Problem {
        key: key.to_string(),
        message: message.to_string(),
        default: default.to_string(),
    }
}

fn candidate(runtime: Runtime, model: &str, effort: Option<Effort>) -> Candidate {
    Candidate {
        runtime,
        model: model.to_string(),
        effort,
    }
}

/// `[[orchestrator.models]]` entries for the named shipped Codex models, so the roster
/// holds them (the built-in roster has only Codex's default).
fn shipped_codex_models(names: &[&str]) -> String {
    let mut text = String::new();
    for name in names {
        let shipped = SHIPPED_CODEX
            .iter()
            .find(|s| s.model == *name)
            .expect("a shipped Codex model");
        let strength = match shipped.strength {
            Strength::Fast => "fast",
            Strength::Standard => "standard",
            Strength::Frontier => "frontier",
        };
        text.push_str(&format!(
            "[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"{}\"\nstrength = \"{strength}\"\n\n",
            shipped.model
        ));
    }
    text
}

#[test]
fn defaults_when_absent() {
    let (t, problems) = tuning_of("");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        t.table,
        Tuning {
            min_samples: 30,
            per_run_cap: 10,
            window: 200,
            budget_factor_percent: 250,
            threshold_percentile: 90,
            min_change_percent: 20,
            escalate_above_percent: 25,
            refit_budgets: true,
            refit_tokens: false,
            path_weights: true,
            adaptive_concurrency: true,
            recover_after_mins: 10,
            halve_hold_secs: 60,
            race_slot_wait_secs: 120,
        }
    );
    let lists = &t.routes;
    for list in [
        &lists.s,
        &lists.m,
        &lists.hub,
        &lists.review,
        &lists.scout,
        &lists.decider,
        &lists.planner,
        &lists.orchestrator,
    ] {
        assert!(list.candidates.is_empty());
        assert_eq!(list.pick, Pick::First);
    }
    assert_eq!(t.configured, ConfiguredBudgets { s: false, m: false });
}

#[test]
fn every_key_is_read() {
    let (t, problems) = tuning_of(
        r#"
[orchestrator.tuning]
min_samples = 40
per_run_cap = 5
window = 300
budget_factor_percent = 200
threshold_percentile = 75
min_change_percent = 10
escalate_above_percent = 50
refit_budgets = false
refit_tokens = true
path_weights = false
adaptive_concurrency = false
recover_after_mins = 30
halve_hold_secs = 0
race_slot_wait_secs = 600
"#,
    );
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        t.table,
        Tuning {
            min_samples: 40,
            per_run_cap: 5,
            window: 300,
            budget_factor_percent: 200,
            threshold_percentile: 75,
            min_change_percent: 10,
            escalate_above_percent: 50,
            refit_budgets: false,
            refit_tokens: true,
            path_weights: false,
            adaptive_concurrency: false,
            recover_after_mins: 30,
            halve_hold_secs: 0,
            race_slot_wait_secs: 600,
        }
    );
}

#[test]
fn out_of_range_values_keep_defaults_with_problems() {
    for (key, value) in [
        ("min_samples", "4"),
        ("budget_factor_percent", "199"),
        ("budget_factor_percent", "301"),
        ("threshold_percentile", "100"),
        ("recover_after_mins", "0"),
    ] {
        let (t, problems) = tuning_of(&format!("[orchestrator.tuning]\n{key} = {value}\n"));
        assert_eq!(problems.len(), 1, "{key} = {value}: {problems:?}");
        assert_eq!(problems[0].key, format!("orchestrator.tuning.{key}"));
        assert_eq!(t.table, Tuning::default(), "{key} = {value}");
    }

    let (_, problems) = tuning_of("[orchestrator.tuning]\nbudget_factor_percent = 199\n");
    assert_eq!(
        problems,
        vec![problem(
            "orchestrator.tuning.budget_factor_percent",
            "must be between 200 and 300",
            "250",
        )]
    );
    assert_eq!(
        problems[0].to_string(),
        "orchestrator.tuning.budget_factor_percent: must be between 200 and 300 (using 250)"
    );

    let (_, problems) = tuning_of("[orchestrator.tuning]\nmin_samples = 4\n");
    assert_eq!(
        problems[0].to_string(),
        "orchestrator.tuning.min_samples: must be between 5 and 1000 (using 30)"
    );
}

#[test]
fn window_below_min_samples_uses_min_samples() {
    let (t, problems) = tuning_of("[orchestrator.tuning]\nmin_samples = 50\nwindow = 20\n");
    assert_eq!(t.table.min_samples, 50);
    assert_eq!(t.table.window, 50);
    assert_eq!(
        problems,
        vec![problem(
            "orchestrator.tuning.window",
            "must be at least min_samples (50)",
            "50",
        )]
    );

    // Equal is fine.
    let (t, problems) = tuning_of("[orchestrator.tuning]\nmin_samples = 50\nwindow = 50\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(t.table.window, 50);
}

#[test]
fn unknown_tuning_keys_are_reported() {
    let (t, problems) = tuning_of("[orchestrator.tuning]\nrace_default = true\nwindow = 100\n");
    assert_eq!(
        problems,
        vec![problem(
            "orchestrator.tuning.race_default",
            "unknown key, ignored",
            "nothing",
        )]
    );
    assert_eq!(
        problems[0].to_string(),
        "orchestrator.tuning.race_default: unknown key, ignored (using nothing)"
    );
    assert_eq!(t.table.window, 100);
}

#[test]
fn an_explicit_budget_is_configured() {
    let (t, problems) = tuning_of("[orchestrator.budget.s]\ntool_calls = 40\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(t.configured, ConfiguredBudgets { s: true, m: false });

    let (t, _) = tuning_of("[orchestrator.budget.m]\nminutes = 60\n");
    assert_eq!(t.configured, ConfiguredBudgets { s: false, m: true });

    // The default values, without the table, are not explicit.
    let (config, _) = parse("");
    assert_eq!(config.orchestrator.budget_s.tool_calls, 40);
    assert_eq!(
        config.orchestrator.tuning.configured,
        ConfiguredBudgets::default()
    );

    // Neither is an empty table, as the Settings screen leaves when it deletes a
    // value that went back to its default, nor `[orchestrator.budget.l]`.
    let (t, _) = tuning_of(
        "[orchestrator.budget.s]\n[orchestrator.budget.m]\n[orchestrator.budget.l]\ntool_calls = 500\n",
    );
    assert_eq!(t.configured, ConfiguredBudgets::default());
}

#[test]
fn route_lists_are_read_in_order() {
    let text = format!(
        r#"{models}
[orchestrator.routes.m]
candidates = [
  {{ runtime = "codex",  model = "gpt-6.1-sol",     effort = "high"   }},
  {{ runtime = "claude", model = "claude-opus-5-5", effort = "medium" }},
]
pick = "spread"

[orchestrator.routes.scout]
candidates = [
  {{ runtime = "codex",  model = "gpt-6-luna" }},
  {{ runtime = "claude", model = "claude-sonnet-5", effort = "low" }},
]
"#,
        models = shipped_codex_models(&["gpt-6.1-sol", "gpt-6-luna"]),
    );
    let (t, problems) = tuning_of(&text);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        t.routes.m,
        RouteList {
            candidates: vec![
                candidate(Runtime::Codex, "gpt-6.1-sol", Some(Effort::High)),
                candidate(Runtime::Claude, "claude-opus-5-5", Some(Effort::Medium)),
            ],
            pick: Pick::Spread,
        }
    );
    assert_eq!(
        t.routes.scout,
        RouteList {
            candidates: vec![
                candidate(Runtime::Codex, "gpt-6-luna", None),
                candidate(Runtime::Claude, "claude-sonnet-5", Some(Effort::Low)),
            ],
            pick: Pick::First,
        }
    );
    assert_eq!(
        RouteLists {
            m: RouteList::default(),
            scout: RouteList::default(),
            ..t.routes.clone()
        },
        RouteLists::default()
    );
}

#[test]
fn route_list_entries_not_in_the_roster_are_skipped() {
    let text = format!(
        r#"{models}
[orchestrator.routes.m]
candidates = [
  {{ runtime = "codex",  model = "gpt-6.1-sol", strength = "frontier" }},
  {{ runtime = "claude", model = "claude-opus-9" }},
  {{ runtime = "perl",   model = "claude-opus-5-5" }},
  {{ runtime = "claude", model = "claude-opus-5-5", effort = "max" }},
  {{ runtime = "claude", model = "" }},
  {{ runtime = "claude", model = "claude-sonnet-5" }},
]

[orchestrator.routes.xl]
candidates = [{{ runtime = "claude", model = "claude-sonnet-5" }}]
"#,
        models = shipped_codex_models(&["gpt-6.1-sol"]),
    );
    let (t, problems) = tuning_of(&text);
    assert_eq!(
        t.routes.m,
        RouteList {
            candidates: vec![
                candidate(Runtime::Codex, "gpt-6.1-sol", None),
                candidate(Runtime::Claude, "claude-sonnet-5", None),
            ],
            pick: Pick::First,
        }
    );
    assert_eq!(
        t.routes,
        RouteLists {
            m: t.routes.m.clone(),
            ..RouteLists::default()
        }
    );
    assert_eq!(
        problems,
        vec![
            problem(
                "orchestrator.routes.m.candidates[1]",
                "claude/claude-opus-9 is not in the roster",
                "entry skipped",
            ),
            problem(
                "orchestrator.routes.m.candidates[2].runtime",
                "must be claude or codex",
                "entry skipped",
            ),
            problem(
                "orchestrator.routes.m.candidates[3].effort",
                "must be low, medium or high",
                "entry skipped",
            ),
            problem(
                "orchestrator.routes.m.candidates[4].model",
                "must not be empty",
                "entry skipped",
            ),
            problem(
                "orchestrator.routes.m.candidates[0].strength",
                "unknown key, ignored",
                "nothing",
            ),
            problem("orchestrator.routes.xl", "unknown key, ignored", "nothing"),
        ]
    );
}

#[test]
fn a_list_naming_only_skipped_entries_is_empty_and_bad_shapes_are_problems() {
    let (t, problems) = tuning_of(
        r#"
[orchestrator.routes.review]
candidates = [{ runtime = "claude", model = "claude-opus-9" }]
pick = "random"

[orchestrator.routes.decider]
candidates = "claude-sonnet-5"

[orchestrator.routes]
planner = 3
"#,
    );
    assert_eq!(t.routes, RouteLists::default());
    assert_eq!(
        problems,
        vec![
            problem(
                "orchestrator.routes.review.candidates[0]",
                "claude/claude-opus-9 is not in the roster",
                "entry skipped",
            ),
            problem(
                "orchestrator.routes.review.pick",
                "must be first or spread",
                "first",
            ),
            problem(
                "orchestrator.routes.decider.candidates",
                "expected an array of tables",
                "no list",
            ),
            problem(
                "orchestrator.routes.planner",
                "expected a table",
                "table of defaults",
            ),
        ]
    );
}
