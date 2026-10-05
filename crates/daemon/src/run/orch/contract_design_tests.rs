//! Milestone 9.6 task M9.6.13 (decisions 25 and 26, DF §5.2): a design run's worker and
//! reviewer prompts carry the requirements their task covers and the spec's Goal; a
//! run without the design flow gets 9.5's prompts byte for byte.

use super::*;
use crate::run::contract::{reviewer_prompt, worker_prompt};
use crate::run::design::state::{DesignState, Requirement};
use crate::run::model::Run;
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

fn requirement(id: &str, text: &str) -> Requirement {
    Requirement {
        id: id.into(),
        text: text.into(),
    }
}

/// A run with task `t1`, its start commit fixed.
fn plain_run() -> Run {
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/src/x.rs\"]", "")],
    ));
    run.tasks[0].start_commit = Some("0123456789abcdef".repeat(2) + "01234567");
    run
}

/// [`plain_run`] with the design flow on: the approved spec's requirements and Goal
/// (as the driver's read-back stores them, task M9.6.10), `t1` covering `covers`.
fn design_run(goal: &str, covers: &[&str]) -> Run {
    let mut run = plain_run();
    run.orch.design = Some(DesignState {
        requirements: vec![
            requirement("R1", "Tokens expire after an hour. Check: a clock test."),
            requirement("R2", "Links are single use. Check: a reuse test."),
            requirement("R3", "Mail is sent once. Check: a mail test."),
        ],
        goal_section: goal.to_string(),
        ..DesignState::default()
    });
    run.tasks[0].spec.covers = covers.iter().map(|c| c.to_string()).collect();
    run
}

const BLOCK: &str = "Spec requirements this task delivers:
R1  Tokens expire after an hour. Check: a clock test.
R3  Mail is sent once. Check: a mail test.
Goal (from the spec): Users reset their password.";

/// Decision 25: the block, in the spec's order, after the scout extract and before the
/// orchestrator's notes and the brief; a long Goal is cut so the block stays within
/// 6 KiB, its cut marked.
#[test]
fn a_workers_prompt_has_its_requirements_and_the_goal_capped() {
    let run = design_run("Users reset their password.", &["R3", "R1"]);
    let block = requirements_block(&run, &run.tasks[0]);
    assert_eq!(block.as_deref(), Some(BLOCK));
    let prompt = worker_prompt(&run, &run.tasks[0], "Scout report s1: x", "Notes: y");
    assert!(
        prompt.ends_with(&format!(
            "- Accept t1\n\nScout report s1: x\n\n{BLOCK}\n\nNotes: y\n\nBrief t1"
        )),
        "{prompt}"
    );
    let long = "goal ".repeat(2_000);
    let run = design_run(&long, &["R1"]);
    let block = requirements_block(&run, &run.tasks[0]).unwrap();
    assert!(block.len() <= REQUIREMENTS_MAX, "{}", block.len());
    assert!(block.starts_with(
        "Spec requirements this task delivers:\nR1  Tokens expire after an hour. Check: a clock test.\nGoal (from the spec): goal goal"
    ));
    let full = format!(
        "Spec requirements this task delivers:\nR1  Tokens expire after an hour. Check: a clock test.\nGoal (from the spec): {long}"
    );
    let kept = block.find("\n[cut: ").unwrap();
    assert!(
        block.ends_with(&format!("\n[cut: {} bytes]", full.len() - kept)),
        "{}",
        &block[kept..]
    );
    // A task with no covers, or covering nothing the spec has, gets no block.
    let run = design_run("Users reset their password.", &[]);
    assert_eq!(requirements_block(&run, &run.tasks[0]), None);
    let run = design_run("Users reset their password.", &["R9"]);
    assert_eq!(requirements_block(&run, &run.tasks[0]), None);
}

const WORKER: &str = "[anthrex] Task t1: Title t1
Run goal: Test goal
Worktree: /tmp/wt/runs/add-password-reset-3f9a/t1
Branch: anthrex/add-password-reset-3f9a/t1
Start commit: 0123456
Size: S
Test mode: tdd
Single-test command: cargo test -- --exact {test}
Check command: cargo test

This task owns:
- crates/a/src/x.rs
Acceptance criteria:
- Accept t1

Scout report s1: x

Notes: y

Brief t1";

