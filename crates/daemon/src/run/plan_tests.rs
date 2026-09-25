use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use proto::{Budget, Effort, Route, RunState, Runtime, Strength};

use super::*;
use crate::run::env::profile_env;
use crate::run::model::ReviewLevel;
use crate::run::model::Run;
use crate::run::roster::pick_reviewer;
use crate::run::test_support::*;

#[test]
fn the_brief_example_builds_a_run() {
    let config = config::Orchestrator::default();
    // The plan's limits must differ from their defaults, or the test cannot tell them apart.
    assert_eq!(
        (config.max_writers, config.max_readers, config.max_bounces),
        (3, 3, 2)
    );

    let run = build_with(EXAMPLE_PLAN, &config).unwrap_or_else(|e| panic!("{}", show(&e)));

    assert_eq!(run.id, RUN_ID);
    assert_eq!(run.goal, "Add password reset");
    assert_eq!(run.state, RunState::AwaitingApproval);
    assert_eq!(run.approved_by, None);
    assert_eq!(run.root, PathBuf::from("/tmp/x"));
    assert_eq!(run.project, PathBuf::from("/tmp/p"));
    assert_eq!(run.git_common_dir, PathBuf::from("/tmp/p/.git"));
    assert_eq!(run.base_sha, "b".repeat(40));
    assert_eq!(run.run_head, run.base_sha);
    assert_eq!(run.created_at, 1_000);
    assert_eq!(run.tasks.len(), 1);

    let t1 = task(&run, "t1");
    assert_eq!(t1.branch, format!("anthrex/{RUN_ID}/t1"));
    assert_eq!(
        t1.worktree,
        PathBuf::from(format!("/tmp/wt/runs/{RUN_ID}/t1"))
    );
    let expected_route = Route {
        runtime: Runtime::Claude,
        model: "claude-sonnet-5".to_string(),
        strength: Strength::Standard,
        effort: Effort::High,
    };
    assert_eq!(t1.route, expected_route);
    assert_eq!(
        t1.budget,
        Budget {
            tool_calls: 120,
            minutes: 45,
            tokens: Some(3_000_000)
        }
    );
    assert_eq!(t1.review_level, Some(ReviewLevel::Medium));
    let reviewer = pick_reviewer(&config.models, &expected_route, ReviewLevel::Medium);
    assert_eq!(t1.review_route, Some(reviewer));
    assert_eq!(t1.notes, Vec::<String>::new());
    assert_eq!(t1.implicit_deps, Vec::<String>::new());

    let limits = &run.limits;
    assert_eq!(
        (limits.max_writers, limits.max_readers, limits.max_bounces),
        (4, 2, 3)
    );
    assert_eq!(limits.max_tasks, config.max_tasks);
    assert_eq!(limits.max_windows, config.max_windows);
    assert_eq!(limits.budget_m, config.budget_m);
    assert_eq!(limits.stall_after_secs, config.stall_after_secs);
    assert_eq!(limits.worker_allowed_tools, config.worker_allowed_tools);
    assert_eq!(run.roster, config.models);
    assert_eq!(run.run_branch(), format!("anthrex/{RUN_ID}/integration"));
    assert_eq!(run.short(), "3f9a");
}

fn profile_spec(f: impl FnOnce(&mut ProfileSpec)) -> ProfileSpec {
    let mut spec = ProfileSpec::default();
    f(&mut spec);
    spec
}

#[test]
fn profile_keys_resolve_per_key() {
    let plan = profile_spec(|p| {
        p.check = Some("make test".to_string());
        p.generated = Some(vec!["plan.lock".to_string()]);
        p.env = Some(BTreeMap::from([(
            "TARGET".to_string(),
            "{worktree}/target".to_string(),
        )]));
    });
    let config = profile_spec(|p| {
        p.single_test = Some("make one T={test}".to_string());
        p.check = Some("config check".to_string());
        p.generated = Some(vec!["config.lock".to_string()]);
        p.setup = Some("config setup".to_string());
    });

    let profile = resolve_profile(&plan, &config);

    assert_eq!(profile.check.as_deref(), Some("make test"));
    assert_eq!(profile.single_test.as_deref(), Some("make one T={test}"));
    assert_eq!(profile.setup.as_deref(), Some("config setup"));
    assert_eq!(profile.generated, vec!["plan.lock".to_string()]);
    assert_eq!(profile.check_timeout_secs, 1800);
    assert_eq!(profile.test_passed, None);
    assert_eq!(
        profile_env(&profile, Path::new("/tmp/wt/runs/r/t1")),
        vec![("TARGET".to_string(), "/tmp/wt/runs/r/t1/target".to_string())]
    );
}

