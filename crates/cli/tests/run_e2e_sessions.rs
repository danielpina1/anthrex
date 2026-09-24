//! Milestone 8a, task 25: headless sessions end to end — a daemon restart and the
//! resume, a process that dies mid-turn, a second death, decision 49's refusal of
//! client control, and permission denials — through a real daemon with `fake-agent` as
//! both runtimes.
//!
//! Process state is only ever read, never signalled: `ps` on the pids the run itself
//! recorded for its rounds.

mod support;

use std::collections::BTreeSet;
use std::time::Duration;

use proto::{
    AgentRole, BlockReason, ClientMsg, DaemonMsg, PlanEdit, RunReply, RunRequest, RunState,
    Runtime, Status, TaskState, WindowKind,
};
use serde_json::{Value, json};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

fn hang() -> Value {
    json!({"hang": {}})
}

fn exit(code: i32) -> Value {
    json!({"exit": code})
}

/// Task `id`'s persisted record in `run.json` (fields the snapshot does not show).
fn task_json(run: &proto::RunInfo, id: &str) -> Value {
    run_json(run)["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["spec"]["id"] == id)
        .cloned()
        .expect("the task in run.json")
}

/// Every pid `run.json` recorded for task `id`'s worker rounds (`pid`, `closed_pid`).
fn recorded_pids(run: &proto::RunInfo, id: &str) -> BTreeSet<u32> {
    task_json(run, id)["rounds"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["role"] == "worker")
        .flat_map(|r| [r["pid"].as_u64(), r["closed_pid"].as_u64()])
        .flatten()
        .map(|p| p as u32)
        .collect()
}

/// The argv of process `pid` as `ps` prints it, if it is alive (a read, never a
/// signal).
fn argv_of(pid: u32) -> Option<String> {
    let out = std::process::Command::new("ps")
        .args(["-ww", "-o", "args=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

/// Every argv `<name>` was started with (`FAKE_AGENT_ARGS_FILE`, one per process).
fn argvs(h: &RunHarness, name: &str) -> Vec<Vec<String>> {
    h.io_lines(name, "args")
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

fn worker_round<'a>(run: &'a proto::RunInfo, id: &str) -> &'a proto::AgentRoundInfo {
    t(run, id)
        .rounds
        .iter()
        .rev()
        .find(|r| r.role == AgentRole::Worker)
        .expect("a worker round")
}

#[test]
fn e2e_daemon_restart_pauses_and_resume_continues() {
    let mut h = RunHarness::new("");
    // The first turn is held open (`hang`) rather than ended: a worker turn that ends
    // with commits and no task_done gets decision 32's DONE_NUDGE at once, which a
    // `read_message` expecting the restart message would take instead. See the
    // Implementation notes for M8a.25. `t3`'s session is idle at the restart instead:
    // its task is blocked on a question, its turn ended (M8a.25 fix round 1).
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            hang(),
            read("The daemon restarted"),
            done("added a"),
        ],
    );
    h.script(
        "worker-t2-1",
        &[
            commit("b.txt", "b\n"),
            hang(),
            read("The daemon restarted"),
            done("added b"),
        ],
    );
    h.script(
        "worker-t3-1",
        &[
            blocked_question("which file?"),
            read("Answer to your question: c.txt"),
            commit("c.txt", "c\n"),
            done("added c"),
        ],
    );
    for task in ["t1", "t2", "t3"] {
        h.script(&format!("reviewer-{task}-1"), &[approve()]);
    }
    let claude = "route = { runtime = \"claude\" }";
    let plan = plan(
        "",
        &[
            task("t1", &["a.txt"], claude),
            task("t2", &["b.txt"], CODEX),
            task("t3", &["c.txt"], claude),
        ],
    );
    let id = h.start(&plan, true);
    let committed = |run: &proto::RunInfo, task: &str, file: &str| {
        t(run, task)
            .rounds
            .iter()
            .any(|r| r.role == AgentRole::Worker && r.session_id.is_some())
            && t(run, task).worktree.join(file).exists()
            && git_read(&t(run, task).worktree, &["log", "-1", "--format=%s"])
                .is_some_and(|s| s.contains(&format!("add {file}")))
    };
    let run = h.wait_run(
        &id,
        |r| {
            committed(r, "t1", "a.txt")
                && committed(r, "t2", "b.txt")
                && t(r, "t3").state == TaskState::Blocked
                && !worker_round(r, "t3").turn_open
        },
        RUN_WAIT,
    );
    let ids = ["t1", "t2", "t3"].map(|task| worker_round(&run, task).session_id.clone().unwrap());
    let windows = ["t1", "t2", "t3"].map(|task| worker_round(&run, task).window_id.unwrap());
    let before = until("the workers' pids in run.json", RUN_WAIT, || {
        let run = h.run(&id)?;
        let pids: BTreeSet<u32> = ["t1", "t2", "t3"]
            .iter()
            .flat_map(|task| recorded_pids(&run, task))
            .collect();
        (pids.len() >= 3).then_some(pids)
    });
    let first_t1 = argvs(&h, "worker-t1-1")
        .first()
        .cloned()
        .expect("t1's first argv");
    let first_t3 = argvs(&h, "worker-t3-1")
        .first()
        .cloned()
        .expect("t3's first argv");

    h.restart_daemon(&[]);
    let run = h.run(&id).expect("the run is restored");
    assert_eq!(run.state, RunState::Paused);
    assert_eq!(run.paused_from, Some(RunState::Running));
    let listed = h.windows();
    for window in windows {
        let info = listed
            .iter()
            .find(|w| w.id == window)
            .unwrap_or_else(|| panic!("window {window} is listed: {listed:#?}"));
        assert_eq!(info.kind, WindowKind::Headless);
        assert_eq!(info.status, Status::Exited, "{info:#?}");
    }
    let mut pids = before;
    for task in ["t1", "t2", "t3"] {
        pids.extend(recorded_pids(&run, task));
    }
    for pid in pids {
        if let Some(argv) = argv_of(pid) {
            for session in &ids {
                assert!(
                    !argv
                        .split_whitespace()
                        .any(|a| a.contains(session.as_str())),
                    "pid {pid} still runs with session {session}: {argv}"
                );
            }
        }
    }

    let output = h.anthrex(&["run", "resume", &id]);
    assert!(
        output.status.success(),
        "run resume: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // The idle session takes its next message, the answer, as a resume.
    match h.request(RunRequest::Edit {
        run_id: id.clone(),
        edits: vec![PlanEdit::Answer {
            task_id: "t3".into(),
            text: "c.txt".into(),
        }],
    }) {
        RunReply::Done { .. } => {}
        other => panic!("the answer: {other:?}"),
    }
    let run = h.wait_run(&id, complete, RUN_WAIT);
    for task in ["t1", "t2", "t3"] {
        assert_eq!(t(&run, task).state, TaskState::Merged, "{task}");
        assert_eq!(t(&run, task).failures, 0, "{task}");
    }

    // Every flag of the first argv, each in its place, with `--resume <id>` where
    // `--session-id <id>` was.
    for (name, first, session) in [
        ("worker-t1-1", &first_t1, &ids[0]),
        ("worker-t3-1", &first_t3, &ids[2]),
    ] {
        let expected: Vec<String> = first
            .iter()
            .map(|a| {
                if a == "--session-id" {
                    "--resume".into()
                } else {
                    a.clone()
                }
            })
            .collect();
        assert!(
            expected
                .windows(2)
                .any(|w| w[0] == "--resume" && w[1] == **session),
            "{name}: {first:#?}"
        );
        let all = argvs(&h, name);
        assert!(
            all.contains(&expected),
            "{name}: no resume argv equal to {expected:#?}\nin {all:#?}"
        );
    }
    let codex = argvs(&h, "worker-t2-1");
    let resumed = codex
        .iter()
        .find(|a| a.len() > 2 && a[0] == "exec" && a[1] == "resume" && a[2] == ids[1])
        .unwrap_or_else(|| panic!("no exec resume {}: {codex:#?}", ids[1]));
    assert!(
        resumed.last().unwrap().ends_with(daemon_resume_worker()),
        "{resumed:#?}"
    );
}

/// `contract::RESUME_WORKER`, the text a resumed worker's message ends with.
fn daemon_resume_worker() -> &'static str {
    "[anthrex] The daemon restarted. Re-read your task above, continue, commit, and call task_done when complete."
}

#[test]
fn e2e_process_that_dies_mid_turn_is_resumed_once() {
    let h = RunHarness::new("");
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            exit(1),
            read("stopped in the middle of a turn"),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.failures, 0);
    let rounds = task_json(&run, "t1")["rounds"].clone();
    let worker = rounds
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["role"] == "worker")
        .unwrap();
    assert_eq!(worker["deaths"], 1, "{worker:#}");
    let all = argvs(&h, "worker-t1-1");
    assert_eq!(all.len(), 2, "{all:#?}");
    assert!(all[1].iter().any(|a| a == "--resume"), "{all:#?}");
}

