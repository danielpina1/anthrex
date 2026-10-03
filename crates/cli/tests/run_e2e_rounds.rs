//! Milestone 9.3 task M9.3.11: rounds end to end (KG §2), through the real binary and a
//! real daemon on temporary paths, with `fake-agent` as the orchestrator in its PTY
//! (`orchestrator-run-1`), as the workers and reviewers, and as the triage decider; no
//! real agent and no `gh`. A goal run's second round from `anthrex run iterate`, its
//! gate, and the accept that lands both rounds; and the request read from stdin, refused
//! while the run is still running.

mod support;

use proto::{RoundOutcome, RunState, TaskState};
use serde_json::json;
use support::orch_script::*;
use support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness};
use support::run_orch::ORCH_WAIT;
use support::run_rounds::*;

#[test]
fn e2e_two_rounds_then_accept_lands_both_rounds() {
    let h = RunHarness::orch("", &[]);
    let mut steps = two_rounds();
    steps.push(read(None));
    let (run, _) = h.two_rounds_accepted(&steps);

    // The orchestrator read the round wake, whole: its first line, the request fenced.
    let texts = texts(&h);
    assert_eq!(texts.len(), 1, "{texts:#?}");
    let wake = format!("{}\n{}", round_wake_head(&run, 2, 1), fenced(REQUEST));
    assert_eq!(texts[0], wake);

    // The base branch has both rounds' files.
    let info = h.wait_run(&run, |r| r.state == RunState::Accepted, RUN_WAIT);
    assert_eq!(h.git(&["show", "main:a.txt"]), "t1");
    assert_eq!(h.git(&["show", "main:b.txt"]), "t2");
    for t in &info.tasks {
        assert_eq!(t.state, TaskState::Merged, "{}", t.id);
    }
    // Two rounds, both completed, and their `round` lines; one `run` line.
    let rounds: Vec<_> = info.rounds.iter().map(|r| (r.n, r.outcome)).collect();
    assert_eq!(
        rounds,
        [
            (1, Some(RoundOutcome::Completed)),
            (2, Some(RoundOutcome::Completed))
        ]
    );
    // The run line is counted once the history has settled: both round lines and a run
    // line written, then the chain idle after the run (the accept's last step), so a
    // second line the accept wrote late is seen (final fix wave, task 11 m2).
    support::run_plans::until("the history lines", RUN_WAIT, || {
        let rounds = h.history_lines("round").len();
        (rounds == 2 && !h.history_lines("run").is_empty()).then_some(())
    });
    support::run_plans::until("the idle orchestrator", REQUEST_WAIT, || {
        let idle = h.snapshot().idle_orchestrators;
        idle.iter().any(|c| c.after_run == run).then_some(())
    });
    let lines = h.history_lines("run");
    assert_eq!(lines.len(), 1, "{lines:#?}");
    assert_eq!(lines[0]["outcome"], json!("accepted"), "{}", lines[0]);
    let rounds: Vec<_> = (h.history_lines("round").iter())
        .map(|l| (l["round"].clone(), l["outcome"].clone()))
        .collect();
    assert_eq!(
        rounds,
        [
            (json!(1), json!("completed")),
            (json!(2), json!("completed"))
        ]
    );
}

/// The stdin test's script poll for `complete`, which starts at the gate's approval
/// (final fix wave, task 11 m4; `docs/timing-budgets.md`): the test's wait for t1's
/// worker to be working (`RUN_WAIT`), the refused iterate (one engine step,
/// `REQUEST_WAIT`) while the worker holds its turn, then t1's path after the hold
/// (`RUN_WAIT`).
const HELD_PATH_WAIT: std::time::Duration = RUN_WAIT
    .saturating_add(REQUEST_WAIT)
    .saturating_add(RUN_WAIT);

#[test]
fn e2e_iterate_from_stdin_and_its_refusal_while_running() {
    let h = RunHarness::orch("", &[]);
    // `t1`'s worker holds its turn until the test creates `go`.
    let go = h.dir.path().join("go");
    h.script(
        "worker-t1-1",
        &[
            wait_file(&go),
            support::run_plans::commit("a.txt", "t1\n"),
            support::run_plans::done("added a.txt"),
        ],
    );
    h.script("reviewer-t1-1", &[support::run_plans::approve()]);
    let steps = [
        prompt(),
        edit_plan(vec![staged("t1", "a.txt", 1)], json!({"submit": true})),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        until("/run/complete", json!(true), HELD_PATH_WAIT),
        edit_plan(vec![], json!({"summary": SUMMARY_1})),
        until("/run/round", json!(2), ITERATE_WAIT),
        read(Some("the user asks for round 2 of run ")),
        marker(),
        read(None),
    ];
    h.script(ORCH, &steps);
    let run = h.start_goal_id("add a", &[]);
    h.approve_round(&run, 1);
    let working = |r: &proto::RunInfo| {
        r.state == RunState::Running
            && r.tasks
                .iter()
                .any(|t| t.id == "t1" && t.state == TaskState::Working)
    };
    h.wait_run(&run, working, RUN_WAIT);

    // A running run: the daemon's exact refusal, exit 1, nothing on stdout.
    let out = h.iterate(&run, "-", &format!("{REQUEST}\n"));
    assert_eq!(out.status.code(), Some(1), "{out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    let refusal = format!("run {} is running; iterate it when it completes", h4(&run));
    assert_eq!(stderr.lines().last(), Some(refusal.as_str()), "{stderr}");
    assert!(out.stdout.is_empty(), "{out:?}");
    assert_eq!(h.run(&run).unwrap().round, 1);

    // Once complete, the same request from stdin starts round 2.
    std::fs::write(&go, "").unwrap();
    h.wait_summary(&run, SUMMARY_1, SUMMARY_WAIT);
    let out = h.iterate(&run, "-", &format!("{REQUEST}\n"));
    ok(&out);
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        format!(
            "run {} round 2 started; its orchestrator plans it",
            h4(&run)
        )
    );
    h.wait_log(
        "the orchestrator to read the round wake",
        |log| passed(log, ORCH) >= 1,
        ORCH_WAIT,
    );
    let texts = texts(&h);
    assert_eq!(texts.len(), 1, "{texts:#?}");
    // Whole: the request read from stdin, its line end trimmed, fenced.
    let wake = format!("{}\n{}", round_wake_head(&run, 2, 1), fenced(REQUEST));
    assert_eq!(texts[0], wake);
    let info = h.run(&run).unwrap();
    assert_eq!((info.round, info.state), (2, RunState::Planning));
}
