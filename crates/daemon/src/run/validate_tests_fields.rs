//! Graph and field rules, decisions 6, 11 and 12.

use std::collections::BTreeSet;

use proto::TaskState;

use super::super::*;
use crate::run::test_support::*;

fn one(id: &str, extra: &str) -> String {
    task_toml(id, "S", &format!("[\"crates/{id}/src/lib.rs\"]"), extra)
}

#[test]
fn duplicate_ids() {
    let text = plan_with(PROFILE, &[one("t1", ""), one("t2", ""), one("t1", "")]);
    assert_eq!(
        errors_of(&text.replacen("crates/t1/src", "crates/x/src", 1)),
        vec![err(Some("t1"), "id", "id", "t1 is used by an earlier task")]
    );
}

#[test]
fn id_syntax() {
    // 16 characters is the longest legal id (one leading, then {0,15}); 17 is not.
    let sixteen = "a".repeat(16);
    let seventeen = "b".repeat(17);
    let text = plan_with(
        PROFILE,
        &[
            one(&sixteen, ""),
            one(&seventeen, ""),
            one("T1", ""),
            one("-a", ""),
            one("a.b", ""),
            one("a-1", ""),
        ],
    );
    let bad = |id: &str| err(Some(id), "id", "id", "must match ^[a-z0-9][a-z0-9-]{0,15}$");
    assert_eq!(
        errors_of(&text),
        vec![bad(&seventeen), bad("T1"), bad("-a"), bad("a.b")]
    );
}

#[test]
fn reserved_id_integration() {
    let text = plan_with(PROFILE, &[one("integration", "")]);
    assert_eq!(
        errors_of(&text),
        vec![err(
            Some("integration"),
            "id",
            "id",
            "integration is reserved for the run branch"
        )]
    );
}

#[test]
fn unknown_dependency() {
    let text = plan_with(
        PROFILE,
        &[one("t1", ""), one("t2", "deps = [\"t1\", \"t9\"]")],
    );
    assert_eq!(
        errors_of(&text),
        vec![err(Some("t2"), "deps", "12.1", "t9 is not a task")]
    );
}

#[test]
fn cycle_is_reported_once() {
    let text = plan_with(
        PROFILE,
        &[
            one("t0", ""),
            one("t1", "deps = [\"t0\", \"t2\"]"),
            one("t2", "deps = [\"t3\"]"),
            one("t3", "deps = [\"t1\"]"),
            one("t4", "deps = [\"t4\"]"),
        ],
    );
    assert_eq!(
        errors_of(&text),
        vec![
            err(None, "deps", "12.1", "cycle t1 -> t2 -> t3 -> t1"),
            err(None, "deps", "12.1", "cycle t4 -> t4"),
        ]
    );
}

#[test]
fn blank_fields() {
    let text = format!(
        "goal = \" \"\n[profile]\ncheck = \"c\"\nsingle_test = \"s {{test}}\"\n{}",
        one("t1", "")
            .replace("title = \"Title t1\"", "title = \"\"")
            .replace("brief = \"Brief t1\"", "brief = \"\\n\"")
            .replace(
                "acceptance = [\"Accept t1\"]",
                "acceptance = [\"ok\", \" \"]"
            )
    );
    assert_eq!(
        errors_of(&text),
        vec![
            err(None, "goal", "fields", "must not be blank"),
            err(Some("t1"), "title", "fields", "must not be blank"),
            err(Some("t1"), "brief", "fields", "must not be blank"),
            err(
                Some("t1"),
                "acceptance",
                "fields",
                "item 2 must not be blank"
            ),
        ]
    );
    let none = plan_with(
        PROFILE,
        &[one("t1", "").replace("acceptance = [\"Accept t1\"]", "acceptance = []")],
    );
    assert_eq!(
        errors_of(&none),
        vec![err(
            Some("t1"),
            "acceptance",
            "fields",
            "at least one item is required"
        )]
    );
}

#[test]
fn owns_required() {
    let text = plan_with(PROFILE, &[task_toml("t1", "S", "[]", "")]);
    assert_eq!(
        errors_of(&text),
        vec![err(
            Some("t1"),
            "owns",
            "fields",
            "at least one glob is required"
        )]
    );
}

#[test]
fn absolute_owns() {
    let text = plan_with(
        PROFILE,
        &[task_toml(
            "t1",
            "S",
            r#"["/etc/passwd", "crates/../x", "crates/a/src/lib.rs"]"#,
            "",
        )],
    );
    assert_eq!(
        errors_of(&text),
        vec![
            err(
                Some("t1"),
                "owns",
                "globs",
                "/etc/passwd must not be absolute"
            ),
            err(
                Some("t1"),
                "owns",
                "globs",
                "crates/../x must not contain .."
            ),
        ]
    );
}

