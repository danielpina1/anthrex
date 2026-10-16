//! Milestone 9.17 task M9.17.10: plan and review quality end to end, through a real
//! daemon with `fake-agent` as the orchestrator, the workers and the reviewers; no real
//! agent. The plan frame's refusal and the gate's lines (decisions 14 and 19), a
//! `brief` finding routed to the orchestrator and, without one, to the user (decisions
//! 7 to 10), and `replaces` judged by the replacement (decisions 22 to 26).

use std::process::Output;

use proto::{AgentRole, HistoryStats, RunState, TaskState};
use serde_json::{Value, json};

use crate::support::orch_script::*;
use crate::support::run_harness::{RUN_WAIT, RunHarness};
use crate::support::run_orch::ORCH_WAIT;
use crate::support::run_plans::{
    approve, brief_finding, changes, commit, done, plan, task, user_texts,
};

/// The orchestrator's script: the first orchestrator session of the run.
const ORCH: &str = "orchestrator-run-1";

/// The reviewer's one finding, about the brief.
const WRONG_BRIEF: &str = "the brief asks for a.txt but the goal needs a.md";

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn ok(out: &Output) -> String {
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        stdout(out),
        String::from_utf8_lossy(&out.stderr)
    );
    stdout(out)
}

/// `anthrex run status <run>`'s text.
fn status(h: &RunHarness, run: &str) -> String {
    ok(&h.anthrex(&["run", "status", run]))
}

/// Writes the orchestrator's script, starts the goal and waits for its window: the run.
fn start(h: &RunHarness, steps: &[Value]) -> String {
    h.script(ORCH, steps);
    let run = h.start_goal_id("add the files", &[]);
    h.orchestrator_window(&run);
    run
}

/// Waits until the orchestrator's script reached its `n`th [`marker`].
fn wait_passed(h: &RunHarness, n: usize) -> Vec<Value> {
    h.wait_log(
        "the orchestrator's script to pass its expectations",
        |log| passed(log, ORCH) >= n,
        RUN_WAIT,
    )
}

/// The green scripts of task `id`: its worker commits `file` and calls `task_done`;
/// its reviewer approves.
fn green(h: &RunHarness, id: &str, file: &str) {
    h.script(
        &format!("worker-{id}-1"),
        &[
            commit(file, &format!("{id}\n")),
            done(&format!("added {file}")),
        ],
    );
    h.script(&format!("reviewer-{id}-1"), &[approve()]);
}

/// Task `t1`'s worker commits `a.txt`, calls `task_done` and then waits for a message
/// it must never get; its reviewer asks for changes with one critical `brief` finding.
fn brief_scripts(h: &RunHarness) {
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "t1\n"),
            done("added a.txt"),
            json!({"read_message": {}}),
        ],
    );
    h.script(
        "reviewer-t1-1",
        &[changes(&[brief_finding("critical", WRONG_BRIEF)])],
    );
}

/// Decision 7: `t1`'s worker read only its first prompt (no `[anthrex] Review round`
/// message), and its one session was the only one.
fn worker_was_never_bounced(h: &RunHarness, info: &proto::RunInfo) {
    let texts = user_texts(&h.io_lines("worker-t1-1", "stdin"));
    assert_eq!(texts.len(), 1, "{texts:#?}");
    assert!(
        texts.iter().all(|t| !t.contains("Review round")),
        "{texts:#?}"
    );
    let t1 = info.tasks.iter().find(|t| t.id == "t1").unwrap();
    let mut sessions: Vec<u32> = (t1.rounds.iter())
        .filter(|r| r.role == AgentRole::Worker)
        .map(|r| r.session)
        .collect();
    sessions.dedup();
    assert_eq!(sessions, [1], "{:#?}", t1.rounds);
    assert_eq!(t1.failures, 0);
}

