//! `[orchestrator.tuning]`, the explicit budgets and the `[orchestrator.routes.*]` model
//! lists (milestone 9.5 task M9.5.7).

use proto::{Effort, Runtime};

use super::*;
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

/// `[[orchestrator.models]]` entries for the named Codex models, so the roster holds them
/// (the built-in roster has only Codex's default). The strengths are the ones the old
/// Settings screen shipped (`settings/shipped.rs`, removed in M9.8.13).
fn codex_models(names: &[&str]) -> String {
    let mut text = String::new();
    for name in names {
        let strength = match *name {
            "gpt-6.1-sol" => "frontier",
            "gpt-6-luna" => "fast",
            other => panic!("no strength for {other}"),
        };
        text.push_str(&format!(
            "[[orchestrator.models]]\nruntime = \"codex\"\nmodel = \"{name}\"\nstrength = \"{strength}\"\n\n"
        ));
    }
    text
}

/// Milestone 9.8 (task M9.8.13): `TuningConfig` no longer keeps the model lists; the
/// migration reads them from the raw `[orchestrator]` table with `read_routes`, as here.
/// The problems are the whole parse's, which still reports the lists' own.
fn lists_of(text: &str) -> (RouteLists, Vec<Problem>) {
    let (config, problems) = parse(text);
    let raw: toml::Table = toml::from_str(text).expect("TOML");
    let orchestrator = (raw.get("orchestrator").and_then(|o| o.as_table()))
        .cloned()
        .unwrap_or_default();
    let lists = read_routes(&orchestrator, &config.orchestrator.models, &mut Vec::new());
    (lists, problems)
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
            refit_budgets: true,
            refit_tokens: false,
            path_weights: true,
            adaptive_concurrency: true,
            recover_after_mins: 10,
            halve_hold_secs: 60,
            race_slot_wait_secs: 120,
        }
    );
    let (lists, _) = lists_of("");
    for list in [
        &lists.s,
        &lists.m,
        &lists.hub,
        &lists.review,
        &lists.scout,
        &lists.decider,
        &lists.planner,
        &lists.orchestrator,
        &lists.brainstorm,
    ] {
        assert!(list.candidates.is_empty());
        assert_eq!(list.pick, Pick::First);
    }
    assert_eq!(t.configured, ConfiguredBudgets::default());
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
    assert_eq!(
        t.configured,
        ConfiguredBudgets {
            s: true,
            ..ConfiguredBudgets::default()
        }
    );

    let (t, _) = tuning_of("[orchestrator.budget.m]\nminutes = 60\n");
    assert_eq!(
        t.configured,
        ConfiguredBudgets {
            m: true,
            ..ConfiguredBudgets::default()
        }
    );

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