#[test]
fn research_and_review_kinds_are_deferred() {
    let text = plan_with(
        PROFILE,
        &[
            one(
                "t1",
                "kind = \"research\"\ntest_mode = \"none\"\ntest_mode_reason = \"r\"",
            ),
            one(
                "t2",
                "kind = \"review\"\ntest_mode = \"none\"\ntest_mode_reason = \"r\"",
            ),
        ],
    );
    assert_eq!(
        errors_of(&text),
        vec![
            err(
                Some("t1"),
                "kind",
                "kind",
                "research tasks are executed from milestone 9; use code or docs"
            ),
            err(
                Some("t2"),
                "kind",
                "kind",
                "review tasks are executed from milestone 9; use code or docs"
            ),
        ]
    );
}

/// Unit 1 task, bound `max_tasks` = 3 (not the default 50): 3 build, 4 are rejected.
#[test]
fn too_many_tasks() {
    let config = config::Orchestrator {
        max_tasks: 3,
        ..config::Orchestrator::default()
    };
    let three = plan_with(PROFILE, &[one("t1", ""), one("t2", ""), one("t3", "")]);
    assert_eq!(build_with(&three, &config).map(|r| r.tasks.len()), Ok(3));

    let four = plan_with(
        PROFILE,
        &[one("t1", ""), one("t2", ""), one("t3", ""), one("t4", "")],
    );
    assert_eq!(
        build_with(&four, &config).unwrap_err(),
        vec![err(None, "tasks", "range", "4 tasks exceed max_tasks (3)")]
    );
}

fn built(tasks: &[String]) -> (Vec<crate::run::model::Task>, crate::run::model::Profile) {
    let run = run_ok(&plan_with(PROFILE, tasks));
    (run.tasks, run.profile)
}

#[test]
fn dependency_on_a_cancelled_task() {
    let (mut tasks, profile) = built(&[one("t1", ""), one("t2", ""), one("t3", "deps = [\"t2\"]")]);
    tasks[1].state = TaskState::Cancelled;
    let touched: BTreeSet<String> = ["t3".to_string()].into();
    assert_eq!(
        validate_tasks(&tasks, &touched, &EditScope::Run, 50, &profile),
        vec![err(Some("t3"), "deps", "12.1", "t2 is cancelled")]
    );
}

#[test]
fn l_rule_applies_only_to_touched_tasks() {
    let (mut tasks, profile) = built(&[one("t1", ""), one("t2", "")]);
    tasks[0].size = proto::Size::L; // raised by rung 3, untouched by this batch
    let untouched: BTreeSet<String> = ["t2".to_string()].into();
    assert_eq!(
        validate_tasks(&tasks, &untouched, &EditScope::Run, 50, &profile),
        vec![]
    );
    let touched: BTreeSet<String> = ["t1".to_string()].into();
    assert_eq!(
        validate_tasks(&tasks, &touched, &EditScope::Run, 50, &profile),
        vec![err(
            Some("t1"),
            "size",
            "7.2.4",
            "L tasks are never executed; split the task (rule 7.2.4)"
        )]
    );
}

#[test]
fn area_scope_rejects_owns_outside_it() {
    let (tasks, profile) = built(&[
        task_toml("t3", "S", r#"["crates/daemon/src/run/x.rs"]"#, ""),
        task_toml("t4", "S", r#"["crates/tui/**"]"#, ""),
    ]);
    let touched: BTreeSet<String> = ["t3".to_string(), "t4".to_string()].into();
    let area = EditScope::Area {
        globs: vec!["crates/daemon/**".to_string()],
    };
    assert_eq!(
        validate_tasks(&tasks, &touched, &area, 50, &profile),
        vec![err(
            Some("t4"),
            "owns",
            "12.1",
            "crates/tui/** is outside the area crates/daemon/**"
        )]
    );

    let bad_area = EditScope::Area {
        globs: vec!["crates/*/src".to_string()],
    };
    assert_eq!(
        validate_tasks(&tasks[..1], &touched, &bad_area, 50, &profile),
        vec![err(
            None,
            "area",
            "12.1",
            "crates/*/src must be <literal>/** or a literal path"
        )]
    );
}

#[test]
fn implicit_dep_never_contradicts_a_declared_one() {
    // t1 declares it waits for t3 (through t2); t3 overlaps t1. Plan order alone would
    // make t3 wait for t1, a deadlock, so t3 gets no implicit dependency on t1.
    let run = run_ok(&plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", r#"["crates/a/src/**"]"#, "deps = [\"t2\"]"),
            task_toml("t2", "S", r#"["crates/b/src/lib.rs"]"#, "deps = [\"t3\"]"),
            task_toml("t3", "S", r#"["crates/a/src/lib.rs"]"#, ""),
            task_toml("t4", "S", r#"["crates/a/**"]"#, ""),
        ],
    ));
    assert_eq!(task(&run, "t3").implicit_deps, Vec::<String>::new());
    assert_eq!(
        task(&run, "t4").implicit_deps,
        vec!["t1".to_string(), "t3".to_string()]
    );
}
