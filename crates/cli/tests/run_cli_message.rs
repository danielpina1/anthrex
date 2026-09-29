//! Milestone 9 task M9.14: `anthrex run message` and `anthrex run refresh` (decision
//! 42h), driven as the real binary against the run harness's isolated daemon, with
//! `fake-agent` as the worker. Every refusal is the daemon's own text (M9.13a).

mod support;

use std::path::Path;

use proto::{BlockReason, MessageKind, PlanEdit, RunReply, RunRequest, TaskState};
use serde_json::{Value, json};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

/// One `anthrex run …` outcome.
struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

/// `anthrex run <args> --dir <repo>`.
fn run(h: &RunHarness, args: &[&str]) -> Out {
    let repo = h.repo.display().to_string();
    let mut all = vec!["run"];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--dir", &repo]);
    let output = h.anthrex(&all);
    Out {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn ok(out: &Out) {
    assert_eq!(
        out.code, 0,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
}

/// Exit 1 with `message` as the whole of stderr's last line.
fn refused_with(out: &Out, message: &str) {
    assert_eq!(
        out.code, 1,
        "stdout: {}\nstderr: {}",
        out.stdout, out.stderr
    );
    assert_eq!(out.stderr.lines().last(), Some(message), "{}", out.stderr);
}

fn wait_for_file(path: &Path) -> Value {
    sh(&format!(
        "for i in $(seq 1 1500); do [ -e '{}' ] && exit 0; sleep 0.2; done; exit 1",
        path.display()
    ))
}

/// A run from a plan file whose `t1` worker runs `first`, then keeps its turn open
/// until the test writes `<tmp>/go`, then runs `after`; `t2` depends on `t1`, so it
/// stays pending. Returns the run id once `t1` is working in a live round.
fn working(h: &RunHarness, first: &[Value], after: &[Value]) -> String {
    let go = h.dir.path().join("go");
    let mut steps = first.to_vec();
    steps.push(wait_for_file(&go));
    steps.extend_from_slice(after);
    h.script("worker-t1-1", &steps);
    let plan_toml = plan(
        "",
        &[
            task("t1", &["a.txt"], ""),
            task("t2", &["b.txt"], "deps = [\"t1\"]"),
        ],
    );
    let id = h.start(&plan_toml, true);
    h.wait_run(
        &id,
        |r| {
            let t1 = t(r, "t1");
            t1.state == TaskState::Working && t1.rounds.iter().any(|r| r.ended_at.is_none())
        },
        RUN_WAIT,
    );
    id
}

fn go(h: &RunHarness) {
    std::fs::write(h.dir.path().join("go"), "").unwrap();
}

/// The daemon's refusal of `request`, sent raw.
fn raw_refusal(h: &RunHarness, request: RunRequest) -> String {
    match h.request(request) {
        RunReply::Refused { message, .. } => message,
        other => panic!("expected a refusal, got {other:?}"),
    }
}

/// Decision 42h and D-3: a task-id recipient gets the message as its next turn, and a
/// `stage:<n>` recipient parses and is refused by the daemon until milestone 9.1.
#[test]
fn run_message_reaches_the_selected_task_and_refuses_a_stage() {
    let h = RunHarness::new("");
    let text = "[anthrex] Message from the user (info): look at the README first";
    let id = working(&h, &[], &[json!({"read_message": {"expect": text}})]);
    let out = run(
        &h,
        &["message", &id, "t1", "look", "at", "the", "README", "first"],
    );
    ok(&out);
    assert_eq!(out.stdout, "applied 1 edit; message for t1\n");
    let info = h.run(&id).unwrap();
    assert_eq!(t(&info, "t1").message_count, 1);
    assert_eq!(
        t(&info, "t1").last_message_line.as_deref(),
        Some("look at the README first")
    );
    // Delivered as the worker's next turn, once its current one ends.
    go(&h);
    until("the worker read the message", RUN_WAIT, || {
        h.io_lines("worker-t1-1", "stdin")
            .iter()
            .any(|l| l.contains(text))
            .then_some(())
    });

    let stage = raw_refusal(
        &h,
        RunRequest::Edit {
            run_id: id.clone(),
            edits: vec![
                serde_json::from_value::<PlanEdit>(json!({
                    "op": "message", "to": "stage:1", "text": "hi", "kind": "info"
                }))
                .unwrap(),
            ],
            submit: false,
        },
    );
    assert!(
        stage.contains("stage recipients arrive with milestone 9.1"),
        "{stage}"
    );
    refused_with(&run(&h, &["message", &id, "stage:1", "hi"]), &stage);
    assert_eq!(t(&h.run(&id).unwrap(), "t1").message_count, 1);
}

/// `running` names every task with a live worker, found by state: never the pending
/// `t2`.
#[test]
fn run_message_running_targets_only_live_workers() {
    let h = RunHarness::new("");
    let id = working(&h, &[], &[]);
    let out = run(&h, &["message", &id, "running", "keep", "going"]);
    ok(&out);
    assert_eq!(out.stdout, "applied 1 edit; message for t1\n");
    let info = h.run(&id).unwrap();
    assert_eq!(t(&info, "t1").message_count, 1);
    assert_eq!(t(&info, "t2").message_count, 0);
    assert_eq!(info.plan_edits[0].recipients, vec!["t1".to_string()]);
}

/// `--kind` defaults to `info`; `change` and `stop_and_wait` are sent as given, and
/// `stop_and_wait` shows as `paused(message)` in `run status`.
#[test]
fn run_message_kind_defaults_to_info() {
    let h = RunHarness::new("");
    let id = working(&h, &[], &[]);
    ok(&run(&h, &["message", &id, "t1", "plain"]));
    let kind = |h: &RunHarness| t(&h.run(&id).unwrap(), "t1").last_message_kind;
    assert_eq!(kind(&h), Some(MessageKind::Info));
    ok(&run(
        &h,
        &["message", &id, "t1", "--kind", "change", "moved"],
    ));
    assert_eq!(kind(&h), Some(MessageKind::Change));
    ok(&run(
        &h,
        &["message", &id, "--kind", "stop_and_wait", "t1", "hold on"],
    ));
    let info = h.run(&id).unwrap();
    assert_eq!(kind(&h), Some(MessageKind::StopAndWait));
    let t1 = t(&info, "t1");
    assert_eq!(t1.state, TaskState::Blocked);
    assert_eq!(t1.block.as_ref().unwrap().reason, BlockReason::MessagePause);
    let status = run(&h, &["status", &id]);
    ok(&status);
    assert!(
        status
            .stdout
            .contains("\n  t1   S    check  paused(message) 0 "),
        "{}",
        status.stdout
    );
    let bad = run(&h, &["message", &id, "t1", "--kind", "loud", "x"]);
    assert_eq!(bad.code, 2, "{}", bad.stderr);
}

/// Decision 40 through the CLI: a message and a refresh are logged with source `user`,
/// a message with its resolved recipients; a refused recipient's line is printed after
/// the reply, in the daemon's words, and does not roll back the others.
#[test]
fn run_message_and_refresh_record_source_user() {
    let h = RunHarness::new("");
    let id = working(&h, &[], &[]);
    let edits = h.dir.path().join("cancel.toml");
    std::fs::write(&edits, "[[edit]]\nop = \"cancel_task\"\ntask_id = \"t2\"\n").unwrap();
    ok(&run(
        &h,
        &["edit", &id, "--file", &edits.display().to_string()],
    ));

    let out = run(&h, &["message", &id, "t1,t2", "hello"]);
    ok(&out);
    assert_eq!(
        out.stdout,
        "applied 1 edit; message for t1\nnot delivered: task t2 is cancelled; a message would not reach a worker\n"
    );
    let info = h.run(&id).unwrap();
    let entry = &info.plan_edits[0];
    assert_eq!(entry.source, "user");
    assert!(entry.accepted);
    assert_eq!(entry.recipients, vec!["t1".to_string()]);
    assert!(entry.text.starts_with("message "), "{}", entry.text);

    let out = run(&h, &["refresh", &id, "t1"]);
    ok(&out);
    assert_eq!(out.stdout, "applied 1 edit\n");
    let info = h.run(&id).unwrap();
    let entry = &info.plan_edits[0];
    assert_eq!(
        (entry.source.as_str(), entry.text.as_str()),
        ("user", "refresh t1")
    );
    assert!(entry.recipients.is_empty());
}

/// Decision 42e's clean-tree check at acceptance, in the daemon's words.
#[test]
fn run_refresh_refuses_uncommitted_work() {
    let h = RunHarness::new("");
    let id = working(&h, &[sh("echo dirty >> README")], &[]);
    let worktree = t(&h.run(&id).unwrap(), "t1").worktree.clone();
    until("the worker changed README", RUN_WAIT, || {
        std::fs::read_to_string(worktree.join("README"))
            .ok()
            .filter(|text| text.contains("dirty"))
    });
    let text = "task t1 has uncommitted changes; send it a message asking it to commit first";
    let raw = raw_refusal(
        &h,
        RunRequest::Edit {
            run_id: id.clone(),
            edits: vec![PlanEdit::Refresh {
                task_id: "t1".into(),
            }],
            submit: false,
        },
    );
    assert!(raw.contains(text), "{raw}");
    refused_with(&run(&h, &["refresh", &id, "t1"]), &raw);
    assert!(h.run(&id).unwrap().plan_edits.is_empty());
}

/// Decision 42h: the help names the three kinds, and `run --help` lists both commands.
#[test]
fn message_and_refresh_help_names_the_kinds() {
    let dir = support::tempdir();
    let help = |args: &[&str]| {
        let out = support::isolated_command(dir.path(), args)
            .output()
            .unwrap();
        assert!(out.status.success(), "{args:?}");
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    let text = help(&["run", "message", "--help"]);
    for kind in ["info", "change", "stop_and_wait"] {
        assert!(text.contains(kind), "{kind} missing from:\n{text}");
    }
    assert!(
        text.contains("stage:<n>") && text.contains("running"),
        "{text}"
    );
    let text = help(&["run", "refresh", "--help"]);
    assert!(text.contains("--kind change"), "{text}");
    let text = help(&["run", "--help"]);
    for command in ["message", "refresh"] {
        assert!(
            text.lines()
                .any(|l| l.trim_start().starts_with(&format!("{command} "))),
            "{command} missing from:\n{text}"
        );
    }
}
