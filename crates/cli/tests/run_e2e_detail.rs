//! Milestone 9.0.5 task 3: a worker's live activity line in the snapshot, and the task
//! detail request (`RunRequest::TaskDetail`, decisions 3, 4 and 7), end to end through a
//! real daemon with `fake-agent` as both runtimes.

mod support;

use proto::{DoneSignal, RunReply, RunRequest, SummarySource, TaskDetailInfo, TaskState};
use serde_json::{Value, json};
use support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness};
use support::run_plans::*;

/// A fragment of decision 32's `DONE_NUDGE`, the turn-end fallback's message.
const DONE_NUDGE: &str = "Your turn ended with commits in your worktree and no task_done";

fn print(text: &str) -> Value {
    json!({"print": text})
}

fn bash(cmd: &str) -> Value {
    json!({"bash": {"cmd": cmd}})
}

/// A tagged `TaskDetail` for `task` of run `run`, and its reply.
fn detail_reply(h: &RunHarness, id: u64, run: &str, task: &str) -> RunReply {
    let request = RunRequest::TaskDetail {
        run_id: run.into(),
        task_id: task.into(),
    };
    let reply = h.tagged(id, request, REQUEST_WAIT);
    assert_eq!(reply.request_id(), Some(id), "{reply:?}");
    reply
}

fn detail(h: &RunHarness, id: u64, run: &str, task: &str) -> TaskDetailInfo {
    match detail_reply(h, id, run, task) {
        RunReply::TaskDetail { detail, .. } => *detail,
        other => panic!("TaskDetail answered {other:?}"),
    }
}

/// The activity of `task` in every snapshot the watcher received.
fn activities(watcher: &support::run_harness::RunWatcher, task: &str) -> Vec<String> {
    watcher
        .snapshots()
        .iter()
        .flat_map(|s| s.runs.iter())
        .flat_map(|r| r.tasks.iter())
        .filter(|t| t.id == task)
        .filter_map(|t| t.activity.clone())
        .collect()
}

#[test]
fn claude_worker_activity_then_summary() {
    let h = RunHarness::new("");
    let watcher = h.subscribe();
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            print("Looking at the tests"),
            bash("cargo --version"),
            // The turn ends here with a commit: the worker waits on the fallback's nudge.
            read(DONE_NUDGE),
            print("Added the stats command and its test."),
            done("stats: mean, median; tests added"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    // While the worker waits on its `read_message`, its round is live and the snapshot
    // names its latest action. A turn end is structural, so it is published at once.
    let seen = until("t1's activity", RUN_WAIT, || {
        activities(&watcher, "t1")
            .into_iter()
            .find(|a| a == "Bash cargo --version" || a.starts_with("says: "))
    });
    assert!(
        seen == "Bash cargo --version" || seen == "says: Looking at the tests",
        "{seen}"
    );
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.done_signal, Some(DoneSignal::TaskDone));
    // Every round has ended: the activity is no longer published.
    assert_eq!(t1.activity, None);
    // Decision 16a still holds: the snapshot carries no plan text past the gate.
    assert!(t1.brief.is_empty() && t1.acceptance.is_empty());

    let got = detail(&h, 41, &id, "t1");
    assert_eq!(got.run_id, id);
    assert_eq!(got.task_id, "t1");
    assert_eq!(got.brief, "Do t1");
    assert_eq!(got.acceptance, ["t1 is done"]);
    assert_eq!(
        got.worker_summary.as_deref(),
        Some("stats: mean, median; tests added")
    );
    assert_eq!(got.summary_source, Some(SummarySource::TaskDone));
}

#[test]
fn codex_worker_summary_is_its_last_message() {
    let h = RunHarness::new("");
    // A Codex worker that never calls `task_done`: its turn ends with a commit, it
    // answers the fallback's nudge with a message and nothing else, and the fallback
    // completes it (as `e2e_turn_end_fallback_completes_a_silent_worker` does).
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            print("Looking at the tests"),
            read(DONE_NUDGE),
            print("Added the stats command and its test."),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], CODEX)]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.done_signal, Some(DoneSignal::TurnEndFallback));

    let got = detail(&h, 42, &id, "t1");
    assert_eq!(
        got.worker_summary.as_deref(),
        Some("Added the stats command and its test.")
    );
    assert_eq!(got.summary_source, Some(SummarySource::LastMessage));
}

#[test]
fn unknown_task_detail_is_refused() {
    let h = RunHarness::new("");
    // At the gate (no `--yes`): nothing is launched, and the run's tasks are known.
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), false);
    let refused_with = |reply: RunReply| match reply {
        RunReply::Refused {
            request, message, ..
        } => {
            assert_eq!(request, "run task-detail");
            message
        }
        other => panic!("expected a refusal, got {other:?}"),
    };
    assert_eq!(
        refused_with(detail_reply(&h, 7, &id, "t9")),
        format!("run {id} has no task t9")
    );
    assert_eq!(
        refused_with(detail_reply(&h, 8, "nope-0001", "t1")),
        "run nope-0001 has no task t1"
    );
    // The known task at the gate has its detail, and no summary yet.
    let got = detail(&h, 9, &id, "t1");
    assert_eq!(got.brief, "Do t1");
    assert_eq!((got.worker_summary, got.summary_source), (None, None));
}
