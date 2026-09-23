//! M8a.11: the role contracts and the worker prompt.

use super::*;
use crate::launch::codex::toml_string;
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

/// M8a.7 moved this half of `toml_string_round_trips_through_the_toml_crate` here: the
/// contracts reach Codex as `-c developer_instructions=<toml_string(contract)>`.
#[test]
fn contracts_round_trip_through_toml_string() {
    assert!(WORKER_CONTRACT.starts_with("You are a worker in an anthrex orchestration run.\n1. "));
    assert!(
        REVIEWER_CONTRACT.starts_with("You are a reviewer in an anthrex orchestration run.\n1. ")
    );
    for contract in [WORKER_CONTRACT, REVIEWER_CONTRACT] {
        let text = format!("x = {}", toml_string(contract));
        let table: toml::Table = toml::from_str(&text).expect("the contract is a TOML string");
        assert_eq!(table["x"].as_str(), Some(contract));
    }
    assert!(WORKER_CONTRACT.ends_with("call task_blocked with kind environment."));
    assert_eq!(WORKER_CONTRACT.lines().count(), 10);
    assert_eq!(REVIEWER_CONTRACT.lines().count(), 7);
}

fn prompt_run(profile: &str, extra: &str) -> Run {
    let mut run = run_ok(&plan_with(
        profile,
        &[task_toml(
            "t1",
            "S",
            "[\"crates/a/src/x.rs\", \"crates/a/tests/**\"]",
            extra,
        )],
    ));
    run.tasks[0].start_commit = Some("0123456789abcdef".repeat(2) + "01234567");
    run
}

#[test]
fn worker_prompt_layout() {
    let run = prompt_run(PROFILE, "test_to_write = \"a::expires\"");
    let prompt = worker_prompt(&run, &run.tasks[0]);
    assert!(
        prompt.starts_with("[anthrex] Task t1: Title t1\n"),
        "{prompt}"
    );
    assert!(prompt.ends_with("\n\nBrief t1"), "{prompt}");
    let lines: Vec<&str> = prompt.lines().collect();
    assert_eq!(
        lines,
        vec![
            "[anthrex] Task t1: Title t1",
            "Run goal: Test goal",
            "Worktree: /tmp/wt/runs/add-password-reset-3f9a/t1",
            "Branch: anthrex/add-password-reset-3f9a/t1",
            "Start commit: 0123456",
            "Size: S",
            "Test mode: tdd",
            "Test to write: a::expires",
            "Single-test command: cargo test -- --exact {test}",
            "Check command: cargo test",
            "",
            "This task owns:",
            "- crates/a/src/x.rs",
            "- crates/a/tests/**",
            "Acceptance criteria:",
            "- Accept t1",
            "",
            "Brief t1",
        ]
    );

    // No check in the profile: no check line. A check-mode task: no single-test line.
    let no_check = PROFILE.replace("check = \"cargo test\"\n", "");
    let run = prompt_run(&no_check, "");
    let prompt = worker_prompt(&run, &run.tasks[0]);
    assert!(!prompt.contains("Check command:"), "{prompt}");
    assert!(
        prompt.contains("\nTest mode: tdd\nSingle-test command: "),
        "{prompt}"
    );
    assert!(!prompt.contains("Test to write:"), "{prompt}");
    let run = prompt_run(
        PROFILE,
        "test_mode = \"check\"\ntest_mode_reason = \"wiring only\"",
    );
    let prompt = worker_prompt(&run, &run.tasks[0]);
    assert!(
        prompt.contains("\nTest mode: check\nCheck command: cargo test\n"),
        "{prompt}"
    );
    assert!(!prompt.contains("Single-test command:"), "{prompt}");
    // A hub task says so.
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("h", "M", "[\"crates/proto/**\"]", "")],
    ));
    run.tasks[0].start_commit = Some("f".repeat(40));
    let prompt = worker_prompt(&run, &run.tasks[0]);
    assert!(prompt.contains("\nSize: M, hub\n"), "{prompt}");

    for tool in ["task_done", "task_blocked"] {
        assert!(WORKER_CONTRACT.contains(tool), "{tool}");
    }
    assert!(REVIEWER_CONTRACT.contains("submit_review"));
}
