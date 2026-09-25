//! Milestone 8a, task 24: the review gate end to end (rejections and the escalation
//! ladder, a disputed finding, an override, minor findings), through a real daemon with
//! `fake-agent` as both runtimes. The other gates are in `run_e2e_gates.rs`.

mod support;

use proto::{AgentRole, BlockReason, RunState, Runtime, TaskState};
use serde_json::{Value, json};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

fn blocked_question(reason: &str) -> Value {
    json!({"mcp_call": {"tool": "task_blocked", "args": {"kind": "question", "reason": reason}}})
}

#[test]
fn e2e_two_rejections_then_a_fresh_peer_worker_is_approved() {
    let h = RunHarness::new("");
    h.script(
        "worker-t1-1",
        &[
            commit("a.rs", "fn a() {}\n"),
            done("added a"),
            read("Review round 1 asked for changes"),
            commit("a.rs", "fn a() { /* fixed */ }\n"),
            done("fixed a"),
            // Killed here at rung 2.
            read("never delivered"),
        ],
    );
    h.script(
        "reviewer-t1-1",
        &[changes(&[finding("important", "a.rs", 3, "off by one")])],
    );
    h.script(
        "reviewer-t1-2",
        &[changes(&[finding("important", "b.rs", 9, "missing check")])],
    );
    h.script(
        "worker-t1-2",
        &[commit("b.rs", "fn b() {}\n"), done("added the check")],
    );
    h.script("reviewer-t1-3", &[approve()]);
    let plan = plan(
        "",
        &[task(
            "t1",
            &["a.rs", "b.rs"],
            "route = { runtime = \"claude\", effort = \"high\" }",
        )],
    );
    let id = h.start(&plan, true);

    // A fresh session of the same task is a task path of its own (k = 2).
    let run = h.wait_run(
        &id,
        |r| {
            t(r, "t1")
                .rounds
                .iter()
                .any(|a| a.role == AgentRole::Worker && a.session == 2 && a.window_id.is_some())
        },
        2 * RUN_WAIT,
    );
    let fresh = t(&run, "t1")
        .rounds
        .iter()
        .find(|a| a.role == AgentRole::Worker && a.session == 2)
        .unwrap();
    assert_eq!(fresh.route.runtime, Runtime::Codex);
    let window = fresh.window_id.unwrap();
    let info = h
        .windows()
        .into_iter()
        .find(|w| w.id == window)
        .expect("the fresh worker's window is listed");
    assert_eq!(info.runtime, Runtime::Codex);

    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.failures, 2);
    let mut sessions: Vec<u32> = t1
        .rounds
        .iter()
        .filter(|r| r.role == AgentRole::Worker)
        .map(|r| r.session)
        .collect();
    sessions.dedup();
    assert_eq!(sessions, [1, 2], "{:#?}", t1.rounds);

    // Round 2's prompt lists round 1's finding to confirm fixed, and not its own.
    let prompts = h.codex_messages("reviewer-t1-2");
    let prompt = prompts.first().expect("reviewer-t1-2 started");
    let (_, earlier) = prompt
        .split_once("Earlier findings to confirm fixed:\n")
        .unwrap_or_else(|| panic!("no earlier findings in round 2's prompt:\n{prompt}"));
    assert!(
        earlier.starts_with("- [important] a.rs:3 off by one"),
        "{prompt}"
    );
    assert!(!prompt.contains("missing check"), "{prompt}");

    // Ruling T24-I1: round 3's prompt lists every earlier round's findings, round 1's
    // and round 2's. (The brief's "and not its own" above can never fail: a round's own
    // finding does not exist when its prompt is built.) Round 3's reviewer is the
    // engine's peer choice for the Codex worker, so its prompt is read from either.
    let mut prompts = user_texts(&h.io_lines("reviewer-t1-3", "stdin"));
    prompts.extend(h.codex_messages("reviewer-t1-3"));
    let prompt = prompts.first().expect("reviewer-t1-3 started");
    let (_, earlier) = prompt
        .split_once("Earlier findings to confirm fixed:\n")
        .unwrap_or_else(|| panic!("no earlier findings in round 3's prompt:\n{prompt}"));
    assert!(
        earlier
            .starts_with("- [important] a.rs:3 off by one\n- [important] b.rs:9 missing check\n"),
        "{prompt}"
    );
}

