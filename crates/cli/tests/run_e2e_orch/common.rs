//! What every scenario of `run_e2e_orch` shares.

use std::process::Output;

use proto::{RunInfo, RunState, TaskInfo, TaskState};
use serde_json::Value;

use crate::support::orch_script::*;
use crate::support::run_harness::{RUN_WAIT, RunHarness};
use crate::support::run_orch::ORCH_WAIT;
use crate::support::run_plans::{approve, commit, done};

/// The orchestrator's script: the first orchestrator session of the run.
pub const ORCH: &str = "orchestrator-run-1";

/// A harness with the brief's M9 test configuration, `lines` more `[orchestrator]`
/// lines, Claude deciders and a triage that answers `plan`.
pub fn harness(lines: &str) -> RunHarness {
    RunHarness::orch(lines, &[])
}

/// Writes the orchestrator's script, starts the goal and waits for its window: the run
/// and the window.
pub fn start(h: &RunHarness, steps: &[Value]) -> (String, u32) {
    h.script(ORCH, steps);
    let run = h.start_goal_id("add the files", &[]);
    let window = h.orchestrator_window(&run);
    (run, window)
}

/// The green scripts of task `id`: its worker commits `file` and calls `task_done`;
/// its reviewer approves.
pub fn green(h: &RunHarness, id: &str, file: &str) {
    h.script(
        &format!("worker-{id}-1"),
        &[
            commit(file, &format!("{id}\n")),
            done(&format!("added {file}")),
        ],
    );
    h.script(&format!("reviewer-{id}-1"), &[approve()]);
}

/// Waits for the plan the orchestrator submitted, then approves it as the user does.
pub fn approve_plan(h: &RunHarness, run: &str) {
    h.wait_run(run, |r| r.state == RunState::AwaitingApproval, ORCH_WAIT);
    let out = h.anthrex(&["run", "approve", run]);
    assert!(out.status.success(), "{}", stderr(&out));
}

/// Waits until task `id` of `run` is `state`, at most `RUN_WAIT`.
pub fn wait_task(h: &RunHarness, run: &str, id: &str, state: TaskState) -> RunInfo {
    h.wait_run(
        run,
        |r| r.tasks.iter().any(|t| t.id == id && t.state == state),
        RUN_WAIT,
    )
}

pub fn task<'a>(run: &'a RunInfo, id: &str) -> &'a TaskInfo {
    run.tasks
        .iter()
        .find(|t| t.id == id)
        .expect("the task is listed")
}

pub fn stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

pub fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

/// Asserts `out` succeeded.
pub fn ok(out: &Output) {
    assert!(
        out.status.success(),
        "exit {:?}\nstdout: {}\nstderr: {}",
        out.status.code(),
        stdout(out),
        stderr(out)
    );
}

/// Waits until the orchestrator's script reached its `n`th [`marker`].
pub fn wait_passed(h: &RunHarness, n: usize) -> Vec<Value> {
    h.wait_log(
        "the orchestrator's script to pass its expectations",
        |log| passed(log, ORCH) >= n,
        RUN_WAIT,
    )
}

/// Waits until the orchestrator's `until("/gate/state", "approved")` step has ended: a
/// `run_status` of its script answered `approved`, so it reads no digest again before
/// its next `read_message`. A digest read drops the wake notes it held (decision 39, "a
/// read clears too"), so a worker that blocks before this would have its note read
/// away by the poll, and the wake-up a scenario waits for would never come.
pub fn wait_saw_approval(h: &RunHarness) {
    let approved = |l: &Value| {
        l["script"] == ORCH
            && l["tool"] == "run_status"
            && serde_json::from_str::<Value>(l["result"].as_str().unwrap_or_default())
                .is_ok_and(|r| r.pointer("/gate/state") == Some(&Value::from("approved")))
    };
    h.wait_log(
        "the orchestrator's script to see the approval",
        |log| log.iter().any(approved),
        ORCH_WAIT,
    );
}
