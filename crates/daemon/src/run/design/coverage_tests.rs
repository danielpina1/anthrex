//! Task M9.6.4: decisions 18 and 19's plan checks, in their order (coverage, dangling
//! `covers`, brief shape), with the exact refusal texts.

use proto::{DesignMode, TaskState};

use super::{BRIEF_HEADINGS, check, table};
use crate::run::design::requirements::Requirement;
use crate::run::model::{Run, Task};
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

/// A brief with all five headings.
pub(crate) fn brief(what: &str) -> String {
    format!(
        "{what}\nFiles:\n- crates/a/src/lib.rs\nTests first:\n- a_test\nSteps:\n1. do it\nAcceptance:\n- it works\nVerify:\ncargo test -p a\n"
    )
}

pub(crate) fn reqs(ids: &[&str]) -> Vec<Requirement> {
    ids.iter()
        .map(|id| Requirement {
            id: id.to_string(),
            text: format!("text of {id}"),
        })
        .collect()
}

pub(crate) fn task_mut<'a>(run: &'a mut Run, id: &str) -> &'a mut Task {
    run.tasks
        .iter_mut()
        .find(|t| t.spec.id == id)
        .unwrap_or_else(|| panic!("no task {id}"))
}

/// A design run of three tasks, each with a full brief, covering `covers[i]`.
pub(crate) fn design_run(covers: [&[&str]; 3]) -> Run {
    let tasks: Vec<String> = ["t1", "t2", "t3"]
        .iter()
        .enumerate()
        .map(|(i, id)| task_toml(id, "S", &format!("[\"crates/m{i}/**\"]"), ""))
        .collect();
    let mut run = run_ok(&plan_with(PROFILE, &tasks));
    run.design_mode = DesignMode::Full;
    for (id, covers) in ["t1", "t2", "t3"].iter().zip(covers) {
        let task = task_mut(&mut run, id);
        task.spec.brief = brief(&format!("Brief {id}"));
        task.spec.covers = covers.iter().map(|c| c.to_string()).collect();
    }
    run
}

#[test]
fn a_covered_plan_passes() {
    let run = design_run([&["R1"], &["R2", "R3"], &[]]);
    assert_eq!(check(&run, &reqs(&["R1", "R2", "R3"])), None);
}

#[test]
fn coverage_lists_uncovered_requirements() {
    let run = design_run([&["R1"], &["R3"], &[]]);
    assert_eq!(
        check(&run, &reqs(&["R1", "R2", "R3", "R4"])),
        Some("R2, R4 are covered by no task".to_string())
    );
    assert_eq!(
        check(&run, &reqs(&["R1", "R2", "R3"])),
        Some("R2 are covered by no task".to_string())
    );
    // A removed (cancelled) task covers nothing.
    let mut run = design_run([&["R1"], &["R2"], &[]]);
    task_mut(&mut run, "t2").state = TaskState::Cancelled;
    assert_eq!(
        check(&run, &reqs(&["R1", "R2"])),
        Some("R2 are covered by no task".to_string())
    );
}

#[test]
fn dangling_covers_are_refused() {
    let run = design_run([&["R1", "R12"], &["R2"], &["R9"]]);
    assert_eq!(
        check(&run, &reqs(&["R1", "R2"])),
        Some(
            "task t1 covers R12, which the spec does not have; \
             task t3 covers R9, which the spec does not have"
                .to_string()
        )
    );
    // Coverage is checked first.
    assert_eq!(
        check(&run, &reqs(&["R1", "R2", "R3"])),
        Some("R3 are covered by no task".to_string())
    );
}

#[test]
fn brief_headings_are_checked() {
    assert_eq!(
        BRIEF_HEADINGS,
        ["Files:", "Tests first:", "Steps:", "Acceptance:", "Verify:"]
    );
    for heading in BRIEF_HEADINGS {
        let mut run = design_run([&["R1"], &[], &[]]);
        let task = task_mut(&mut run, "t2");
        task.spec.brief = task.spec.brief.replace(&format!("{heading}\n"), "");
        assert_eq!(
            check(&run, &reqs(&["R1"])),
            Some(format!(
                "task t2's brief is missing the heading \"{heading}\""
            )),
            "{heading}"
        );
    }
    // Each task is named, with its first missing heading.
    let mut run = design_run([&["R1"], &[], &[]]);
    task_mut(&mut run, "t1").spec.brief = "Files: inline is not its own line\n".to_string();
    task_mut(&mut run, "t3").spec.brief = brief("x").replace("Verify:", "Verify: later");
    assert_eq!(
        check(&run, &reqs(&["R1"])),
        Some(
            "task t1's brief is missing the heading \"Files:\"; \
             task t3's brief is missing the heading \"Verify:\""
                .to_string()
        )
    );
    // Dangling covers come before the brief shape.
    task_mut(&mut run, "t2").spec.covers = vec!["R7".into()];
    assert_eq!(
        check(&run, &reqs(&["R1"])),
        Some("task t2 covers R7, which the spec does not have".to_string())
    );
    // Surrounding whitespace on a heading's line is allowed.
    let mut run = design_run([&["R1"], &[], &[]]);
    task_mut(&mut run, "t1").spec.brief = brief("x").replace("Steps:", "  Steps:  ");
    assert_eq!(check(&run, &reqs(&["R1"])), None);
}

#[test]
fn a_run_without_the_design_flow_is_never_checked() {
    let mut run = design_run([&["R9"], &[], &[]]);
    task_mut(&mut run, "t1").spec.brief = "no headings".to_string();
    run.design_mode = DesignMode::Off;
    assert_eq!(check(&run, &reqs(&["R1"])), None);
}

#[test]
fn the_table_maps_each_requirement_to_its_tasks_in_run_order() {
    let mut run = design_run([&["R2", "R1"], &["R1"], &["R9"]]);
    task_mut(&mut run, "t2").state = TaskState::Cancelled;
    assert_eq!(
        table(&run, &reqs(&["R1", "R2", "R3"])),
        vec![
            ("R1".to_string(), vec!["t1".to_string()]),
            ("R2".to_string(), vec!["t1".to_string()]),
            ("R3".to_string(), vec![]),
        ]
    );
}
