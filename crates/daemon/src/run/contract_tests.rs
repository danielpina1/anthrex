//! M8a.11: the role contracts and the worker prompt, and the diff clamp's tests
//! (moved here from `contract.rs` in M9.5 to keep that file under 600 lines).

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
    assert!(WORKER_CONTRACT.ends_with("Do not try to fix it or change git settings."));
    assert_eq!(WORKER_CONTRACT.lines().count(), 13);
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
    let prompt = worker_prompt(&run, &run.tasks[0], "", "");
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
    let prompt = worker_prompt(&run, &run.tasks[0], "", "");
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
    let prompt = worker_prompt(&run, &run.tasks[0], "", "");
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
    let prompt = worker_prompt(&run, &run.tasks[0], "", "");
    assert!(prompt.contains("\nSize: M, hub\n"), "{prompt}");

    for tool in ["task_done", "task_blocked"] {
        assert!(WORKER_CONTRACT.contains(tool), "{tool}");
    }
    assert!(REVIEWER_CONTRACT.contains("submit_review"));
}

/// The Claude tool-search fix (2026-09-27): each contract names its tools by the full
/// id a Claude session sees, so a model that searches for a tool searches for that id.
/// Codex names MCP tools differently, so the id is marked as Claude's.
#[test]
fn contracts_name_each_tool_by_its_claude_id() {
    for tool in ["task_done", "task_blocked"] {
        let named = format!("{tool} (in Claude: mcp__anthrex__{tool})");
        assert!(WORKER_CONTRACT.contains(&named), "{named}");
    }
    let named = "submit_review (in Claude: mcp__anthrex__submit_review)";
    assert!(REVIEWER_CONTRACT.contains(named), "{named}");
}

/// M8a's worker contract with decision 41's lines 10 and 11 (Interfaces "Contracts")
/// and task M9.13c's line 12 (M9.1 check 11: the `packed-refs.lock` warning), byte for
/// byte.
#[test]
fn worker_contract_is_exact() {
    assert_eq!(WORKER_CONTRACT, WORKER_EXPECTED);
}

const WORKER_EXPECTED: &str = r#"You are a worker in an anthrex orchestration run.
1. Work only in this worktree and only in the paths this task owns. Changing files outside them stops the task.
2. Follow the test mode in your task prompt. For tdd: write the named test first, commit it while it fails (that commit is the red commit), then make it pass.
3. Commit your work in this worktree with clear messages. Its HEAD is detached: commit on it, and the engine records your commits on the task's branch. Never create, switch or push branches, and never rewrite commits already there. Commit new files: untracked files are not part of your work.
4. Use sub-agents to read and explore if you like; do all writing yourself.
5. When the task is complete and committed, call the anthrex tool task_done (in Claude: mcp__anthrex__task_done) with a summary (and, for tdd, the test and the red commit). Then stop.
6. If you cannot continue, call task_blocked (in Claude: mcp__anthrex__task_blocked): kind question if you need an answer, mis_sized if the task is bigger than one task, environment if a tool or setup is broken. Then stop.
7. If you believe a review finding is wrong, do not fix it: call task_blocked with kind question and say why.
8. Messages that start with [anthrex] come from the orchestration engine. Do what they say, commit, and call task_done again.
9. Nobody can answer a permission prompt. If a tool is denied, work without it or call task_blocked with kind environment.
10. If you learn something that affects other tasks or the plan, such as another place that must change, a wrong assumption in the brief, or a risk, report it with task_note (in Claude: mcp__anthrex__task_note) and keep working. Use task_blocked only when you cannot continue.
11. A message of kind change means the plan or the code around this task changed: in your next task_done summary, start with Changes applied: and say how you applied it. A message of kind stop_and_wait means finish your current step, commit anything worth keeping, and end your turn without calling task_done; wait for the next message.
12. After a commit, git may print Unable to create '.../packed-refs.lock': Operation not permitted. That is expected: the commit succeeded, and it needs no action. Do not try to fix it or change git settings."#;

/// Decision 42d and ruling D-11: the contract asks a worker to acknowledge a change
/// message in its next task_done summary; the engine never checks the summary.
#[test]
fn change_message_requires_acknowledgement_in_task_done() {
    let line = WORKER_CONTRACT.lines().nth(11).expect("line 11");
    assert!(line.starts_with("11. A message of kind change "), "{line}");
    assert!(
        line.contains("in your next task_done summary, start with Changes applied:"),
        "{line}"
    );
    assert!(
        line.contains("A message of kind stop_and_wait means"),
        "{line}"
    );
}

