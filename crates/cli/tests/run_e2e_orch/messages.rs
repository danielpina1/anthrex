//! Worker messages, refresh and task notes (decision 42, TT §12.7): the orchestrator's
//! `refresh` and `change` message after an interface merge, a worker's `task_note`,
//! `stop_and_wait`, a daemon restart, and the user's `run message`.

use std::path::Path;

use proto::{BlockReason, RunState, TaskState};
use serde_json::{Value, json};

use crate::common::*;
use crate::support::orch_script::*;
use crate::support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness};
use crate::support::run_orch::ORCH_WAIT;
use crate::support::run_plans::{approve, commit, done, user_texts};

const STOP_REFUSAL: &str = "this task was asked to stop and wait; wait for the next message";
const CHANGE: &str = "t0 changed the interface in iface.txt; build on it";

/// The messages recorded for task `id` in `run.json` (decision 42d).
fn recorded(h: &RunHarness, run: &str, id: &str) -> Vec<Value> {
    let json = h.run_json(run);
    let task = json["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["spec"]["id"] == id)
        .cloned()
        .unwrap();
    task["orch"]["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// For a second, while `id`'s worker turn is open, none of its recorded messages is
/// delivered: a message never interrupts a turn (decision 42).
fn held_while_the_turn_is_open(h: &RunHarness, run: &str, id: &str, n: usize) {
    let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
    while std::time::Instant::now() < until {
        let messages = recorded(h, run, id);
        assert_eq!(messages.len(), n, "{messages:?}");
        assert!(
            messages.iter().all(|m| m["delivered"] == false),
            "{messages:?}"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// The user texts `name`'s worker session read.
fn texts(h: &RunHarness, name: &str) -> Vec<String> {
    user_texts(&h.io_lines(name, "stdin"))
}

fn touch(path: &Path) {
    std::fs::write(path, "").unwrap();
}

/// A message edit through `edit_plan`, alone in its call.
fn message(to: &[&str], kind: &str, text: &str) -> Value {
    edit_plan(
        vec![json!({"op": "message", "to": to, "kind": kind, "text": text})],
        json!({}),
    )
}

#[test]
fn e2e_message_refresh_and_task_note() {
    let h = harness("max_writers = 4\n");
    let dir = h.dir.path();
    let (t1go, t2go, t3go, saw) = (
        dir.join("t1go"),
        dir.join("t2go"),
        dir.join("t3go"),
        dir.join("t1-saw-iface"),
    );
    green(&h, "t0", "iface.txt");
    let changes = "Changes applied: t1 now builds on the interface t0 merged.";
    h.script(
        "worker-t1-1",
        &[
            commit("t1.txt", "t1\n"),
            wait_file(&t1go),
            read(Some("Your branch now includes the latest merged work")),
            json!({"sh": {"cmd": format!("test -f iface.txt && touch '{}'", saw.display())}}),
            done(changes),
        ],
    );
    h.script(
        "worker-t2-1",
        &[
            wait_file(&t2go),
            call(
                "task_note",
                json!({"kind": "discovery", "text": "the hook fires twice"}),
            ),
            commit("t2.txt", "t2\n"),
            done("added t2.txt"),
        ],
    );
    h.script(
        "worker-t3-1",
        &[
            commit("t3.txt", "t3\n"),
            wait_file(&t3go),
            call_err("task_done", json!({"summary": "done early"})),
            read(Some("(stop_and_wait): hold on")),
            read(Some("(info): carry on")),
            done("added t3.txt"),
        ],
    );
    for id in ["t1", "t2", "t3"] {
        h.script(&format!("reviewer-{id}-1"), &[approve()]);
    }
    let tasks = [
        ("t0", "iface.txt", json!({"interface_change": true})),
        ("t1", "t1.txt", json!({})),
        ("t2", "t2.txt", json!({})),
        ("t3", "t3.txt", json!({})),
    ];
    let steps = [
        prompt(),
        edit_plan(
            tasks
                .iter()
                .map(|(id, file, more)| add(plan_task(id, &[file], more.clone())))
                .collect(),
            json!({"submit": true}),
        ),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        until("/tasks/0/state", json!("merged"), RUN_WAIT),
        edit_plan(vec![json!({"op": "refresh", "task_id": "t1"})], json!({})),
        message(&["t1"], "change", CHANGE),
        expect("/delivered", json!(["t1"])),
        message(&["t3"], "stop_and_wait", "hold on"),
        read(Some("t2 noted a discovery: the hook fires twice")),
        call("run_status", json!({})),
        expect("/task_notes/0/task", json!("t2")),
        expect("/task_notes/0/kind", json!("discovery")),
        message(&["t3"], "info", "carry on"),
        marker(),
        read(None),
    ];
    let (run, window) = start(&h, &steps);
    approve_plan(&h, &run);

    // The refresh, then the change message, each in its own call; t1's turn is open.
    h.wait_log(
        "the stop_and_wait to t3",
        |log| {
            log.iter().any(|l| {
                l["script"] == ORCH
                    && l["ok"] == true
                    && l["args"]["edits"][0]["kind"] == "stop_and_wait"
            })
        },
        RUN_WAIT,
    );
    held_while_the_turn_is_open(&h, &run, "t1", 1);
    touch(&t1go);

    // t3 is paused; its worker's `task_done` is refused with the exact text.
    let info = h.wait_run(
        &run,
        |r| {
            task(r, "t3")
                .block
                .as_ref()
                .is_some_and(|b| b.reason == BlockReason::MessagePause)
        },
        REQUEST_WAIT,
    );
    assert_eq!(task(&info, "t3").state, TaskState::Blocked);
    held_while_the_turn_is_open(&h, &run, "t3", 1);
    touch(&t3go);
    let log = h.wait_log(
        "t3's task_done",
        |log| {
            log.iter()
                .any(|l| l["script"] == "worker-t3-1" && l["tool"] == "task_done")
        },
        RUN_WAIT,
    );
    let refused = log
        .iter()
        .find(|l| l["script"] == "worker-t3-1" && l["tool"] == "task_done")
        .unwrap();
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["result"], STOP_REFUSAL);

    // t2's note wakes the orchestrator, which then releases t3.
    h.wait_orchestrator_idle(window);
    touch(&t2go);
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT * 4);
    wait_passed(&h, 1);
    for id in ["t0", "t1", "t2", "t3"] {
        assert_eq!(task(&info, id).state, TaskState::Merged, "{id}");
    }

    // t1's next turn carried both texts, its checkout had t0's file, and its summary
    // says how it applied the change.
    let turn = texts(&h, "worker-t1-1")
        .into_iter()
        .find(|t| t.contains("Your branch now includes the latest merged work"))
        .unwrap();
    let message = format!("[anthrex] Message from the orchestrator (change): {CHANGE}");
    assert!(turn.contains(&message), "{turn}");
    assert!(saw.exists(), "t1's checkout did not have iface.txt");
    let done_call = h
        .mcp_log()
        .into_iter()
        .find(|l| l["script"] == "worker-t1-1" && l["tool"] == "task_done")
        .unwrap();
    assert!(
        done_call["args"]["summary"]
            .as_str()
            .unwrap()
            .starts_with("Changes applied:"),
        "{done_call}"
    );
    let diff = task(&info, "t1").diff.expect("t1's measured diff");
    assert_eq!(diff.files, 1, "{diff:?}");
    assert!(
        recorded(&h, &run, "t1")
            .iter()
            .all(|m| m["delivered"] == true)
    );
    // t2's note was recorded, and it kept working; it is in the snapshot, as t2's.
    let noted = h
        .mcp_log()
        .into_iter()
        .find(|l| l["script"] == "worker-t2-1" && l["tool"] == "task_note")
        .unwrap();
    assert_eq!(noted["result"], "Note recorded. Keep working.", "{noted}");
    let notes = &task(&info, "t2").task_notes;
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert_eq!(notes[0].text, "the hook fires twice");
    let wake = h
        .read_messages(ORCH)
        .into_iter()
        .find(|m| m["text"].as_str().unwrap().contains("t2 noted a discovery"));
    assert!(wake.is_some(), "{:?}", h.read_messages(ORCH));
}

#[test]
fn e2e_messages_are_not_redelivered_after_a_restart() {
    let mut h = harness("");
    let dir = h.dir.path().to_path_buf();
    let (go1, go2, go3) = (dir.join("go1"), dir.join("go2"), dir.join("go3"));
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "t1\n"),
            wait_file(&go1),
            read(Some("note this")),
            wait_file(&go2),
            done("added a.txt"),
        ],
    );
    h.script(
        "worker-t2-1",
        &[
            commit("b.txt", "t2\n"),
            wait_file(&go3),
            read(Some("(stop_and_wait): wait for me")),
            read(Some("(info): go on")),
            done("added b.txt"),
        ],
    );
    for id in ["t1", "t2"] {
        h.script(&format!("reviewer-{id}-1"), &[approve()]);
    }
    let steps = [
        prompt(),
        edit_plan(
            vec![
                add(plan_task("t1", &["a.txt"], json!({}))),
                add(plan_task("t2", &["b.txt"], json!({}))),
            ],
            json!({"submit": true}),
        ),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        read(None),
        read(None),
    ];
    let (run, _) = start(&h, &steps);
    approve_plan(&h, &run);
    wait_task(&h, &run, "t1", TaskState::Working);
    wait_task(&h, &run, "t2", TaskState::Working);
    let user_message = |h: &RunHarness, kind: &str, to: &str, text: &str| {
        let mut args = vec!["run", "message", run.as_str(), "--kind", kind, to];
        args.extend(text.split(' '));
        ok(&h.anthrex(&args));
    };
    user_message(&h, "info", "t1", "note this");
    user_message(&h, "stop_and_wait", "t2", "wait for me");
    touch(&go1);
    touch(&go3);
    h.wait_run(
        &run,
        |_| {
            recorded(&h, &run, "t1")
                .iter()
                .all(|m| m["delivered"] == true)
        },
        RUN_WAIT,
    );
    let texts_before = |h: &RunHarness| {
        texts(h, "worker-t2-1")
            .iter()
            .any(|t| t.contains("(stop_and_wait): wait for me"))
    };
    crate::support::run_plans::until("t2's stop turn", RUN_WAIT, || {
        texts_before(&h).then_some(())
    });

    h.restart_daemon(&[]);
    h.wait_run(&run, |r| r.state == RunState::Paused, REQUEST_WAIT);
    ok(&h.anthrex(&["run", "resume", &run]));
    h.wait_run(&run, |r| r.state == RunState::Running, REQUEST_WAIT);
    // Decision 42c: a pause survives a restart and `run resume`.
    let info = h.run(&run).unwrap();
    let block = task(&info, "t2").block.clone().expect("t2 is still paused");
    assert_eq!(block.reason, BlockReason::MessagePause);

    touch(&go2);
    user_message(&h, "info", "t2", "go on");
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT * 2);
    for id in ["t1", "t2"] {
        assert_eq!(task(&info, id).state, TaskState::Merged, "{id}");
    }
    let seen = texts(&h, "worker-t1-1")
        .iter()
        .filter(|t| t.contains("note this"))
        .count();
    assert_eq!(seen, 1, "{:#?}", texts(&h, "worker-t1-1"));
    let stops = texts(&h, "worker-t2-1")
        .iter()
        .filter(|t| t.contains("wait for me"))
        .count();
    assert_eq!(stops, 1, "{:#?}", texts(&h, "worker-t2-1"));
}