/// Decisions 14 and 19: a submit without the frame is refused; the orchestrator reads
/// the refusal and submits with it; the gate's `run status` shows both lists, and they
/// are gone once the user approves.
#[test]
fn e2e_submit_without_frame_is_refused_then_gate_shows_it() {
    let h = RunHarness::orch("", &[]);
    green(&h, "t1", "a.txt");
    let edits = || vec![add(plan_task("t1", &["a.txt"], json!({})))];
    let steps = [
        prompt(),
        call_err("edit_plan", json!({"edits": edits(), "submit": true})),
        expect_error("submit needs decided"),
        // All or nothing: the refused call added no task.
        call("run_status", json!({})),
        expect("/tasks", json!([])),
        edit_plan(
            edits(),
            json!({"submit": true,
                "decided": [{"question": "Which store?", "answer": "SQLite"}],
                "out_of_scope": ["no UI"]}),
        ),
        expect("/awaiting_approval", json!(true)),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        marker(),
        read(None),
    ];
    let run = start(&h, &steps);
    h.wait_run(&run, |r| r.state == RunState::AwaitingApproval, ORCH_WAIT);

    // The refusal applied nothing, and the orchestrator read its exact text.
    let log = h.mcp_log();
    let refused = (log.iter())
        .find(|l| l["script"] == ORCH && l["tool"] == "edit_plan")
        .unwrap();
    assert_eq!(refused["ok"], false, "{refused}");
    assert!(
        (refused["result"].as_str().unwrap())
            .contains("send both with submit, as empty lists if there are none"),
        "{refused}"
    );
    let text = status(&h, &run);
    let goal = text.lines().position(|l| l.starts_with("  goal: "));
    let decided = (text.lines()).position(|l| l == "  decided for you: Which store? → SQLite");
    let out = (text.lines()).position(|l| l == "  out of scope: no UI");
    assert!(
        goal.is_some() && goal < decided && decided < out,
        "the frame after the goal:\n{text}"
    );

    ok(&h.anthrex(&["run", "approve", &run]));
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT);
    wait_passed(&h, 1);
    assert_eq!(info.plan_frame, None);
    let text = status(&h, &run);
    assert!(
        !text.contains("decided for you") && !text.contains("out of scope"),
        "{text}"
    );
}

/// Decisions 7 to 9: a critical `brief` finding does not bounce the worker; the task
/// merges, and the orchestrator reads the finding as a `brief` task note.
#[test]
fn e2e_brief_finding_reaches_the_orchestrator_not_the_worker() {
    let h = RunHarness::orch("", &[]);
    brief_scripts(&h);
    let steps = [
        prompt(),
        edit_plan(
            vec![add(plan_task("t1", &["a.txt"], json!({})))],
            json!({"submit": true, "decided": [], "out_of_scope": []}),
        ),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        until("/task_notes/0/kind", json!("brief"), RUN_WAIT),
        expect("/task_notes/0/task", json!("t1")),
        marker(),
        read(None),
    ];
    let run = start(&h, &steps);
    h.wait_run(&run, |r| r.state == RunState::AwaitingApproval, ORCH_WAIT);
    ok(&h.anthrex(&["run", "approve", &run]));
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT);
    wait_passed(&h, 1);
    let t1 = info.tasks.iter().find(|t| t.id == "t1").unwrap();
    assert_eq!(t1.state, TaskState::Merged);
    worker_was_never_bounced(&h, &info);

    // The note the orchestrator read, in its `run_status` digest.
    let log = h.mcp_log();
    let read = (log.iter())
        .filter(|l| l["script"] == ORCH && l["tool"] == "run_status" && l["ok"] == true)
        .filter_map(|l| serde_json::from_str::<Value>(l["result"].as_str()?).ok())
        .find(|r| r.pointer("/task_notes/0/kind") == Some(&json!("brief")))
        .expect("a run_status with the brief note");
    let note = read
        .pointer("/task_notes/0/text")
        .unwrap()
        .as_str()
        .unwrap();
    assert_eq!(note, format!("review round 1 (critical): {WRONG_BRIEF}"));
    // A run with an orchestrator has no sticky attention line (decision 10).
    let text = status(&h, &run);
    assert!(
        !text.contains("the reviewer says the brief is wrong"),
        "{text}"
    );
}

