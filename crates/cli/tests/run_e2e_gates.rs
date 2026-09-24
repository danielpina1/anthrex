//! Milestone 8a, task 24: the gates end to end (the test proof, the check, a rate-limit
//! retry and a failed turn, the turn-end fallback) and the plan refusals, through a real
//! daemon with `fake-agent` as both runtimes. The review scenarios are in
//! `run_e2e_review.rs`.

mod support;

use std::path::Path;
use std::time::Duration;

use proto::{AgentRole, DaemonMsg, DoneSignal, Status, TaskState};
use serde_json::{Value, json};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

/// The brief's `check.sh`: exits 0 unless a file `broken` exists.
const CHECK_SH: &str =
    "if [ -e broken ]; then echo 'broken is present'; exit 1; fi\necho check ok\n";

/// The brief's profile for these tests.
fn gates_plan(tasks: &[String]) -> String {
    format!(
        "goal = \"Gates\"\n\n[profile]\ncheck = \"sh check.sh\"\nsingle_test = \"sh tests/{{test}}.sh\"\ntest_passed = \"PASS {{test}}\"\n{}",
        tasks.concat()
    )
}

/// A harness whose base commit has `check.sh`, with `orchestrator` config lines.
fn harness(orchestrator: &str) -> RunHarness {
    RunHarness::with_repo(orchestrator, &[], true, &[("check.sh", CHECK_SH)])
}

/// An S `tdd` task owning `owns`.
fn tdd_task(id: &str, owns: &[&str]) -> String {
    task(id, owns, "").replace(
        "test_mode = \"check\"\ntest_mode_reason = \"smoke\"\n",
        "test_mode = \"tdd\"\n",
    )
}

/// Writes `tests/<name>.sh` with `body` and commits it as `message`.
fn commit_test(name: &str, body: &str, message: &str) -> Value {
    sh(&format!(
        "mkdir -p tests && printf '%s' '{body}' > tests/{name}.sh && git add -A && git commit -qm '{message}'"
    ))
}

#[test]
fn e2e_tdd_red_commit_that_passes_is_rejected() {
    let h = harness("");
    h.script(
        "worker-t1-1",
        &[
            commit_test("t_add", "echo PASS t_add\n", "add t_add"),
            capture("red", "git rev-parse HEAD"),
            commit("lib.txt", "lib\n"),
            done_tdd("t_add", "{{red}}"),
            read("The test proof failed: at the red commit"),
            commit_test(
                "t_add",
                "test -f feature.txt || exit 1\necho PASS t_add\n",
                "make t_add fail without the feature",
            ),
            capture("red", "git rev-parse HEAD"),
            commit("feature.txt", "feature\n"),
            done_tdd("t_add", "{{red}}"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(
        &gates_plan(&[tdd_task("t1", &["tests/**", "lib.txt", "feature.txt"])]),
        true,
    );
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.bounces.proof, 1, "{:?}", t1.bounces);
    assert_eq!(t1.state, TaskState::Merged);
    let texts = user_texts(&h.io_lines("worker-t1-1", "stdin"));
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("[anthrex] The test proof failed: at the red commit")),
        "{texts:#?}"
    );
}