#[test]
fn e2e_user_run_message_is_recorded_with_source_user() {
    let h = harness("");
    let go = h.dir.path().join("go");
    let text = "[anthrex] Message from the user (info): use lowercase";
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "t1\n"),
            wait_file(&go),
            read(Some(text)),
            done("kept it lowercase"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let steps = [
        prompt(),
        edit_plan(
            vec![add(plan_task("t1", &["a.txt"], json!({})))],
            json!({"submit": true}),
        ),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        read(None),
        read(None),
    ];
    let (run, _) = start(&h, &steps);
    approve_plan(&h, &run);
    wait_task(&h, &run, "t1", TaskState::Working);
    let out = h.anthrex(&[
        "run",
        "message",
        &run,
        "--kind",
        "info",
        "t1",
        "use",
        "lowercase",
    ]);
    ok(&out);
    touch(&go);
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT);
    assert_eq!(task(&info, "t1").state, TaskState::Merged);
    assert!(
        texts(&h, "worker-t1-1").iter().any(|t| t == text),
        "{:#?}",
        texts(&h, "worker-t1-1")
    );
    let entry = info
        .plan_edits
        .iter()
        .find(|e| e.text.starts_with("message t1"))
        .unwrap_or_else(|| panic!("{:#?}", info.plan_edits));
    assert_eq!(entry.text, "message t1 (info)");
    assert_eq!(entry.source, "user");
    assert_eq!(entry.recipients, vec!["t1".to_string()]);
    assert!(entry.accepted);
}