#[test]
fn e2e_a_second_death_in_a_round_is_a_stall() {
    let h = RunHarness::new("");
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            exit(1),
            read("stopped in the middle of a turn"),
            exit(1),
        ],
    );
    h.script("worker-t1-2", &[commit("b.txt", "b\n"), done("finished")]);
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["a.txt", "b.txt"], "")]), true);
    // Session 1's path, then session 2's (k = 2).
    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.stalls, 1);
    assert!(
        t1.rounds
            .iter()
            .any(|r| r.role == AgentRole::Worker && r.session == 2),
        "{:#?}",
        t1.rounds
    );
    assert!(!argvs(&h, "worker-t1-2").is_empty(), "worker-t1-2 ran");
    assert_eq!(
        h.git(&["show", &format!("anthrex/{id}/integration:b.txt")]),
        "b"
    );
}

/// How long the daemon holds a `CreateWindow`'s `done` line (the debug builds'
/// `ANTHREX_TEST_DELAY_WINDOW_MS`): far over `fake-agent`'s start and exit (tens of
/// milliseconds), so the session's exit reaches the engine before its round has the
/// window, and is dropped (the race in the followups file, From M8a.25).
const WINDOW_HOLD_MS: &str = "2000";
/// Session 2's and the reviewer's own wait before they work, so their tool calls come
/// after their windows are known (the hold applies to every `CreateWindow`): twice the
/// hold.
const AFTER_HOLD_MS: u64 = 4000;
/// Over `AFTER_HOLD_MS`, so session 2's wait is not itself a stall.
const STALL_AFTER_SECS: u64 = 10;
/// `engine::signals::INTERRUPT_GRACE_SECS`: the interrupt of a session with no process
/// never ends its turn, so the grace runs out.
const INTERRUPT_GRACE_SECS: u64 = 30;

