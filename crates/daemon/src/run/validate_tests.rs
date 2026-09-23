use proto::{Size, TestMode};

use super::*;
use crate::run::model::ReviewLevel;
use crate::run::plan::resolve_profile;
use crate::run::test_support::*;

// ---- Size rules, decision 9 ----

#[test]
fn two_modules_raise_s_to_m() {
    let run = run_ok(&plan_with(
        PROFILE,
        &[task_toml(
            "t1",
            "S",
            r#"["crates/a/src/lib.rs", "crates/b/src/lib.rs"]"#,
            "",
        )],
    ));
    let t1 = task(&run, "t1");
    assert_eq!(t1.spec.size, Size::S);
    assert_eq!(t1.size, Size::M);
    assert!(!t1.hub);
}

#[test]
fn two_modules_with_interface_change_is_l_and_rejected() {
    let text = plan_with(
        PROFILE,
        &[task_toml(
            "t1",
            "S",
            r#"["crates/a/src/lib.rs", "crates/b/src/lib.rs"]"#,
            "interface_change = true",
        )],
    );
    assert_eq!(
        errors_of(&text),
        vec![err(
            Some("t1"),
            "size",
            "7.2.4",
            "L tasks are never executed; split the task (rule 7.2.4)"
        )]
    );
}

#[test]
fn hub_touch_raises_to_m_and_sets_hub() {
    let run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", r#"["crates/proto/src/run.rs"]"#, "")],
    ));
    let t1 = task(&run, "t1");
    assert_eq!(t1.size, Size::M);
    assert!(t1.hub);
}

