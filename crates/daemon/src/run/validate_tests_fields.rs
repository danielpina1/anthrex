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

fn built(tasks: &[String]) -> (Vec<crate::run::model::Task>, proto::Runtime) {
    let run = run_ok(&plan_with(PROFILE, tasks));
    (run.tasks, run.limits.default_runtime)
}

#[test]
fn dependency_on_a_cancelled_task() {
    let (mut tasks, runtime) = built(&[one("t1", ""), one("t2", ""), one("t3", "deps = [\"t2\"]")]);
    tasks[1].state = TaskState::Cancelled;
    let touched: BTreeSet<String> = ["t3".to_string()].into();
    assert_eq!(
        validate_tasks(&tasks, &touched, &EditScope::Run, 50, runtime),
        vec![err(Some("t3"), "deps", "12.1", "t2 is cancelled")]
    );
}

#[test]
fn l_rule_applies_only_to_touched_tasks() {
    let (mut tasks, runtime) = built(&[one("t1", ""), one("t2", "")]);
    tasks[0].size = proto::Size::L; // raised by rung 3, untouched by this batch
    let untouched: BTreeSet<String> = ["t2".to_string()].into();
    assert_eq!(
        validate_tasks(&tasks, &untouched, &EditScope::Run, 50, runtime),
        vec![]
    );
    let touched: BTreeSet<String> = ["t1".to_string()].into();
    assert_eq!(
        validate_tasks(&tasks, &touched, &EditScope::Run, 50, runtime),
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
    let (tasks, runtime) = built(&[
        task_toml("t3", "S", r#"["crates/daemon/src/run/x.rs"]"#, ""),
        task_toml("t4", "S", r#"["crates/tui/**"]"#, ""),
    ]);
    let touched: BTreeSet<String> = ["t3".to_string(), "t4".to_string()].into();
    let area = EditScope::Area {
        globs: vec!["crates/daemon/**".to_string()],
    };
    assert_eq!(
        validate_tasks(&tasks, &touched, &area, 50, runtime),
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
        validate_tasks(&tasks[..1], &touched, &bad_area, 50, runtime),
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

/// Review finding 1: the deadlock skip must see implicit edges already added, not only
/// declared ones. t2 waits for t1 (implicit), t1 waits for t3 (declared); giving t3 an
/// implicit dependency on t2 would close the cycle.
#[test]
fn implicit_deps_never_close_a_cycle_through_other_implicit_deps() {
    let run = run_ok(&plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", r#"["crates/a/**"]"#, "deps = [\"t3\"]"),
            task_toml("t2", "S", r#"["crates/a/src/**"]"#, ""),
            task_toml("t3", "S", r#"["crates/a/src/y.rs"]"#, ""),
        ],
    ));
    assert_eq!(task(&run, "t1").implicit_deps, Vec::<String>::new());
    assert_eq!(task(&run, "t2").implicit_deps, vec!["t1".to_string()]);
    assert_eq!(task(&run, "t3").implicit_deps, Vec::<String>::new());
    assert_eq!(combined_cycles(&run.tasks), vec![]);
}

#[test]
fn the_combined_graph_check_reports_an_implicit_cycle() {
    let (mut tasks, _) = built(&[one("t1", "deps = [\"t2\"]"), one("t2", "")]);
    tasks[1].implicit_deps = vec!["t1".to_string()];
    assert_eq!(
        combined_cycles(&tasks),
        vec![err(None, "deps", "12.1", "cycle t1 -> t2 -> t1")]
    );
}

// ---- Edit-time scoping (review finding 11); M8a.6 relies on each of these. ----

fn set(ids: &[&str]) -> BTreeSet<String> {
    ids.iter().map(|s| s.to_string()).collect()
}

#[test]
fn cross_runtime_overlap_ignores_finished_earlier_tasks() {
    let (mut tasks, runtime) = built(&[
        task_toml("t1", "S", r#"["crates/a/src/**"]"#, ""),
        task_toml("t2", "S", r#"["crates/b/src/lib.rs"]"#, ""),
    ]);
    tasks[0].state = TaskState::Merged;
    tasks[1].spec.route.runtime = Some(proto::Runtime::Codex);
    tasks[1].spec.owns = vec!["crates/a/src/lib.rs".to_string()];
    assert_eq!(
        validate_tasks(&tasks, &set(&["t2"]), &EditScope::Run, 50, runtime),
        vec![]
    );
}

#[test]
fn implicit_deps_ignore_finished_tasks() {
    let (mut tasks, _) = built(&[
        task_toml("t1", "S", r#"["crates/a/src/**"]"#, ""),
        task_toml("t2", "S", r#"["crates/a/**"]"#, ""),
        task_toml("t3", "S", r#"["crates/a/src/lib.rs"]"#, ""),
    ]);
    tasks[0].state = TaskState::Cancelled;
    assert_eq!(
        implicit_deps(&tasks),
        vec![vec![], vec![], vec!["t2".to_string()]]
    );
}

#[test]
fn implicit_deps_skip_a_declared_dependency() {
    let run = run_ok(&plan_with(
        PROFILE,
        &[
            task_toml("t1", "S", r#"["crates/a/src/**"]"#, ""),
            task_toml("t2", "S", r#"["crates/a/src/lib.rs"]"#, "deps = [\"t1\"]"),
        ],
    ));
    assert_eq!(task(&run, "t2").implicit_deps, Vec::<String>::new());
}

#[test]
fn a_cancelled_dependency_of_an_untouched_task_is_allowed() {
    let (mut tasks, runtime) = built(&[one("t1", ""), one("t2", ""), one("t3", "deps = [\"t2\"]")]);
    tasks[1].state = TaskState::Cancelled;
    assert_eq!(
        validate_tasks(&tasks, &set(&["t1"]), &EditScope::Run, 50, runtime),
        vec![]
    );
}

/// Unit 1 task, bound 2: three tasks with one cancelled count as two.
#[test]
fn max_tasks_does_not_count_cancelled_tasks() {
    let (mut tasks, runtime) = built(&[one("t1", ""), one("t2", ""), one("t3", "")]);
    tasks[1].state = TaskState::Cancelled;
    assert_eq!(
        validate_tasks(&tasks, &set(&[]), &EditScope::Run, 2, runtime),
        vec![]
    );
    tasks[1].state = TaskState::Merged;
    assert_eq!(
        validate_tasks(&tasks, &set(&[]), &EditScope::Run, 2, runtime),
        vec![err(None, "tasks", "range", "3 tasks exceed max_tasks (2)")]
    );
}

#[test]
fn the_area_rule_applies_only_to_touched_tasks() {
    let (tasks, runtime) = built(&[
        task_toml("t3", "S", r#"["crates/daemon/src/x.rs"]"#, ""),
        task_toml("t4", "S", r#"["crates/tui/**"]"#, ""),
    ]);
    let area = EditScope::Area {
        globs: vec!["crates/daemon/**".to_string()],
    };
    assert_eq!(
        validate_tasks(&tasks, &set(&["t3"]), &area, 50, runtime),
        vec![]
    );
}

#[test]
fn a_duplicate_of_a_finished_task_is_still_a_duplicate() {
    let (mut tasks, runtime) = built(&[one("t1", ""), one("t2", "")]);
    tasks[0].state = TaskState::Cancelled;
    tasks[1].spec.id = "t1".to_string();
    assert_eq!(
        validate_tasks(&tasks, &set(&["t1"]), &EditScope::Run, 50, runtime),
        vec![err(Some("t1"), "id", "id", "t1 is used by an earlier task")]
    );
}

#[test]
fn a_dependency_resolves_to_the_first_task_with_that_id() {
    // The first t2 is pending, a later duplicate t2 is cancelled: t3's dependency is
    // the first one, so the only error is the duplicate itself.
    let (mut tasks, runtime) = built(&[one("t2", ""), one("t9", ""), one("t3", "deps = [\"t2\"]")]);
    tasks[1].spec.id = "t2".to_string();
    tasks[1].state = TaskState::Cancelled;
    tasks.swap(1, 2);
    // Order now: t2 (pending), t3 (deps t2), t2 (cancelled).
    assert_eq!(
        validate_tasks(&tasks, &set(&["t3"]), &EditScope::Run, 50, runtime),
        vec![]
    );
}
