//! Task M9.6.4: decision 21's `plan.md`, as a snapshot string.

use proto::{DesignMode, Effort, Route, Runtime, Size, Strength, TaskState, TestMode};

use super::render;
use crate::run::design::requirements::Requirement;
use crate::run::model::{Run, Task};
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

fn task_mut<'a>(run: &'a mut Run, id: &str) -> &'a mut Task {
    run.tasks
        .iter_mut()
        .find(|t| t.spec.id == id)
        .unwrap_or_else(|| panic!("no task {id}"))
}

fn route(runtime: Runtime, model: &str, effort: Effort) -> Route {
    Route {
        runtime,
        model: model.to_string(),
        strength: Strength::Standard,
        effort,
    }
}

fn reqs() -> Vec<Requirement> {
    ["R1", "R2", "R3"]
        .iter()
        .map(|id| Requirement {
            id: id.to_string(),
            text: format!("text of {id}"),
        })
        .collect()
}

fn run() -> Run {
    let tasks: Vec<String> = ["t1", "t2", "t3", "t4"]
        .iter()
        .enumerate()
        .map(|(i, id)| task_toml(id, "S", &format!("[\"crates/m{i}/**\"]"), ""))
        .collect();
    let mut run = run_ok(&plan_with(PROFILE, &tasks));
    run.goal = "Add password reset\nwith a second line that is not the head".to_string();
    run.design_mode = DesignMode::Full;
    let setups = [
        (
            "t1",
            2,
            "Token model",
            &["R1", "R2"][..],
            Size::M,
            TestMode::Tdd,
        ),
        (
            "t2",
            1,
            "Reset \x1b[31mlink",
            &["R2"][..],
            Size::S,
            TestMode::Check,
        ),
        ("t3", 1, "Mail", &[][..], Size::L, TestMode::None),
        ("t4", 1, "Removed", &["R3"][..], Size::S, TestMode::Tdd),
    ];
    for (id, stage, title, covers, size, mode) in setups {
        let task = task_mut(&mut run, id);
        task.spec.stage = stage;
        task.spec.title = title.to_string();
        task.spec.covers = covers.iter().map(|c| c.to_string()).collect();
        task.spec.brief = format!("Do {id}.\nFiles:\n- a.rs\r\nVerify:\ncargo test");
        task.size = size;
        task.test_mode = mode;
        task.route = route(Runtime::Claude, "claude-sonnet-5", Effort::High);
    }
    task_mut(&mut run, "t2").route = route(Runtime::Codex, "", Effort::Medium);
    task_mut(&mut run, "t4").state = TaskState::Cancelled;
    run
}

#[test]
fn plan_md_renders_stages_tasks_and_the_coverage_table() {
    let expected = "\
# Plan: Add password reset with a second line that is not the head

## Stage 1

### t2 Reset  [31mlink
Covers: R2
Size: S · Route: codex (default) medium · Tests: check

Do t2.
Files:
- a.rs
Verify:
cargo test

### t3 Mail
Covers: none
Size: L · Route: claude claude-sonnet-5 high · Tests: none

Do t3.
Files:
- a.rs
Verify:
cargo test

## Stage 2

### t1 Token model
Covers: R1, R2
Size: M · Route: claude claude-sonnet-5 high · Tests: tdd

Do t1.
Files:
- a.rs
Verify:
cargo test

## Coverage

| Requirement | Tasks |
|---|---|
| R1 | t1 |
| R2 | t1, t2 |
| R3 | none |
";
    assert_eq!(render(&run(), &reqs()), expected);
}

#[test]
fn the_title_is_the_goal_head_cut_to_60_characters() {
    let mut run = run();
    run.goal = "g".repeat(200);
    let md = render(&run, &reqs());
    assert_eq!(
        md.lines().next(),
        Some(format!("# Plan: {}", "g".repeat(60)).as_str())
    );
}

#[test]
fn a_plan_without_requirements_has_an_empty_table() {
    let md = render(&run(), &[]);
    assert!(
        md.ends_with("## Coverage\n\n| Requirement | Tasks |\n|---|---|\n"),
        "{md}"
    );
}