/// Task M9.13c (M9.1 check 11): a sandboxed worker's commit may warn that git cannot
/// create `packed-refs.lock`. The commit succeeded; the contract says so, so the worker
/// neither "fixes" it nor changes git settings, and the sandbox is not widened.
#[test]
fn the_packed_refs_lock_warning_is_expected() {
    let line = WORKER_CONTRACT.lines().nth(12).expect("line 12");
    assert!(
        line.starts_with("12. After a commit, git may print "),
        "{line}"
    );
    for part in [
        "packed-refs.lock': Operation not permitted",
        "the commit succeeded",
        "Do not try to fix it or change git settings.",
    ] {
        assert!(line.contains(part), "{part}: {line}");
    }
}

const EXTRACT: &str = "Scout report s1:\n  Summary one\nFiles: crates/a/src/x.rs";

fn notes() -> String {
    crate::run::orch::contract::notes_section(&[crate::run::orch::TaskMessage {
        at: 47_220,
        source: crate::run::orch::EditSource::Orchestrator,
        kind: proto::MessageKind::Change,
        text: "use the v2 token API".into(),
        delivered: true,
        outbox: None,
    }])
}

/// Decisions 34 and 42d, spec §14.2: the prompt text, the profile summary, the scout
/// extract, the notes, and the brief last.
#[test]
fn worker_prompt_places_the_scout_extract_before_the_brief() {
    let run = prompt_run(PROFILE, "test_to_write = \"a::expires\"");
    let notes = notes();
    let prompt = worker_prompt(&run, &run.tasks[0], EXTRACT, &notes);
    let at = |needle: &str| {
        prompt
            .find(needle)
            .unwrap_or_else(|| panic!("{needle:?} missing from {prompt}"))
    };
    assert!(at("[anthrex] Task t1") < at("Check command: cargo test"));
    assert!(at("Check command: cargo test") < at("Scout report s1:"));
    assert!(at("Scout report s1:") < at("Notes from the orchestrator:"));
    assert!(
        prompt.ends_with(&format!("- Accept t1\n\n{EXTRACT}\n\n{notes}\n\nBrief t1")),
        "{prompt}"
    );
}

#[test]
fn worker_prompt_puts_saved_messages_before_the_brief() {
    let run = prompt_run(PROFILE, "");
    let prompt = worker_prompt(&run, &run.tasks[0], "", &notes());
    assert!(
        prompt.ends_with(
            "- Accept t1\n\nNotes from the orchestrator:\n- 13:07 (change, from orchestrator) use the v2 token API\n\nBrief t1"
        ),
        "{prompt}"
    );
}

#[test]
fn handover_prompt_carries_the_saved_messages() {
    let run = prompt_run(PROFILE, "");
    let notes = notes();
    let prompt = handover_prompt(
        &run,
        &run.tasks[0],
        "rung 2",
        " 1 file",
        "+x",
        EXTRACT,
        &notes,
    );
    assert!(
        prompt.starts_with(&worker_prompt(&run, &run.tasks[0], EXTRACT, &notes)),
        "{prompt}"
    );
    assert!(
        prompt.contains(&format!("{notes}\n\nBrief t1\n\nThis is session ")),
        "{prompt}"
    );
}

#[test]
fn reviewer_prompt_lists_change_messages() {
    let run = prompt_run(PROFILE, "");
    let section = "Messages the worker received:\n- 13:07 (change) use the v2 token API";
    let base = "c".repeat(40);
    let head = "d".repeat(40);
    let prompt = reviewer_prompt(&run, &run.tasks[0], 1, &base, &head, "+x", section);
    assert!(
        prompt.ends_with(&format!("\n{section}\n\nBrief t1")),
        "{prompt}"
    );
    let without = reviewer_prompt(&run, &run.tasks[0], 1, &base, &head, "+x", "");
    assert!(
        !without.contains("Messages the worker received"),
        "{without}"
    );
    assert!(without.ends_with("+x\n\nBrief t1"), "{without}");
}

// The diff clamp (M8a.8).

/// Every cut position modulo a character: `k` ASCII bytes shift a body of 3-byte
/// (`世`) or 4-byte (`𝄞`) characters, so over `k` in 0..=3 each cut lands on every
/// offset inside a character.
#[test]
fn clamp_diff_cuts_on_character_boundaries_at_every_offset() {
    for body in ["世", "𝄞", "a世𝄞"] {
        for k in 0..=3 {
            let text = format!("{}{}", "a".repeat(k), body.repeat(20_000));
            for max in [REVIEW_DIFF_MAX, 1000, 1001, 1002, 1003] {
                let out = clamp_diff(&text, max);
                assert!(out.len() <= max, "{body} k={k} max={max}: {}", out.len());
                assert!(
                    out.len() >= max - 3,
                    "{body} k={k} max={max}: {}",
                    out.len()
                );
                assert_eq!(out.matches(DIFF_CUT_MARKER).count(), 1);
                let (head, tail) = out.split_once(DIFF_CUT_MARKER).unwrap();
                assert!(text.starts_with(head), "{body} k={k} max={max}");
                assert!(text.ends_with(tail), "{body} k={k} max={max}");
            }
        }
    }
}