const REVIEWER: &str = "[anthrex] Review task t1 \"Title t1\", round 1, level small.
Base: ccccccc
Head: ddddddd
Test mode: tdd
Look first for tests that were weakened or made trivial to pass.
Review the diff only.
Acceptance criteria:
- Accept t1
Diff (ccccccc..ddddddd):
+x
Messages the worker received:
- 13:07 (change) use the v2 token API

Brief t1";

fn reviewer(run: &Run) -> String {
    let messages = "Messages the worker received:\n- 13:07 (change) use the v2 token API";
    let (base, head) = ("c".repeat(40), "d".repeat(40));
    reviewer_prompt(run, &run.tasks[0], 1, &base, &head, "+x", messages)
}

/// The addendum's non-design runs: 9.5's worker and reviewer prompts, byte for byte,
/// even when a task carries `covers` (a plan file may name them; only a design run's
/// approved spec gives them meaning).
#[test]
fn a_non_design_task_prompt_is_unchanged() {
    let mut run = plain_run();
    run.tasks[0].spec.covers = vec!["R1".into()];
    let worker = worker_prompt(&run, &run.tasks[0], "Scout report s1: x", "Notes: y");
    assert_eq!(worker, WORKER);
    assert_eq!(reviewer(&run), REVIEWER);
    assert_eq!(requirements_block(&run, &run.tasks[0]), None);
}

/// Decision 26: the reviewer gets the same block and is asked to judge the change
/// against each requirement, citing its id; the verdict is unchanged.
#[test]
fn the_reviewer_prompt_asks_for_a_verdict_per_requirement() {
    let run = design_run("Users reset their password.", &["R1", "R3"]);
    let prompt = reviewer(&run);
    let expected =
        format!("- 13:07 (change) use the v2 token API\n{BLOCK}\n{JUDGE_LINE}\n\nBrief t1");
    assert!(prompt.ends_with(&expected), "{prompt}");
    assert_eq!(
        JUDGE_LINE,
        "Judge the change against each requirement above; cite its id (R2) in any finding that concerns it."
    );
    let plain = design_run("Users reset their password.", &[]);
    assert_eq!(reviewer(&plain), REVIEWER);
}

/// Final fix wave FW-72 (WB-C M-5): `run start --goal`'s message for a design run says
/// the run is brainstorming and its orchestrator may ask questions in its window; a run
/// without the flow gets 9.5's text byte for byte.
#[test]
fn a_design_runs_start_message_mentions_the_questions() {
    let info = proto::TriageInfo {
        kinds: vec![proto::TaskKind::Code],
        scale: proto::Scale::Plan,
        path: proto::RunPath::Plan,
        reason: "two modules".into(),
        source: proto::DeciderSource::Decider,
        fallback_reason: None,
        at: 0,
    };
    let path = proto::RunPath::Plan;
    assert_eq!(
        start_message(&info, "run-1", path, DesignMode::Full),
        "triage: code/plan (decider)\nplan path: run run-1 is brainstorming with its orchestrator, which may ask you questions in its window\ntalk to it with: anthrex, then C-b T and Enter on the run\nwatch with: anthrex run status run-1"
    );
    assert_eq!(
        start_message(&info, "run-1", path, DesignMode::Off),
        crate::run::orch::contract::planned_message(&info, "run-1", path)
    );
}

/// Final fix wave FW-69 (WB-C M-1): a paired task's test writer is given the
/// requirements its task covers and the spec's Goal, as its implementer is, before the
/// brief, in its first turn and in a fresh session's hand-over; without the design flow
/// its prompt is 9.5's.
#[test]
fn a_paired_test_writer_gets_the_requirements() {
    use crate::run::contract_patterns::{test_writer_handover, test_writer_prompt};
    let run = design_run("Users reset their password.", &["R3", "R1"]);
    let prompt = test_writer_prompt(&run, &run.tasks[0]);
    assert!(
        prompt.ends_with(&format!("- Accept t1\n\n{BLOCK}\n\nBrief t1")),
        "{prompt}"
    );
    let handover = test_writer_handover(&run, &run.tasks[0], "lost", "stat", "patch");
    assert!(handover.starts_with(&prompt), "{handover}");
    let plain = plain_run();
    let before = test_writer_prompt(&plain, &plain.tasks[0]);
    assert!(before.ends_with("- Accept t1\n\nBrief t1"), "{before}");
    assert_eq!(prompt.replace(&format!("{BLOCK}\n\n"), ""), before);
}