#[test]
fn cache_dirs_come_from_the_plan_else_the_config() {
    // M8a final fix batch F1c (I2).
    let config = profile_spec(|p| p.cache_dirs = Some(vec!["~/.cache/c".to_string()]));
    assert_eq!(
        resolve_profile(&profile_spec(|_| {}), &config).cache_dirs,
        vec!["~/.cache/c".to_string()]
    );
    let plan = profile_spec(|p| p.cache_dirs = Some(Vec::new()));
    assert!(resolve_profile(&plan, &config).cache_dirs.is_empty());
}

#[test]
fn a_blank_plan_key_clears_the_config_value() {
    let plan = profile_spec(|p| p.check = Some("  ".to_string()));
    let config = profile_spec(|p| p.check = Some("config check".to_string()));
    assert_eq!(resolve_profile(&plan, &config).check, None);
}

#[test]
fn generated_globs_are_validated() {
    let text = plan_with(
        &format!("{PROFILE}generated = [\"/abs/Cargo.lock\", \"../x.lock\", \"ok.lock\"]\n"),
        &[task_toml("t1", "S", r#"["crates/a/src/lib.rs"]"#, "")],
    );
    assert_eq!(
        errors_of(&text),
        vec![
            err(
                None,
                "profile.generated",
                "globs",
                "/abs/Cargo.lock must not be absolute"
            ),
            err(
                None,
                "profile.generated",
                "globs",
                "../x.lock must not contain .."
            ),
        ]
    );
}

#[test]
fn plan_protected_adds_to_config_and_builtins() {
    let plan = profile_spec(|p| p.protected = Some(vec!["docs/agents/**".to_string()]));
    let config = profile_spec(|p| p.protected = Some(vec!["ops/prompts/**".to_string()]));

    let profile = resolve_profile(&plan, &config);

    let mut expected: Vec<String> = BUILTIN_PROTECTED.iter().map(|s| s.to_string()).collect();
    expected.push("ops/prompts/**".to_string());
    expected.push("docs/agents/**".to_string());
    assert_eq!(profile.protected, expected);

    // Neither side can remove a built-in: an empty list still leaves all five.
    let none = resolve_profile(
        &profile_spec(|p| p.protected = Some(Vec::new())),
        &ProfileSpec::default(),
    );
    assert_eq!(none.protected.len(), BUILTIN_PROTECTED.len());
}

fn protected_note(glob: &str, path: &str) -> String {
    format!(
        "owns {glob} covers protected {path}; name it exactly in owns if this task must change it (rule 6.protected)"
    )
}

#[test]
fn owns_covering_a_protected_file_warns_without_rejecting() {
    // No modules, hub or source, so no size or test-mode note mixes in.
    let profile = "goal = \"g\"\n[profile]\ncheck = \"c\"\nsingle_test = \"s {test}\"\ntest_passed = \"{test} ok\"\n";
    let text = plan_with(
        profile,
        &[
            task_toml("t1", "S", r#"["**"]"#, ""),
            task_toml("t2", "S", r#"["AGENTS.md"]"#, ""),
            task_toml("t3", "S", r#"[".claude/**"]"#, ""),
        ],
    );
    let mut pre = preflight();
    pre.protected_files = vec!["AGENTS.md".to_string(), ".claude/settings.json".to_string()];

    let run = build_full(&text, &config::Orchestrator::default(), pre)
        .unwrap_or_else(|e| panic!("{}", show(&e)));

    assert_eq!(
        task(&run, "t1").notes,
        vec![
            protected_note("**", "AGENTS.md"),
            protected_note("**", ".claude/settings.json"),
        ]
    );
    assert_eq!(task(&run, "t2").notes, Vec::<String>::new());
    assert_eq!(
        task(&run, "t3").notes,
        vec![protected_note(".claude/**", ".claude/settings.json")]
    );
    assert_eq!(
        run.protected_files,
        vec!["AGENTS.md".to_string(), ".claude/settings.json".to_string()]
    );
}

fn route(runtime: Runtime, model: &str, strength: Strength, effort: Effort) -> Route {
    Route {
        runtime,
        model: model.to_string(),
        strength,
        effort,
    }
}

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
            Effort::Low
        )
    );
    assert_eq!(
        task(&run, "m").route,
        route(
            Runtime::Claude,
            "claude-sonnet-5",
            Strength::Standard,
            Effort::Medium
        )
    );
    assert_eq!(
        task(&run, "hub").route,
        route(
            Runtime::Claude,
            "claude-opus-5",
            Strength::Frontier,
            Effort::High
        )
    );
    // Budgets by class: S the S budget, M and hub the M budget.
    let config = config::Orchestrator::default();
    assert_eq!(task(&run, "s").budget, config.budget_s);
    assert_eq!(task(&run, "m").budget, config.budget_m);
    assert_eq!(task(&run, "hub").budget, config.budget_m);
}

#[test]
fn default_runtime_comes_from_config() {
    let text = plan_with(
        PROFILE,
        &[task_toml("s", "S", r#"["crates/a/src/lib.rs"]"#, "")],
    );
    let config = config::Orchestrator {
        default_runtime: Runtime::Codex,
        ..config::Orchestrator::default()
    };
    let run = build_with(&text, &config).unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(
        task(&run, "s").route,
        route(Runtime::Codex, "", Strength::Standard, Effort::Low)
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
            Effort::Low
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
            err(
                Some("c"),
                "route",
                "route",
                "the roster has no codex model at frontier strength"
            ),
        ]
    );
}

/// Unit 1, bounds 1..=8 (writers, readers), 1..=5 (bounces) and 10..=14400
/// (check_timeout_secs): each edge is accepted and the value one past it rejected. The
/// unit (1) divides every bound, so each edge is hit exactly. Within every fixture the
/// three limits stay pairwise distinct, so a limit read from the wrong key shows.
#[test]
fn limit_ranges() {
    let tasks = [task_toml("t1", "S", r#"["crates/a/src/lib.rs"]"#, "")];
    let header = |w: u8, r: u8, b: u8, timeout: u64| {
        PROFILE.replace(
            "goal = \"Test goal\"\n",
            &format!(
                "goal = \"Test goal\"\nmax_writers = {w}\nmax_readers = {r}\nmax_bounces = {b}\n"
            ),
        ) + &format!("check_timeout_secs = {timeout}\n")
    };
    let limits = |run: &Run| {
        (
            run.limits.max_writers,
            run.limits.max_readers,
            run.limits.max_bounces,
            run.profile.check_timeout_secs,
        )
    };

    for accepted in [
        (1, 2, 3, 10),
        (3, 1, 2, 11),
        (2, 3, 1, 12),
        (8, 7, 5, 14_400),
        (7, 8, 4, 14_399),
    ] {
        let (w, r, b, t) = accepted;
        let run = run_ok(&plan_with(&header(w, r, b, t), &tasks));
        assert_eq!(limits(&run), accepted);
    }

    let range = |field: &str, lo: u64, hi: u64| {
        err(
            None,
            field,
            "range",
            &format!("must be between {lo} and {hi}"),
        )
    };
    let cases = [
        ((0, 2, 3, 10), range("max_writers", 1, 8)),
        ((9, 2, 3, 10), range("max_writers", 1, 8)),
        ((2, 0, 3, 10), range("max_readers", 1, 8)),
        ((2, 9, 3, 10), range("max_readers", 1, 8)),
        ((2, 3, 0, 10), range("max_bounces", 1, 5)),
        ((2, 3, 6, 10), range("max_bounces", 1, 5)),
        (
            (2, 3, 4, 9),
            range("profile.check_timeout_secs", 10, 14_400),
        ),
        (
            (2, 3, 4, 14_401),
            range("profile.check_timeout_secs", 10, 14_400),
        ),
    ];
    for ((w, r, b, t), expected) in cases {
        assert_eq!(
            errors_of(&plan_with(&header(w, r, b, t), &tasks)),
            vec![expected],
            "limits {w}/{r}/{b}/{t}"
        );
    }
}

/// Unit 1, bound "at least 1" on each axis: 0 is rejected and 1 accepted.
#[test]
fn budget_ranges() {
    let zero = plan_with(
        PROFILE,
        &[task_toml(
            "t1",
            "S",
            r#"["crates/a/src/lib.rs"]"#,
            "[task.budget]\ntool_calls = 0\nminutes = 0\ntokens = 0",
        )],
    );
    let at_least_one = |field: &str| err(Some("t1"), field, "range", "must be at least 1");
    assert_eq!(
        errors_of(&zero),
        vec![
            at_least_one("budget.tool_calls"),
            at_least_one("budget.minutes"),
            at_least_one("budget.tokens"),
        ]
    );

    let one = plan_with(
        PROFILE,
        &[task_toml(
            "t1",
            "S",
            r#"["crates/a/src/lib.rs"]"#,
            "[task.budget]\ntool_calls = 1\nminutes = 1\ntokens = 1",
        )],
    );
    assert_eq!(
        task(&run_ok(&one), "t1").budget,
        Budget {
            tool_calls: 1,
            minutes: 1,
            tokens: Some(1)
        }
    );
}

#[test]
fn single_test_must_contain_the_placeholder() {
    let text = plan_with(
        &PROFILE.replace("-- --exact {test}", "-- --exact TEST"),
        &[task_toml("t1", "S", r#"["crates/a/src/lib.rs"]"#, "")],
    );
    assert_eq!(
        errors_of(&text),
        vec![err(
            None,
            "profile.single_test",
            "profile",
            "must contain {test}"
        )]
    );
}

#[test]
fn test_passed_must_compile_as_a_regex() {
    let text = plan_with(
        &PROFILE.replace("\\.\\.\\. ok'", "(ok'"),
        &[task_toml("t1", "S", r#"["crates/a/src/lib.rs"]"#, "")],
    );
    // Built at run time: clippy rejects an invalid regex literal.
    let pattern = format!("test {} (ok", "t");
    let regex_error = regex::Regex::new(&pattern).unwrap_err();
    assert_eq!(
        errors_of(&text),
        vec![err(
            None,
            "profile.test_passed",
            "profile",
            &format!("is not a valid regular expression: {regex_error}")
        )]
    );

    // `{test}` itself is a placeholder, not a regex repetition, so this one compiles.
    let good = plan_with(
        PROFILE,
        &[task_toml("t1", "S", r#"["crates/a/src/lib.rs"]"#, "")],
    );
    run_ok(&good);
}

#[test]
fn env_key_syntax() {
    let text = plan_with(
        &format!("{PROFILE}[profile.env]\n\"1BAD\" = \"x\"\n\"A-B\" = \"y\"\n_OK_1 = \"z\"\n"),
        &[task_toml("t1", "S", r#"["crates/a/src/lib.rs"]"#, "")],
    );
    let bad = |key: &str| {
        err(
            None,
            "profile.env",
            "profile",
            &format!("key {key} must match [A-Za-z_][A-Za-z0-9_]*"),
        )
    };
    assert_eq!(errors_of(&text), vec![bad("1BAD"), bad("A-B")]);
}

#[test]
fn all_errors_are_collected() {
    let text = plan_with(
        &PROFILE.replace(
            "goal = \"Test goal\"\n",
            "goal = \"Test goal\"\nmax_writers = 0\n",
        ),
        &[
            task_toml("t1", "S", r#"["crates/a/src/lib.rs"]"#, "")
                .replace("title = \"Title t1\"", "title = \" \""),
            task_toml("t2", "S", r#"["crates/b/src/lib.rs"]"#, "deps = [\"t9\"]"),
        ],
    );
    assert_eq!(
        errors_of(&text),
        vec![
            err(None, "max_writers", "range", "must be between 1 and 8"),
            err(Some("t1"), "title", "fields", "must not be blank"),
            err(Some("t2"), "deps", "12.1", "t9 is not a task"),
        ]
    );
}

#[path = "plan_tests_parse.rs"]
mod parse;

#[path = "plan_tests_build.rs"]
mod build;