/// Decision 10: a plan-file run has no orchestrator, so the `brief` note is the user's:
/// a sticky attention line in `anthrex run status`.
#[test]
fn e2e_plan_file_brief_finding_is_an_attention_line() {
    let h = RunHarness::new("");
    brief_scripts(&h);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    let info = h.wait_run(&id, |r| r.state == RunState::Complete, RUN_WAIT);
    let t1 = info.tasks.iter().find(|t| t.id == "t1").unwrap();
    assert_eq!(t1.state, TaskState::Merged);
    worker_was_never_bounced(&h, &info);
    let text = status(&h, &id);
    assert!(
        text.contains(
            "  attention: t1: the reviewer says the brief is wrong (1 notes); read them in the task's notes\n"
        ),
        "{text}"
    );
}

/// Decisions 22 to 26: `t1` blocks with a question; the orchestrator replaces it with
/// `t2`, which cancels `t1`; `t2` merges, its history record names `t1`, and `run
/// stats` judges the work by `t2` alone.
#[test]
fn e2e_replaces_is_judged_by_the_replacement() {
    let h = RunHarness::orch("", &[]);
    let go = h.dir.path().join("go");
    // The worker blocks only once the orchestrator polls no more, so the block's note
    // wakes it (as `run_e2e_orch::steer`'s blocked question).
    h.script(
        "worker-t1-1",
        &[
            wait_file(&go),
            call(
                "task_blocked",
                json!({"kind": "question", "reason": "which file?"}),
            ),
            json!({"read_message": {}}),
        ],
    );
    green(&h, "t2", "b.txt");
    let m = json!({"size": "M"});
    let replacement = json!({"op": "add_task",
        "task": plan_task("t2", &["b.txt"], m.clone()), "replaces": ["t1"]});
    let steps = [
        prompt(),
        edit_plan(
            vec![add(plan_task("t1", &["a.txt"], m))],
            json!({"submit": true, "decided": [], "out_of_scope": []}),
        ),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        read(Some("t1 blocked (question)")),
        edit_plan(vec![replacement], json!({})),
        marker(),
        read(None),
    ];
    let run = start(&h, &steps);
    h.wait_run(&run, |r| r.state == RunState::AwaitingApproval, ORCH_WAIT);
    ok(&h.anthrex(&["run", "approve", &run]));
    h.wait_log(
        "the orchestrator's script to see the approval",
        |log| {
            log.iter().any(|l| {
                l["script"] == ORCH
                    && l["tool"] == "run_status"
                    && serde_json::from_str::<Value>(l["result"].as_str().unwrap_or_default())
                        .is_ok_and(|r| r.pointer("/gate/state") == Some(&json!("approved")))
            })
        },
        ORCH_WAIT,
    );
    std::fs::write(&go, "").unwrap();
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT * 2);
    wait_passed(&h, 1);
    let state = |id: &str| info.tasks.iter().find(|t| t.id == id).unwrap().state;
    assert_eq!(state("t1"), TaskState::Cancelled);
    assert_eq!(state("t2"), TaskState::Merged);
    let t2 = info.tasks.iter().find(|t| t.id == "t2").unwrap();
    assert_eq!(t2.replaces, ["t1"]);
    let t1 = info.tasks.iter().find(|t| t.id == "t1").unwrap();
    assert_eq!(t1.replaced_by, ["t2"]);

    // History: `t2`'s record names `t1`.
    let records = h.history_lines("task");
    let record = |id: &str| {
        (records.iter())
            .find(|l| l["run_id"] == run.as_str() && l["task_id"] == id)
            .unwrap_or_else(|| panic!("no {id} record: {records:#?}"))
    };
    assert_eq!(record("t2")["replaces"], json!(["t1"]));
    assert_eq!(record("t1")["outcome"], "cancelled", "{records:#?}");

    // Statistics: `t1` is replaced, and the M row counts `t2` only.
    let repo = h.repo.display().to_string();
    let out = ok(&h.anthrex(&["run", "stats", "--json", "--dir", &repo]));
    let stats: HistoryStats = serde_json::from_str(&out).expect("HistoryStats");
    assert_eq!(stats.replaced, 1, "{stats:#?}");
    let row = (stats.rows.iter())
        .find(|r| r.class == "M")
        .expect("an M row");
    assert_eq!((row.tasks, row.merged), (1, 1), "{row:?}");
    let text = ok(&h.anthrex(&["run", "stats", "--dir", &repo]));
    assert!(
        text.contains("replaced 1 (judged by their replacements)\n"),
        "{text}"
    );
}