#[test]
fn l_is_rejected() {
    let text = plan_with(
        PROFILE,
        &[task_toml("t1", "L", r#"["crates/a/src/lib.rs"]"#, "")],
    );
    assert_eq!(
        errors_of(&text),
        vec![err(
            Some("t1"),
            "size",
            "7.2.4",
            "L tasks are never executed; split the task (rule 7.2.4)"
        )]
    );
}

#[test]
fn raises_are_recorded_as_notes() {
    let run = run_ok(&plan_with(
        PROFILE,
        &[
            task_toml(
                "two",
                "S",
                r#"["crates/a/src/lib.rs", "crates/b/src/lib.rs"]"#,
                "",
            ),
            task_toml("hub", "S", r#"["crates/proto/src/run.rs"]"#, ""),
            task_toml("many", "S", r#"["crates/**"]"#, ""),
            // Already M: two modules raise nothing, so there is no note.
            task_toml(
                "mm",
                "M",
                r#"["crates/c/src/lib.rs", "crates/d/src/lib.rs"]"#,
                "",
            ),
        ],
    ));
    assert_eq!(
        task(&run, "two").notes,
        vec!["size raised from S to M: owns spans 2 modules (rule 7.2.1)".to_string()]
    );
    assert_eq!(
        task(&run, "hub").notes,
        vec!["size raised from S to M: owns touch the hub globs (rule 7.2.3)".to_string()]
    );
    // `crates/**` spans more than one module by itself and also touches the hub.
    assert_eq!(
        task(&run, "many").notes,
        vec!["size raised from S to M: owns spans more than one module (rule 7.2.1)".to_string()]
    );
    assert!(task(&run, "many").hub);
    assert_eq!(task(&run, "mm").notes, Vec::<String>::new());

    // The rule-7.2.2 raise is also a note, on a task that then fails rule 7.2.4.
    let profile = resolve_profile(
        &crate::run::plan::parse_plan(&plan_with(PROFILE, &[task_toml("x", "S", "[\"x\"]", "")]))
            .unwrap()
            .profile,
        &proto::ProfileSpec::default(),
    );
    let config = config::Orchestrator::default();
    let limits = crate::run::plan::run_limits(&config, None, None, None);
    let spec = crate::run::plan::parse_plan(&plan_with(
        PROFILE,
        &[task_toml(
            "big",
            "M",
            r#"["crates/a/src/lib.rs", "crates/b/src/lib.rs"]"#,
            "interface_change = true",
        )],
    ))
    .unwrap()
    .tasks
    .remove(0);
    let big = resolve_task(
        spec,
        &profile,
        &limits,
        &config.models,
        config.default_runtime,
    )
    .unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(big.size, Size::L);
    assert_eq!(
        big.notes,
        vec![
            "size raised from M to L: owns spans 2 modules and interface_change is set (rule 7.2.2)"
                .to_string()
        ]
    );
}

// ---- Test-mode rules, decision 10 ----

#[test]
fn code_defaults_to_tdd_and_docs_to_none() {
    let run = run_ok(&plan_with(
        PROFILE,
        &[
            task_toml("code", "S", r#"["crates/a/src/lib.rs"]"#, ""),
            task_toml("docs", "S", r#"["docs/guide.md"]"#, "kind = \"docs\""),
        ],
    ));
    assert_eq!(task(&run, "code").test_mode, TestMode::Tdd);
    assert_eq!(task(&run, "docs").test_mode, TestMode::None);
}

#[test]
fn non_tdd_needs_a_reason() {
    let text = plan_with(
        PROFILE,
        &[
            task_toml(
                "a",
                "S",
                r#"["crates/a/src/lib.rs"]"#,
                "test_mode = \"check\"",
            ),
            task_toml(
                "b",
                "S",
                r#"["docs/b.md"]"#,
                "kind = \"docs\"\ntest_mode = \"check\"\ntest_mode_reason = \"  \"",
            ),
            task_toml(
                "c",
                "S",
                r#"["crates/c/src/lib.rs"]"#,
                "test_mode = \"check\"\ntest_mode_reason = \"a refactor with full coverage\"",
            ),
        ],
    );
    let reason = |id: &str| {
        err(
            Some(id),
            "test_mode_reason",
            "8",
            "required when test_mode is check or none",
        )
    };
    assert_eq!(errors_of(&text), vec![reason("a"), reason("b")]);
}

#[test]
fn none_on_source_is_rejected() {
    let text = plan_with(
        PROFILE,
        &[
            task_toml(
                "a",
                "S",
                r#"["crates/a/src/lib.rs"]"#,
                "test_mode = \"none\"\ntest_mode_reason = \"trivial\"",
            ),
            // Outside source: allowed. (Decision 11's literal-prefix intersection makes
            // `crates/*/src/**` touch everything under `crates/`, so "outside" is a
            // path outside `crates/`.)
            task_toml(
                "b",
                "S",
                r#"["scripts/release.sh"]"#,
                "test_mode = \"none\"\ntest_mode_reason = \"manifest only\"",
            ),
        ],
    );
    assert_eq!(
        errors_of(&text),
        vec![err(
            Some("a"),
            "test_mode",
            "8.1",
            "a code task whose owns touch the profile's source globs cannot be none (rule 8.1)"
        )]
    );
}

#[test]
fn hub_code_is_forced_to_tdd() {
    let run = run_ok(&plan_with(
        PROFILE,
        &[task_toml(
            "t1",
            "M",
            r#"["crates/proto/src/run.rs"]"#,
            "test_mode = \"check\"\ntest_mode_reason = \"protocol only\"",
        )],
    ));
    let t1 = task(&run, "t1");
    assert_eq!(t1.test_mode, TestMode::Tdd);
    assert_eq!(
        t1.notes,
        vec!["test mode forced to tdd: hub task (rule 8.2)".to_string()]
    );
}

#[test]
fn tdd_without_single_test_becomes_check_and_raises_review() {
    // Owns outside source (and outside `crates/`, see `none_on_source_is_rejected`) and
    // a check present, so rule 8.3 is the only raise.
    let profile = PROFILE.replace("single_test = \"cargo test -- --exact {test}\"\n", "");
    let run = run_ok(&plan_with(
        &profile,
        &[task_toml("t1", "S", r#"["scripts/release.sh"]"#, "")],
    ));
    let t1 = task(&run, "t1");
    assert_eq!(t1.test_mode, TestMode::Check);
    assert_eq!(t1.review_level, Some(ReviewLevel::Medium));
    assert_eq!(
        t1.notes,
        vec!["test mode check: the profile has no single_test (rule 8.3)".to_string()]
    );
}

// ---- Review level, decision 35 ----

#[test]
fn no_check_raises_every_review() {
    let with_check = plan_with(
        PROFILE,
        &[
            task_toml("s", "S", r#"["crates/a/src/lib.rs"]"#, ""),
            task_toml("m", "M", r#"["crates/b/src/lib.rs"]"#, ""),
            task_toml("hub", "M", r#"["crates/proto/src/lib.rs"]"#, ""),
        ],
    );
    let run = run_ok(&with_check);
    let levels =
        |run: &crate::run::model::Run| ["s", "m", "hub"].map(|id| task(run, id).review_level);
    assert_eq!(
        levels(&run),
        [
            Some(ReviewLevel::Small),
            Some(ReviewLevel::Medium),
            Some(ReviewLevel::Frontier)
        ]
    );

    let without = run_ok(&with_check.replace("check = \"cargo test\"\n", ""));
    assert_eq!(
        levels(&without),
        [
            Some(ReviewLevel::Medium),
            Some(ReviewLevel::Frontier),
            Some(ReviewLevel::Frontier)
        ]
    );
}

#[test]
fn check_mode_on_source_raises_review() {
    let run = run_ok(&plan_with(
        PROFILE,
        &[
            task_toml(
                "src",
                "S",
                r#"["crates/a/src/lib.rs"]"#,
                "test_mode = \"check\"\ntest_mode_reason = \"glue\"",
            ),
            task_toml(
                "out",
                "S",
                r#"["scripts/release.sh"]"#,
                "test_mode = \"check\"\ntest_mode_reason = \"glue\"",
            ),
        ],
    ));
    assert_eq!(task(&run, "src").review_level, Some(ReviewLevel::Medium));
    assert_eq!(task(&run, "out").review_level, Some(ReviewLevel::Small));
}

#[test]
fn review_small_off_skips_s_but_not_hub() {
    let text = plan_with(
        PROFILE,
        &[
            task_toml("s", "S", r#"["crates/a/src/lib.rs"]"#, ""),
            // S raised to Medium by rule 8's level rule: still reviewed.
            task_toml(
                "raised",
                "S",
                r#"["crates/b/src/lib.rs"]"#,
                "test_mode = \"check\"\ntest_mode_reason = \"glue\"",
            ),
            task_toml("hub", "S", r#"["crates/proto/src/lib.rs"]"#, ""),
        ],
    );
    let config = config::Orchestrator {
        review_small: false,
        ..config::Orchestrator::default()
    };
    let run = build_with(&text, &config).unwrap_or_else(|e| panic!("{}", show(&e)));
    assert_eq!(task(&run, "s").review_level, None);
    assert_eq!(task(&run, "s").review_route, None);
    assert_eq!(task(&run, "raised").review_level, Some(ReviewLevel::Medium));
    assert_eq!(task(&run, "hub").review_level, Some(ReviewLevel::Frontier));
    assert!(task(&run, "hub").review_route.is_some());
}

// ---- Runtimes and implicit dependencies, decision 11 ----

#[test]
fn cross_runtime_overlap_is_rejected_on_the_later_task() {
    let text = plan_with(
        PROFILE,
        &[
            task_toml(
                "t1",
                "M",
                r#"["crates/proto/**"]"#,
                "[task.route]\nruntime = \"claude\"",
            ),
            task_toml(
                "t2",
                "M",
                r#"["crates/proto/src/run.rs"]"#,
                "[task.route]\nruntime = \"codex\"\nstrength = \"standard\"",
            ),
        ],
    );
    // Codex has no frontier model, so t2 (a hub task) names standard explicitly.
    assert_eq!(
        errors_of(&text),
        vec![err(
            Some("t2"),
            "owns",
            "9",
            "overlaps task t1's owns (crates/proto/**) and the two tasks run on different runtimes (claude, codex) (rule 9)"
        )]
    );
}

#[test]
fn same_runtime_overlap_is_allowed_and_becomes_an_implicit_dep() {
    let run = run_ok(&plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", r#"["crates/a/src/**"]"#, ""),
            task_toml("t2", "S", r#"["crates/a/src/lib.rs"]"#, ""),
            task_toml("t3", "S", r#"["crates/b/src/lib.rs"]"#, ""),
        ],
    ));
    assert_eq!(task(&run, "t1").implicit_deps, Vec::<String>::new());
    assert_eq!(task(&run, "t2").implicit_deps, vec!["t1".to_string()]);
    assert_eq!(task(&run, "t3").implicit_deps, Vec::<String>::new());
}

#[path = "validate_tests_fields.rs"]
mod fields;