#[test]
fn e2e_tdd_test_that_did_not_run_is_rejected() {
    let h = harness("");
    h.script(
        "worker-t1-1",
        &[
            commit_test(
                "t_reset",
                "test -f reset.txt || exit 1\necho ran\n",
                "add t_reset",
            ),
            capture("red", "git rev-parse HEAD"),
            commit("reset.txt", "reset\n"),
            done_tdd("t_reset", "{{red}}"),
            read("expected a line matching PASS t_reset"),
            commit_test(
                "t_reset",
                "test -f reset.txt || exit 1\necho PASS t_reset\n",
                "say PASS t_reset",
            ),
            done_tdd("t_reset", "{{red}}"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(
        &gates_plan(&[tdd_task("t1", &["tests/**", "reset.txt"])]),
        true,
    );
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.bounces.proof, 1, "{:?}", t1.bounces);
    let texts = user_texts(&h.io_lines("worker-t1-1", "stdin"));
    let want = "[anthrex] The test proof failed: the output did not show that t_reset ran and passed (expected a line matching PASS t_reset)";
    assert!(texts.iter().any(|t| t.starts_with(want)), "{texts:#?}");
}

#[test]
fn e2e_check_fails_once_then_passes() {
    let h = harness("");
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            commit("broken", "x\n"),
            done("added a"),
            read("The check failed"),
            sh("git rm -q broken && git commit -qm 'remove broken'"),
            done("fixed the check"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&gates_plan(&[task("t1", &["a.txt", "broken"], "")]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.bounces.check, 1, "{:?}", t1.bounces);
    assert_eq!(t1.state, TaskState::Merged);
    let json = run_json(&run);
    let checks: Vec<&Value> = json["tasks"][0]["checks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["on_candidate"] != true)
        .collect();
    assert_eq!(checks.len(), 2, "{checks:#?}");
    assert_eq!(checks[0]["ok"], false, "{checks:#?}");
    assert_eq!(checks[1]["ok"], true, "{checks:#?}");
}

/// The unix time `perl` prints, written to `<io>/<name>`: a timestamp taken inside the
/// agent's own turn.
fn stamp(io: &Path, name: &str) -> Value {
    sh(&format!(
        "perl -MTime::HiRes=time -e 'printf \"%.3f\", time' > {}",
        io.join(name).display()
    ))
}

fn read_stamp(io: &Path, name: &str) -> f64 {
    std::fs::read_to_string(io.join(name))
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// Decision 32's config for this test: two different values, so an engine that waits
/// `stall_after_secs` for the continue fails the timing assertion.
const RATE_LIMIT_RETRY_SECS: u64 = 7;
/// The script's silence after its retry event.
const SILENCE_MS: u64 = 9000;

#[test]
fn e2e_rate_limit_retry_is_not_a_stall_and_a_failed_turn_is_continued() {
    let h = harness(&format!(
        "stall_after_secs = 5\nrate_limit_retry_secs = {RATE_LIMIT_RETRY_SECS}"
    ));
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            stamp(&h.io, "before-retry"),
            json!({"api_retry": {"error": "rate_limit", "delay_ms": 8000, "times": 1}}),
            json!({"wait_ms": SILENCE_MS}),
            json!({"fail_turn": {"error": "rate_limit"}}),
            read("stopped on an API error"),
            stamp(&h.io, "continued"),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let watcher = h.watch(None);
    let id = h.start(&gates_plan(&[task("t1", &["a.txt"], "")]), true);
    // Decision 32's timer is on the critical path: `rate_limit_retry_secs` on top.
    let run = h.wait_run(
        &id,
        complete,
        RUN_WAIT + Duration::from_secs(RATE_LIMIT_RETRY_SECS),
    );
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);

    let stdin = h.io_lines("worker-t1-1", "stdin");
    let interrupts: Vec<&String> = stdin
        .iter()
        .filter(|l| serde_json::from_str::<Value>(l).is_ok_and(|v| v["type"] == "control_request"))
        .collect();
    assert!(
        interrupts.is_empty(),
        "an interrupt was sent: {interrupts:?}"
    );

    let worker = t1
        .rounds
        .iter()
        .find(|r| r.role == AgentRole::Worker)
        .and_then(|r| r.window_id)
        .unwrap();
    let attention = watcher.received().iter().any(|m| {
        matches!(m, DaemonMsg::WindowsChanged { windows }
            if windows.iter().any(|w| w.id == worker && w.status == Status::Attention))
    });
    assert!(attention, "the worker window never showed Attention");

    // The failed turn ended `SILENCE_MS` after the retry, which came right after the
    // first stamp; the second stamp is taken as soon as the continue arrived.
    let failed_at = read_stamp(&h.io, "before-retry") + SILENCE_MS as f64 / 1000.0;
    let continued_at = read_stamp(&h.io, "continued");
    assert!(
        continued_at - failed_at >= RATE_LIMIT_RETRY_SECS as f64,
        "the continue came {:.3}s after the failed turn",
        continued_at - failed_at
    );
    assert_eq!(
        run.rate_limits.get("claude"),
        Some(&1),
        "{:?}",
        run.rate_limits
    );
}

#[test]
fn e2e_turn_end_fallback_completes_a_silent_worker() {
    let h = harness("");
    h.script("worker-t1-1", &[commit("a.txt", "a\n")]);
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&gates_plan(&[task("t1", &["a.txt"], "")]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.done_signal, Some(DoneSignal::TurnEndFallback));
    let texts = user_texts(&h.io_lines("worker-t1-1", "stdin"));
    let nudge = "[anthrex] Your turn ended with commits on your branch and no task_done. If the task is complete, call task_done now (for a tdd task, with test and red). If you are stuck, call task_blocked.";
    assert!(texts.iter().any(|t| t == nudge), "{texts:#?}");
}

#[test]
fn e2e_plan_with_an_l_task_is_rejected() {
    let h = harness("");
    let l_task = task("t1", &["a.txt"], "").replace("size = \"S\"", "size = \"L\"");
    let message = refused(h.start_reply(&h.repo, &gates_plan(&[l_task]), true, false));
    assert!(
        message
            .lines()
            .any(|l| l == "task t1: size: L tasks are never executed; split the task (rule 7.2.4)"),
        "{message}"
    );
    assert!(no_run_branches(&h.repo));
    assert!(h.snapshot().runs.is_empty());
}

#[test]
fn e2e_cross_runtime_overlapping_owns_are_rejected() {
    let h = harness("");
    let plan = gates_plan(&[
        task("t1", &["src/**"], ""),
        task("t2", &["src/a.rs"], CODEX),
    ]);
    let message = refused(h.start_reply(&h.repo, &plan, true, false));
    assert!(
        message.lines().any(|l| l
            == "task t2: owns: overlaps task t1's owns (src/**) and the two tasks run on different runtimes (claude, codex) (rule 9)"),
        "{message}"
    );
    assert!(no_run_branches(&h.repo));
    assert!(h.snapshot().runs.is_empty());
}