#[test]
fn clamp_diff_leaves_text_within_the_limit_alone() {
    let text = "世".repeat(10);
    assert_eq!(clamp_diff(&text, 30), text);
    assert_eq!(clamp_diff(&text, 31), text);
    let cut = clamp_diff(&text, 29);
    assert!(cut.len() <= 29, "{cut:?}");
}

#[test]
fn clamp_diff_below_the_marker_keeps_a_head_only() {
    let text = "世".repeat(100);
    let out = clamp_diff(&text, 10);
    assert_eq!(out, "世世世");
}

// Decision 54's reviewer line (whole-branch review, ruling C-27 item I-1).

const INTERFACE_LINE: &str =
    "Check that the old interface still works: callers outside this task must still build.";

/// `t1`'s reviewer prompt with `interface_change`, in a run with `layout`, on the
/// tiered profile or the untiered one.
fn interface_prompt(interface: bool, multi: bool, tiered: bool) -> String {
    let mut run = prompt_run(PROFILE, "");
    run.tasks[0].spec.interface_change = interface;
    if multi {
        run.stage_layout = crate::run::model::StageLayout::Multi;
    }
    if tiered {
        run.profile.tiers.build_check = Some("cargo build".into());
    }
    let (base, head) = ("c".repeat(40), "d".repeat(40));
    reviewer_prompt(&run, &run.tasks[0], 1, &base, &head, "+x", "")
}

#[test]
fn an_interface_change_reviewer_checks_the_old_interface() {
    for (multi, tiered) in [(true, false), (false, true), (true, true)] {
        let prompt = interface_prompt(true, multi, tiered);
        let lines: Vec<&str> = prompt.lines().collect();
        assert_eq!(
            lines[4], INTERFACE_LINE,
            "multi={multi} tiered={tiered}\n{prompt}"
        );
        assert_eq!(prompt.matches(INTERFACE_LINE).count(), 1, "{prompt}");
    }
}

#[test]
fn the_interface_line_needs_an_interface_change_and_a_staged_or_tiered_run() {
    for (interface, multi, tiered) in [
        (false, true, true),
        (false, true, false),
        (false, false, true),
        (true, false, false),
    ] {
        let prompt = interface_prompt(interface, multi, tiered);
        assert!(
            !prompt.contains("old interface"),
            "interface={interface} multi={multi} tiered={tiered}\n{prompt}"
        );
    }
    // A Single untiered run's prompt is byte-identical with or without the flag.
    assert_eq!(
        interface_prompt(true, false, false),
        interface_prompt(false, false, false)
    );
}

/// Ruling F-4a (2026-10-01): a check whose kept output is empty (a passing tier job
/// keeps none) is one line saying how it ended, never an empty block the reviewer reads
/// as missing evidence. A check with output keeps M8a's block.
#[test]
fn reviewer_prompt_states_a_check_with_no_output() {
    let mut run = prompt_run(PROFILE, "");
    let check = |ok: bool, code: Option<i32>, timed_out: bool, tail: &str| CheckRecord {
        at: 1,
        ok,
        code,
        timed_out,
        tail: tail.into(),
        secs: 3,
        on_candidate: false,
        summary: None,
        summary_source: None,
        tier: None,
        lane: None,
    };
    let (base, head) = ("c".repeat(40), "d".repeat(40));
    let cases = [
        (
            check(true, Some(0), false, ""),
            "Last check: passed (exit 0, 3s); output omitted.",
        ),
        (
            check(true, Some(0), false, "\n  \n"),
            "Last check: passed (exit 0, 3s); output omitted.",
        ),
        (
            check(false, Some(101), false, ""),
            "Last check: failed (exit 101, 3s); output omitted.",
        ),
        (
            check(false, None, true, ""),
            "Last check: timed out (3s); output omitted.",
        ),
    ];
    for (record, line) in cases {
        run.tasks[0].checks = vec![record];
        let prompt = reviewer_prompt(&run, &run.tasks[0], 1, &base, &head, "+x", "");
        assert!(prompt.contains(&format!("\n{line}\n")), "{prompt}");
        assert!(!prompt.contains("Last check (40 lines)"), "{prompt}");
    }
    run.tasks[0].checks = vec![check(true, Some(0), false, "test a::works ... ok")];
    let prompt = reviewer_prompt(&run, &run.tasks[0], 1, &base, &head, "+x", "");
    assert!(
        prompt.contains("Last check (40 lines):\ntest a::works ... ok\n"),
        "{prompt}"
    );
}