#[test]
fn e2e_disputed_finding_blocks_as_a_question_and_the_answer_resumes_it() {
    let h = RunHarness::new("");
    h.script(
        "worker-t1-1",
        &[
            commit("a.rs", "fn a() {}\n"),
            done("added a"),
            read("Review round 1 asked for changes"),
            blocked_question("the finding is wrong"),
            read("Answer to your question"),
            done("kept a as it is"),
        ],
    );
    h.script(
        "reviewer-t1-1",
        &[changes(&[finding("important", "a.rs", 1, "a is wrong")])],
    );
    h.script("reviewer-t1-2", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["a.rs"], "")]), true);

    let run = h.wait_run(&id, |r| t(r, "t1").state == TaskState::Blocked, RUN_WAIT);
    assert_eq!(run.state, RunState::Running);
    let block = t(&run, "t1").block.clone().unwrap();
    assert_eq!(block.reason, BlockReason::Question, "{block:?}");
    assert!(
        run.attention
            .iter()
            .any(|l| l.starts_with("t1 blocked (question)")),
        "{:?}",
        run.attention
    );

    let edits = h.dir.path().join("answer.toml");
    std::fs::write(
        &edits,
        "[[edit]]\nop = \"answer\"\ntask_id = \"t1\"\ntext = \"you are right; keep it\"\n",
    )
    .unwrap();
    let output = h.anthrex(&["run", "edit", &id, "--file", edits.to_str().unwrap()]);
    assert!(
        output.status.success(),
        "run edit: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.reviews.len(), 2, "{:#?}", t1.reviews);
    let texts = user_texts(&h.io_lines("worker-t1-1", "stdin"));
    assert!(
        texts
            .iter()
            .any(|t| t == "[anthrex] Answer to your question: you are right; keep it"),
        "{texts:#?}"
    );
}

#[test]
fn e2e_override_merges_without_approval_and_is_reported() {
    let h = RunHarness::new("");
    h.script(
        "worker-t1-1",
        &[
            commit("a.rs", "fn a() {}\n"),
            done("added a"),
            read("Review round 1 asked for changes"),
            commit("a.rs", "fn a() { /* fixed */ }\n"),
            done("fixed a"),
            // Killed here at rung 3.
            read("never delivered"),
        ],
    );
    h.script(
        "reviewer-t1-1",
        &[changes(&[finding("critical", "a.rs", 1, "unsafe cast")])],
    );
    h.script(
        "reviewer-t1-2",
        &[changes(&[finding("critical", "a.rs", 2, "still unsafe")])],
    );
    // The check logs where it ran: a green merge candidate leaves no check record, so
    // the log shows the candidate check (in the integration worktree) did run.
    // Final fix batch F1c (I2): a confined check writes outside its checkout only to
    // the profile's `cache_dirs`.
    let out = h.dir.path().canonicalize().unwrap().join("out");
    std::fs::create_dir_all(&out).unwrap();
    let checks = out.join("checks.log");
    let check = format!("pwd >> {}", checks.display());
    let profile = format!("cache_dirs = [{:?}]", out.display().to_string());
    let plan = format!(
        "max_bounces = 1\n{}",
        plan(&profile, &[task("t1", &["a.rs"], "")])
    )
    .replace("check = \"true\"", &format!("check = {check:?}"));
    let id = h.start(&plan, true);

    let run = h.wait_run(&id, |r| t(r, "t1").state == TaskState::Blocked, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.bounces.review, 2, "{:?}", t1.bounces);
    assert_eq!(t1.rung, 3);
    assert_eq!(
        t1.block.as_ref().map(|b| b.reason),
        Some(BlockReason::MisSized)
    );

    let output = h.anthrex(&["run", "override", &id, "t1", "--reason", "trusted"]);
    assert!(
        output.status.success(),
        "run override: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.merged_without_approval.as_deref(), Some("trusted"));
    let ran = std::fs::read_to_string(&checks).unwrap();
    let last = ran.lines().last().unwrap_or_default();
    assert!(last.ends_with("/integration"), "{ran}");
    assert_eq!(
        h.git(&["rev-parse", &format!("anthrex/{id}/integration")]),
        t1.merge_commit.clone().unwrap()
    );
    report_with(&run, "merged without approval: trusted");
}

#[test]
fn e2e_minor_findings_do_not_bounce() {
    let h = RunHarness::new("");
    green_scripts(&h.repo);
    h.script(
        "reviewer-t1-1",
        &[changes(&[
            finding("minor", "a.txt", 1, "a trailing newline would be nicer"),
            json!({"severity": "minor", "text": "consider a longer name"}),
        ])],
    );
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!((t1.failures, t1.bounces.review), (0, 0));
    assert_eq!(t1.reviews.len(), 1);
    let report = report_with(&run, "a trailing newline would be nicer");
    assert!(
        report.contains(
            "Minor:\n- a.txt:1: a trailing newline would be nicer\n- consider a longer name\n"
        ),
        "{report}"
    );
}
