//! Milestone 9.1 decisions 43, 44 and 54 in a plan file (task M9.1.11): stage numbers,
//! dependencies across stages, and atomic tasks. The reserved ids of every source are
//! `engine/tests/plan_stages.rs`; amends and the rules over an edited run are
//! `edits_tests_stages.rs`.

use proto::{Size, TaskKind, TestMode};

use crate::run::model::{ReviewLevel, Run, StageLayout};
use crate::run::test_support::*;

/// One S task owning `crates/<id>/src/lib.rs`, with `extra` appended.
fn one(id: &str, extra: &str) -> String {
    task_toml(id, "S", &format!("[\"crates/{id}/src/lib.rs\"]"), extra)
}

fn stages(run: &Run) -> Vec<(&str, u16)> {
    run.tasks.iter().map(|t| (t.id(), t.spec.stage)).collect()
}

#[test]
fn stage_dependency_on_a_later_stage_is_rejected() {
    let text = plan_with(
        PROFILE,
        &[one("t1", "deps = [\"t9\"]"), one("t9", "stage = 2")],
    );
    assert_eq!(
        errors_of(&text),
        [err(
            Some("t1"),
            "stage",
            "4.1",
            "stage 1 cannot depend on t9 in stage 2"
        )]
    );
    // TT §4.1's own example, as `PlanError` displays it.
    let text = plan_with(
        PROFILE,
        &[
            one("t1", ""),
            one("t4", "stage = 2\ndeps = [\"t9\"]"),
            one("t9", "stage = 3"),
        ],
    );
    let shown: Vec<String> = errors_of(&text).iter().map(ToString::to_string).collect();
    assert_eq!(
        shown,
        ["task t4: stage: stage 2 cannot depend on t9 in stage 3"]
    );

    // The same stage and an earlier one are accepted, and the stages survive
    // `build_run`.
    let run = run_ok(&plan_with(
        PROFILE,
        &[
            one("t1", ""),
            one("t2", "deps = [\"t1\"]"),
            one("t3", "stage = 2\ndeps = [\"t1\", \"t2\"]"),
            one("t4", "stage = 2\ndeps = [\"t3\"]"),
        ],
    ));
    assert_eq!(stages(&run), [("t1", 1), ("t2", 1), ("t3", 2), ("t4", 2)]);
}

#[test]
fn stages_must_be_contiguous_and_non_empty() {
    let text = plan_with(PROFILE, &[one("t1", ""), one("t3", "stage = 3")]);
    assert_eq!(
        errors_of(&text),
        [err(
            None,
            "stage",
            "4.1",
            "stages must be numbered from 1 without gaps: stage 2 has no task"
        )]
    );
    // Stage 1 must have a task too; each missing stage is named.
    let text = plan_with(PROFILE, &[one("t2", "stage = 2"), one("t4", "stage = 4")]);
    let gap = |k: u16| {
        err(
            None,
            "stage",
            "4.1",
            &format!("stages must be numbered from 1 without gaps: stage {k} has no task"),
        )
    };
    assert_eq!(errors_of(&text), [gap(1), gap(3)]);

    let run = run_ok(&plan_with(
        PROFILE,
        &[
            one("t1", "stage = 3"),
            one("t2", "stage = 1"),
            one("t3", "stage = 2"),
        ],
    ));
    assert_eq!(stages(&run), [("t1", 3), ("t2", 1), ("t3", 2)]);
}

#[test]
fn stage_must_be_between_1_and_32() {
    for stage in [0, 33, 1000] {
        let text = plan_with(
            PROFILE,
            &[one("t1", ""), one("t2", &format!("stage = {stage}"))],
        );
        // Only the range error: an out-of-range stage opens no gap.
        assert_eq!(
            errors_of(&text),
            [err(Some("t2"), "stage", "4.1", "must be between 1 and 32")],
            "stage {stage}"
        );
    }
    let mut tasks = Vec::new();
    for n in 1..=32 {
        tasks.push(task_toml(
            &format!("t{n}"),
            "S",
            &format!("[\"crates/c{n}/src/lib.rs\"]"),
            &format!("stage = {n}"),
        ));
    }
    let run = run_ok(&plan_with(PROFILE, &tasks));
    assert_eq!(run.tasks.last().unwrap().spec.stage, 32);
}

#[test]
fn two_atomic_tasks_in_one_stage_are_rejected() {
    let atomic = "atomic = true\natomic_reason = \"protocol bump\"";
    let text = plan_with(
        PROFILE,
        &[one("t1", atomic), one("t2", ""), one("t3", atomic)],
    );
    assert_eq!(
        errors_of(&text),
        [err(
            Some("t3"),
            "atomic",
            "4.3",
            "stage 1 already has the atomic task t1"
        )]
    );
    // One per stage is accepted.
    let run = run_ok(&plan_with(
        PROFILE,
        &[
            one("t1", atomic),
            one("t3", &format!("stage = 2\n{atomic}")),
        ],
    ));
    assert!(run.tasks.iter().all(|t| t.spec.atomic));
}

