//! Milestone 9.5 task M9.5.22: tuning end to end (decisions 4 to 16 and 38, spec §21),
//! through a real daemon with `fake-agent` as both runtimes and the orchestrator. A
//! recorded history refits a run's budget and a confirmed proposal applies; a
//! configured budget beats the refit; a rate limit halves a runtime's writers; and a
//! Claude orchestrator's first turn waits for its anthrex tools.

mod support;

use std::collections::BTreeMap;

use proto::{Budget, RunInfo, RunState, TaskInfo, TaskState};
use serde_json::{Value, json};
use support::orch_script::{add, edit_plan, plan_task, read};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_orch::ORCH_WAIT;
use support::run_plans::*;
use support::run_pr::log_lines;

/// A plan with no `[profile]`: the stored profile is the run's.
fn plan_of(tasks: &[String]) -> String {
    format!("goal = \"Tune\"\n{}", tasks.concat())
}

fn ok(out: &std::process::Output) {
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

/// `refit.jsonl` as the history, `run stats --apply thresholds.s --yes`, then a one-task
/// S plan whose worker waits for its release: the run id once `t1` works and both
/// estimate fields are set, and that snapshot.
fn refit_and_start(h: &RunHarness) -> (String, RunInfo) {
    h.history("refit.jsonl");
    let repo = h.repo.display().to_string();
    let out = h.anthrex(&[
        "run",
        "stats",
        "--dir",
        &repo,
        "--apply",
        "thresholds.s",
        "--yes",
    ]);
    ok(&out);
    let applied = String::from_utf8_lossy(&out.stdout);
    assert!(applied.starts_with("applied thresholds.s: "), "{applied}");
    h.script(
        "worker-t1-1",
        &[
            h.wait_release("t1"),
            commit("a.txt", "a\n"),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan_of(&[task("t1", &["a.txt"], "")]), true);
    let working = |r: &RunInfo| {
        t(r, "t1").state == TaskState::Working
            && r.estimate_left_secs.is_some()
            && r.bound_ratio_permille.is_some()
    };
    let run = h.wait_run(&id, working, RUN_WAIT);
    (id, run)
}

/// Releases `t1`'s worker and waits for the run to complete with `t1` merged.
fn finish(h: &RunHarness, id: &str) {
    h.release("t1");
    let run = h.wait_run(id, complete, RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
}

/// The run log's `tuning: ` lines.
fn tuning_lines(h: &RunHarness, id: &str) -> Vec<String> {
    (log_lines(h, id).into_iter())
        .filter(|l| l.starts_with("tuning: "))
        .collect()
}

#[test]
fn e2e_history_refits_budgets_and_a_confirmed_proposal_applies() {
    let h = RunHarness::tuning("", "", &[]);
    let (id, run) = refit_and_start(&h);
    assert_eq!(
        t(&run, "t1").budget,
        Budget {
            tool_calls: 55,
            minutes: 18,
            tokens: None
        }
    );
    let lines = tuning_lines(&h, &id);
    assert!(
        lines.contains(&"tuning: budget S 55 calls 18m from 34 samples".to_string()),
        "{lines:#?}"
    );
    assert!(
        lines.contains(&"tuning: thresholds S 35, M 100 lines".to_string()),
        "{lines:#?}"
    );
    finish(&h, &id);
}

#[test]
fn e2e_a_configured_budget_beats_the_refit() {
    let h = RunHarness::tuning(
        "",
        "[orchestrator.budget.s]\ntool_calls = 70\nminutes = 25\n",
        &[],
    );
    let (id, run) = refit_and_start(&h);
    assert_eq!(
        t(&run, "t1").budget,
        Budget {
            tool_calls: 70,
            minutes: 25,
            tokens: None
        }
    );
    let lines = tuning_lines(&h, &id);
    let configured = "tuning: budget S 70 calls 25m configured (refit would be 55 calls 18m)";
    assert!(lines.contains(&configured.to_string()), "{lines:#?}");
    assert!(
        !lines.iter().any(|l| l.starts_with("tuning: budget S 55")),
        "{lines:#?}"
    );
    finish(&h, &id);
}

/// `t2` is past its check: it no longer holds a writer slot.
fn past_check(task: &TaskInfo) -> bool {
    matches!(
        task.state,
        TaskState::Review | TaskState::MergeQueue | TaskState::Merged
    )
}

#[test]
fn e2e_a_rate_limit_halves_writers_for_that_runtime() {
    let h = RunHarness::tuning("", "", &[]);
    let watcher = h.subscribe();
    // On Claude by their row (milestone 9.8 decision 31: a plan's route is ignored).
    let claude = "";
    h.script(
        "worker-t1-1",
        &[
            json!({"api_retry": {"error": "rate_limit", "delay_ms": 100, "times": 1}}),
            commit("a.txt", "a\n"),
            done("added a"),
        ],
    );
    h.script(
        "worker-t2-1",
        &[
            commit("b.txt", "b\n"),
            h.wait_release("t2"),
            done("added b"),
        ],
    );
    h.script("worker-t3-1", &[commit("c.txt", "c\n"), done("added c")]);
    for task in ["t1", "t2", "t3"] {
        h.script(&format!("reviewer-{task}-1"), &[approve()]);
    }
    let tasks = [
        task("t1", &["a.txt"], claude),
        task("t2", &["b.txt"], claude),
        task("t3", &["c.txt"], claude),
    ];
    let id = h.start(&format!("max_writers = 2\n{}", plan_of(&tasks)), true);

    // t1's rate limit halves Claude's writers.
    let capped = BTreeMap::from([("claude".to_string(), 1u8)]);
    h.wait_run(&id, |r| r.writer_caps == capped, RUN_WAIT);
    // t1 merges while t2 holds Claude's one writer: t3 waits.
    let run = h.wait_run(&id, |r| t(r, "t1").state == TaskState::Merged, RUN_WAIT);
    assert_eq!(
        t(&run, "t3").state,
        TaskState::Queued,
        "{:?}",
        t(&run, "t3")
    );
    assert_eq!(t(&run, "t2").state, TaskState::Working);
    assert_eq!(run.writer_caps, capped);
    h.release("t2");
    let run = h.wait_run(&id, complete, 3 * RUN_WAIT);
    for id in ["t1", "t2", "t3"] {
        assert_eq!(t(&run, id).state, TaskState::Merged, "{id}");
    }
    assert_eq!(run.rate_limits.get("claude"), Some(&1));

    // Every snapshot from t1's merge until t2 was past its check had t3 queued.
    // One read of the watcher: it keeps receiving snapshots after the run completes, so
    // three separate reads (one per task) could see lists of different lengths.
    let snapshots: Vec<(TaskInfo, TaskInfo, TaskInfo)> = (watcher.snapshots().into_iter())
        .flat_map(|s| s.runs)
        .filter(|r| r.run_id == id)
        .filter_map(|r| {
            let find = |task: &str| r.tasks.iter().find(|t| t.id == task).cloned();
            Some((find("t1")?, find("t2")?, find("t3")?))
        })
        .collect();
    let window: Vec<&TaskInfo> = (snapshots.iter())
        .filter(|(t1, t2, _)| t1.state == TaskState::Merged && !past_check(t2))
        .map(|(_, _, t3)| t3)
        .collect();
    assert!(
        !window.is_empty(),
        "no snapshot between t1's merge and t2's check"
    );
    for t3 in window {
        assert_eq!(t3.state, TaskState::Queued, "{t3:?}");
    }
    report_with(&run, "rate limit on claude: writers 2 → 1");
}

const ORCH: &str = "orchestrator-run-1";

#[test]
fn e2e_the_orchestrators_first_turn_waits_for_its_tools() {
    let h = RunHarness::orch("", &[]);
    h.script(
        ORCH,
        &[
            edit_plan(
                vec![add(plan_task("t1", &["a.txt"], json!({})))],
                json!({"submit": true}),
            ),
            read(None),
        ],
    );
    let id = h.start_goal_id("add a", &[]);
    let window = h.orchestrator_window(&id);
    h.wait_run(&id, |r| r.state == RunState::AwaitingApproval, ORCH_WAIT);

    // The first turn reached the session's terminal: `fake-agent` lists the anthrex
    // server's tools before it reads anything (decision 31), and the anthrex server sends
    // its ready notice on the first `tools/list` it answers.
    let first: Vec<Value> = (h.io_lines(ORCH, "stdin").iter())
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v.get("first_message").is_some())
        .collect();
    assert_eq!(first.len(), 1, "{first:?}");
    let text = first[0]["first_message"].as_str().unwrap_or_default();
    let opening = format!("[anthrex] You are the orchestrator of run {id} ");
    assert!(text.starts_with(&opening), "the first turn: {text}");
    let json = h.run_json(&id);
    assert_ne!(
        json["orch"]["orchestrator"]["first_turn_pending"],
        json!(true)
    );
    // It went because the ready notice came, not by the grace (ruling T5a-1), and the
    // driver pasted it as a wake-up (decision 38).
    let log = log_lines(&h, &id);
    assert!(
        !log.iter().any(|l| l.starts_with("first turn sent without")),
        "{log:#?}"
    );
    let daemon_log = std::fs::read_to_string(h.data().join("daemon.log")).unwrap_or_default();
    assert!(
        daemon_log.contains(&format!("wake-up delivered to window {window} (")),
        "{}",
        h.log_tail()
    );
}