/// M8a.25 fix round 1 (review finding 1): a worker whose process exits before its
/// `CreateWindow` result is stepped has its exit dropped, and its round recorded that
/// dead process's pid. The stall's kill then waited for an exit that had already come,
/// the round never ended and rung 2's fresh session never started.
#[test]
fn e2e_a_session_that_exits_before_its_window_is_known_still_escalates() {
    let h = RunHarness::with_env(
        &format!("stall_after_secs = {STALL_AFTER_SECS}"),
        &[("ANTHREX_TEST_DELAY_WINDOW_MS", WINDOW_HOLD_MS)],
        true,
    );
    h.script("worker-t1-1", &[exit(1)]);
    h.script(
        "worker-t1-2",
        &[
            json!({"wait_ms": AFTER_HOLD_MS}),
            commit("a.txt", "a\n"),
            done("added a"),
        ],
    );
    h.script(
        "reviewer-t1-1",
        &[json!({"wait_ms": AFTER_HOLD_MS}), approve()],
    );
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    // Session 1's path, then session 2's (k = 2), and the stall and the interrupt's
    // grace waited out on the way.
    let wait = 2 * RUN_WAIT + Duration::from_secs(STALL_AFTER_SECS + INTERRUPT_GRACE_SECS);
    let run = h.wait_run(&id, complete, wait);
    let log = std::fs::read_to_string(h.data().join("daemon.log")).unwrap_or_default();
    assert!(
        log.contains("ANTHREX_TEST_DELAY_WINDOW_MS: holding op done lines"),
        "the window hold was not armed:\n{log}"
    );
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.stalls, 1);
    assert!(
        t1.rounds
            .iter()
            .any(|r| r.role == AgentRole::Worker && r.session == 2),
        "{:#?}",
        t1.rounds
    );
}