#[test]
fn atomic_needs_a_reason() {
    for reason in ["", "atomic_reason = \"\"", "atomic_reason = \"  \""] {
        let text = plan_with(PROFILE, &[one("t1", &format!("atomic = true\n{reason}"))]);
        assert_eq!(
            errors_of(&text),
            [err(
                Some("t1"),
                "atomic_reason",
                "4.3",
                "required when atomic is true"
            )],
            "{reason:?}"
        );
    }
    // A reason without `atomic` is harmless.
    run_ok(&plan_with(
        PROFILE,
        &[one("t1", "atomic_reason = \"not atomic\"")],
    ));
}

/// Decision 54: an atomic task is a hub task (tdd, frontier review, at least M), and
/// spanning modules with `interface_change` does not raise it to L.
#[test]
fn an_atomic_task_is_a_hub_task_that_keeps_its_size() {
    let owns = r#"["crates/a/src/lib.rs", "crates/b/src/lib.rs"]"#;
    let extra = "interface_change = true\ntest_mode = \"check\"\ntest_mode_reason = \"r\"";
    let atomic = format!("{extra}\natomic = true\natomic_reason = \"protocol bump\"");
    let run = run_ok(&plan_with(PROFILE, &[task_toml("t1", "S", owns, &atomic)]));
    let t1 = task(&run, "t1");
    assert!(t1.hub);
    assert_eq!(t1.size, Size::M);
    assert_eq!(t1.spec.kind, TaskKind::Code);
    assert_eq!(t1.test_mode, TestMode::Tdd);
    assert_eq!(t1.review_level, Some(ReviewLevel::Frontier));
    assert_eq!(t1.route.strength, proto::Strength::Frontier);

    // The control: the same task, not atomic, is raised to L and refused.
    let errors = errors_of(&plan_with(PROFILE, &[task_toml("t1", "S", owns, extra)]));
    assert!(
        errors.iter().any(|e| e.rule == "7.2.4"),
        "{}",
        show(&errors)
    );
}

/// Pinning: a plan that names no stage is one stage. Decision 46's `stage_layout` is
/// added by M9.1.12 (see "Implementation notes", task M9.1.11).
#[test]
fn plans_without_stages_are_one_stage() {
    for text in [
        EXAMPLE_PLAN.to_string(),
        plan_with(PROFILE, &[one("t1", "")]),
    ] {
        let run = run_ok(&text);
        assert!(!run.tasks.is_empty());
        for t in &run.tasks {
            assert_eq!(t.spec.stage, 1, "{}", t.id());
            assert!(!t.spec.atomic);
            assert_eq!(t.spec.atomic_reason, None);
        }
    }
    // Milestone 9.1 decision 46 (task M9.1.12): and they are one stage when approved.
    let mut run = run_ok(&plan_with(PROFILE, &[one("t1", ""), one("t2", "")]));
    assert_eq!(run.stage_layout, StageLayout::Single);
    assert_eq!(run.stages.len(), 1);
    assert_eq!(run.stage_branch(1), run.run_branch());
    run.state = proto::RunState::Running;
    assert!(crate::run::validate_stages::single_layout_rule(&run).is_empty());
    // M8b's stored run, written before stages existed.
    let run: Run =
        serde_json::from_str(include_str!("engine/tests/m8b_run.json")).expect("m8b run.json");
    assert!(!run.tasks.is_empty());
    assert!(
        run.tasks
            .iter()
            .all(|t| t.spec.stage == 1 && !t.spec.atomic)
    );
}

/// Controller ruling C-14 (b): decision 41's implicit dependencies are stage-aware. A
/// later-stage task waits for an overlapping earlier-stage one whatever the plan order,
/// an earlier stage never waits for a later one, and within a stage the plan order
/// decides as before.
#[test]
fn implicit_deps_point_only_to_the_same_or_an_earlier_stage() {
    let shared = |id: &str, extra: &str| task_toml(id, "S", "[\"crates/x/**\"]", extra);
    let implicit = |run: &Run, id: &str| run.task(id).unwrap().implicit_deps.clone();
    // The stage-2 task comes first in plan order, yet it waits for the stage-1 one.
    let run = run_ok(&plan_with(
        PROFILE,
        &[shared("t1", "stage = 2"), shared("t2", "")],
    ));
    assert_eq!(implicit(&run, "t1"), vec!["t2".to_string()]);
    assert!(implicit(&run, "t2").is_empty());
    // In plan order, the same.
    let run = run_ok(&plan_with(
        PROFILE,
        &[shared("t1", ""), shared("t2", "stage = 2")],
    ));
    assert!(implicit(&run, "t1").is_empty());
    assert_eq!(implicit(&run, "t2"), vec!["t1".to_string()]);
    // Within one stage, the later in plan order waits, as decision 41 says.
    let run = run_ok(&plan_with(
        PROFILE,
        &[
            shared("t1", "stage = 2"),
            shared("t2", ""),
            shared("t3", "stage = 2"),
        ],
    ));
    assert_eq!(implicit(&run, "t1"), vec!["t2".to_string()]);
    assert_eq!(
        implicit(&run, "t3"),
        vec!["t1".to_string(), "t2".to_string()]
    );
    // A started stage-2 task never makes a stage-1 task wait for it.
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[shared("t1", "stage = 2"), shared("t2", "")],
    ));
    run.tasks[0].state = proto::TaskState::Working;
    run.tasks[0].start_commit = Some(run.base_sha.clone());
    let deps = crate::run::validate_graph::implicit_deps(&run.tasks);
    assert_eq!(deps, vec![Vec::<String>::new(), Vec::new()]);
}