/// Milestone 9.6 decision 33 (task M9.6.13): ruling RH-5 holds for the design
/// agents' budgets too: a `[orchestrator.design.budget.<role>]` that sets a value
/// beats the refit; an empty one does not.
#[test]
fn an_explicit_design_budget_is_configured() {
    let (t, problems) = tuning_of(
        "[orchestrator.design.budget.brainstormer]
tool_calls = 60
",
    );
    assert!(problems.is_empty(), "{problems:?}");
    let brainstormer = ConfiguredBudgets {
        brainstormer: true,
        ..ConfiguredBudgets::default()
    };
    assert_eq!(t.configured, brainstormer);
    let (t, _) = tuning_of(
        "[orchestrator.design.budget.doc_reviewer]
minutes = 20
",
    );
    let reviewer = ConfiguredBudgets {
        doc_reviewer: true,
        ..ConfiguredBudgets::default()
    };
    assert_eq!(t.configured, reviewer);
    let (t, _) = tuning_of(
        "[orchestrator.design.budget.brainstormer]
",
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
        models = codex_models(&["gpt-6.1-sol", "gpt-6-luna"]),
    );
    let (lists, problems) = lists_of(&text);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        lists.m,
        RouteList {
            candidates: vec![
                candidate(Runtime::Codex, "gpt-6.1-sol", Some(Effort::HIGH)),
                candidate(Runtime::Claude, "claude-opus-5-5", Some(Effort::MEDIUM)),
            ],
            pick: Pick::Spread,
        }
    );
    assert_eq!(
        lists.scout,
        RouteList {
            candidates: vec![
                candidate(Runtime::Codex, "gpt-6-luna", None),
                candidate(Runtime::Claude, "claude-sonnet-5", Some(Effort::LOW)),
            ],
            pick: Pick::First,
        }
    );
    assert_eq!(
        RouteLists {
            m: RouteList::default(),
            scout: RouteList::default(),
            ..lists.clone()
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
        models = codex_models(&["gpt-6.1-sol"]),
    );
    let (lists, problems) = lists_of(&text);
    assert_eq!(
        lists.m,
        RouteList {
            candidates: vec![
                candidate(Runtime::Codex, "gpt-6.1-sol", None),
                candidate(Runtime::Claude, "claude-sonnet-5", None),
            ],
            pick: Pick::First,
        }
    );
    assert_eq!(
        lists,
        RouteLists {
            m: lists.m.clone(),
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
    // Decision 9a's exact text.
    assert_eq!(
        problems[0].to_string(),
        "orchestrator.routes.m.candidates[1]: claude/claude-opus-9 is not in the roster (entry skipped)"
    );
}

#[test]
fn a_list_naming_only_skipped_entries_is_empty_and_bad_shapes_are_problems() {
    let (lists, problems) = lists_of(
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
    assert_eq!(lists, RouteLists::default());
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

#[test]
fn a_bad_candidate_model_or_entry_says_what_is_wrong() {
    let (lists, problems) = lists_of(
        r#"
[orchestrator.routes.s]
candidates = [
  { runtime = "claude", model = 5 },
  { runtime = "claude" },
  "claude-sonnet-5",
  { runtime = "claude", model = "claude-haiku-4-5" },
]
"#,
    );
    assert_eq!(
        lists.s.candidates,
        vec![candidate(Runtime::Claude, "claude-haiku-4-5", None)]
    );
    assert_eq!(
        problems,
        vec![
            problem(
                "orchestrator.routes.s.candidates[0].model",
                "must be a string",
                "entry skipped",
            ),
            problem(
                "orchestrator.routes.s.candidates[1].model",
                "is required",
                "entry skipped",
            ),
            problem(
                "orchestrator.routes.s.candidates[2]",
                "expected a table",
                "entry skipped",
            ),
        ]
    );
}

#[test]
fn bad_table_shapes_and_types_keep_defaults() {
    let (t, problems) = tuning_of("[orchestrator]\ntuning = 3\n");
    assert_eq!(t.table, Tuning::default());
    assert_eq!(
        problems,
        vec![problem(
            "orchestrator.tuning",
            "expected a table",
            "table of defaults",
        )]
    );

    let (t, problems) = tuning_of("[orchestrator.tuning]\nrefit_budgets = \"no\"\n");
    assert!(t.table.refit_budgets);
    assert_eq!(
        problems,
        vec![problem(
            "orchestrator.tuning.refit_budgets",
            "expected a boolean",
            "true",
        )]
    );
}

#[test]
fn the_u64_keys_have_upper_bounds() {
    for (key, value, message, default) in [
        ("recover_after_mins", 241, "must be between 1 and 240", "10"),
        ("halve_hold_secs", 3601, "must be between 0 and 3600", "60"),
        (
            "race_slot_wait_secs",
            3601,
            "must be between 0 and 3600",
            "120",
        ),
    ] {
        let (t, problems) = tuning_of(&format!("[orchestrator.tuning]\n{key} = {value}\n"));
        assert_eq!(t.table, Tuning::default(), "{key}");
        assert_eq!(
            problems,
            vec![problem(
                &format!("orchestrator.tuning.{key}"),
                message,
                default
            )]
        );
    }
    let (t, problems) =
        tuning_of("[orchestrator.tuning]\nrecover_after_mins = 240\nhalve_hold_secs = 3600\n");
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        (t.table.recover_after_mins, t.table.halve_hold_secs),
        (240, 3600)
    );
}

/// Deliberate (fix round 1 of M9.5.7): a `min_samples` above the default window raises
/// the window too, with a problem, although the file never names `window`.
#[test]
fn min_samples_alone_above_the_default_window_raises_it() {
    let (t, problems) = tuning_of("[orchestrator.tuning]\nmin_samples = 500\n");
    assert_eq!((t.table.min_samples, t.table.window), (500, 500));
    assert_eq!(
        problems.iter().map(|p| p.to_string()).collect::<Vec<_>>(),
        vec!["orchestrator.tuning.window: must be at least min_samples (500) (using 500)"]
    );
}

/// Milestone 9.8 decision 30 (task M9.8.13): the route refit is gone, so
/// `escalate_above_percent` stays a known key whose value nothing uses: one problem
/// saying so, and no unknown-key problem.
#[test]
fn escalate_above_percent_is_known_and_unused() {
    let (_, problems) = tuning_of("[orchestrator.tuning]\nescalate_above_percent = 40\n");
    assert_eq!(
        problems,
        [problem(
            "orchestrator.tuning.escalate_above_percent",
            "no longer used; models come from the role table",
            "nothing",
        )]
    );
    assert_eq!(
        problems[0].to_string(),
        "orchestrator.tuning.escalate_above_percent: no longer used; models come from the role table (using nothing)"
    );
}