fn blocked_question(reason: &str) -> Value {
    json!({"mcp_call": {"tool": "task_blocked", "args": {"kind": "question", "reason": reason}}})
}

#[test]
fn e2e_headless_windows_refuse_client_control_while_the_engine_delivers() {
    let h = RunHarness::new("");
    h.script(
        "worker-t1-1",
        &[
            blocked_question("which file?"),
            read("Answer to your question: a.txt"),
            commit("a.txt", "a\n"),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    let run = h.wait_run(
        &id,
        |r| {
            t(r, "t1")
                .block
                .as_ref()
                .is_some_and(|b| b.reason == BlockReason::Question)
        },
        RUN_WAIT,
    );
    let window = worker_round(&run, "t1").window_id.unwrap();
    let session = worker_round(&run, "t1").session_id.clone().unwrap();
    let pid = until("the worker's pid in run.json", RUN_WAIT, || {
        recorded_pids(&h.run(&id)?, "t1").into_iter().next()
    });

    let client = h.watch(None);
    let control = format!(
        "window {window} is a headless session of run {id}; only the engine drives it. Use anthrex run cancel to stop it"
    );
    let cases = [
        (
            ClientMsg::Subscribe {
                window_id: window,
                cols: 80,
                rows: 24,
            },
            "subscribe",
            format!("window {window} is a headless session; open its conversation with C-b m"),
        ),
        (
            ClientMsg::Input {
                window_id: window,
                bytes: b"hello\r".to_vec(),
            },
            "input",
            control.clone(),
        ),
        (
            ClientMsg::Kill { window_id: window },
            "kill",
            control.clone(),
        ),
        (
            ClientMsg::Remove {
                window_id: window,
                remove_worktree: false,
                force: true,
            },
            "remove",
            control.clone(),
        ),
        (
            ClientMsg::Restart { window_id: window },
            "restart",
            control.clone(),
        ),
    ];
    for (msg, want_request, want_message) in cases {
        client.send(msg);
        match client.wait_for(Duration::from_secs(10), |m| {
            matches!(m, DaemonMsg::Error { .. })
        }) {
            DaemonMsg::Error { request, message } => {
                assert_eq!(request, want_request);
                assert_eq!(message, want_message);
            }
            _ => unreachable!(),
        }
        let info = h
            .windows()
            .into_iter()
            .find(|w| w.id == window)
            .expect("the worker window is still listed");
        assert_ne!(info.status, Status::Exited, "{info:#?}");
        let argv = argv_of(pid).unwrap_or_else(|| panic!("pid {pid} is gone"));
        assert!(argv.contains(&session), "{argv}");
    }

    match h.request(RunRequest::Edit {
        run_id: id.clone(),
        edits: vec![PlanEdit::Answer {
            task_id: "t1".into(),
            text: "a.txt".into(),
        }],
    }) {
        RunReply::Done { .. } => {}
        other => panic!("the answer: {other:?}"),
    }
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    let texts = user_texts(&h.io_lines("worker-t1-1", "stdin"));
    assert!(
        texts
            .iter()
            .any(|t| t == "[anthrex] Answer to your question: a.txt"),
        "{texts:#?}"
    );
}

#[test]
fn e2e_permission_denials_block_the_task_as_environment() {
    let h = RunHarness::new("denials_before_block = 2");
    let deny = json!({"deny": {"tool": "Write", "reason": "not allowed"}});
    h.script("worker-t1-1", &[deny.clone(), deny, hang()]);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    let run = h.wait_run(&id, |r| t(r, "t1").state == TaskState::Blocked, RUN_WAIT);
    let block = t(&run, "t1").block.clone().unwrap();
    assert_eq!(block.reason, BlockReason::Environment);
    let text = "the agent was denied 2 times; last: Write: not allowed";
    assert_eq!(block.text, text);
    assert!(
        run.attention
            .iter()
            .any(|l| l.starts_with("t1 blocked (environment)") && l.contains(text)),
        "{:?}",
        run.attention
    );
    let window = worker_round(&run, "t1").window_id.unwrap();
    until("the worker window to exit", RUN_WAIT, || {
        h.windows()
            .into_iter()
            .find(|w| w.id == window && w.status == Status::Exited)
    });
    assert_eq!(worker_round(&run, "t1").route.runtime, Runtime::Claude);
}
