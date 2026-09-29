//! Milestone 9 task M9.14: the orchestrator's `anthrex run` surface, driven as the
//! real binary against the run harness's isolated daemon (`fake-agent` as the
//! orchestrator, the workers, the research scout and the triage decider): `--orchestrator`,
//! a planned goal's start, hold verdicts, `run status`, `run accept` of a research run,
//! `run promote` and the user's submit (decision 13).

mod support;

use std::path::Path;

use daemon::run::orch::contract::planned_message;
use proto::{HoldState, RunPath, RunState, RunsSnapshot, Runtime, TaskState};
use serde_json::{Value, json};
use support::run_adapt::triage_single;
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_orch::*;
use support::run_plans::*;

const ORCH: &str = "orchestrator-run-1";

/// One `anthrex run …` outcome.
struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

/// `anthrex run <args> --dir <repo>` with `input` on stdin.
fn run_in(h: &RunHarness, args: &[&str], input: &str) -> Out {
    let repo = h.repo.display().to_string();
    let mut all = vec!["run"];
    all.extend_from_slice(args);
    all.extend_from_slice(&["--dir", &repo]);
    let output = h.anthrex_input(&all, input);
    Out {
        code: output.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    }
}

fn run(h: &RunHarness, args: &[&str]) -> Out {
    run_in(h, args, "")
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

const BAD_ORCHESTRATOR: &str = "--orchestrator: expected claude or codex, optionally :<model>";

#[test]
fn orchestrator_flag_parses_and_refuses_bad_values() {
    let h = RunHarness::orch("", &[]);
    for bad in ["gemini", "", ":opus", "claude-opus", "gemini:x"] {
        let out = run(&h, &["start", "--goal", "g", "--orchestrator", bad]);
        refused_with(&out, BAD_ORCHESTRATOR);
        let out = run(&h, &["promote", "zzzz", "--orchestrator", bad]);
        refused_with(&out, BAD_ORCHESTRATOR);
    }
    assert!(h.snapshot().runs.is_empty(), "a refused flag started a run");
    // `claude:<model>` reaches the daemon: the planned run's orchestrator runs it (a
    // roster model; the daemon refuses any other in its own words).
    let out = run(
        &h,
        &[
            "start",
            "--goal",
            "g",
            "--orchestrator",
            "claude:test-model",
        ],
    );
    refused_with(&out, "claude:test-model is not in the roster");
    h.decider("triage", 2, triage_plan());
    let id = h.start_goal_id(
        "rework storage",
        &["--orchestrator", "claude:claude-sonnet-5"],
    );
    let route = h.run(&id).unwrap().orchestrator.unwrap().route;
    assert_eq!(
        (route.runtime, route.model.as_str()),
        (Runtime::Claude, "claude-sonnet-5")
    );
}

#[test]
fn orchestrator_flag_is_refused_with_plan() {
    let h = RunHarness::new("");
    let plan_path = h.plan(&plan("", &[task("t1", &["a.txt"], "")]));
    let plan_path = plan_path.display().to_string();
    let out = run(
        &h,
        &["start", "--plan", &plan_path, "--orchestrator", "claude"],
    );
    refused_with(&out, "--orchestrator applies to --goal runs");
    assert!(h.snapshot().runs.is_empty());
}

/// Pinning (M9.13 built the planned start): stdout the run id, stderr the planned
/// message, exit 0.
#[test]
fn start_goal_on_the_plan_path_prints_the_planned_message() {
    let h = RunHarness::orch("", &[]);
    let out = run(&h, &["start", "--goal", "rework storage"]);
    ok(&out);
    let id = out.stdout.trim().to_string();
    let info = h.run(&id).expect("the run is listed");
    assert_eq!(out.stdout, format!("{id}\n"));
    assert_eq!(info.state, RunState::Planning);
    let triage = info.triage.expect("its triage");
    assert_eq!(
        out.stderr,
        format!("{}\n", planned_message(&triage, &id, RunPath::Plan))
    );
}

/// A fast-path run promoted, whose orchestrator adds `t2` and submits: hold
/// `promotion` awaits the user while `t1` keeps working.
fn held_run() -> (RunHarness, String) {
    let h = RunHarness::orch("", &[]);
    h.decider("triage", 1, triage_single(&["a.txt"]));
    h.script("worker-t1-1", &[wait_for_file(&h.dir.path().join("go"))]);
    let t2 = json!({
        "id": "t2", "title": "Task t2", "brief": "Do it.", "acceptance": ["done"],
        "owns": ["t2.txt"], "size": "S", "test_mode": "check",
        "test_mode_reason": "a text file",
    });
    h.script(
        ORCH,
        &[
            json!({"mcp_call": {"tool": "edit_plan", "args": {
                "edits": [{"op": "add_task", "task": t2}], "submit": true}}}),
            read_message(),
        ],
    );
    let id = h.start_goal_id("add a", &[]);
    h.wait_run(&id, |r| t(r, "t1").state == TaskState::Working, RUN_WAIT);
    ok(&run(&h, &["promote", &id]));
    let awaiting = |r: &proto::RunInfo| {
        r.holds
            .iter()
            .any(|h| h.id == "promotion" && h.state == HoldState::Awaiting)
    };
    h.wait_run(&id, awaiting, ORCH_WAIT);
    (h, id)
}

fn hold_state(h: &RunHarness, id: &str) -> HoldState {
    h.run(id).unwrap().holds[0].state
}

#[test]
fn approve_and_reject_hold() {
    let (h, id) = held_run();
    refused_with(
        &run(&h, &["approve", &id, "--hold", "nope"]),
        &format!("run {id} has no hold nope"),
    );
    let out = run(&h, &["approve", &id, "--hold", "promotion"]);
    ok(&out);
    assert!(!out.stdout.is_empty(), "the daemon's reply is printed");
    assert_eq!(hold_state(&h, &id), HoldState::Approved);
    let info = h.run(&id).unwrap();
    assert_ne!(t(&info, "t2").state, TaskState::Cancelled);

    // `reject --hold` asks nothing: it cancels only held tasks that never started.
    let (h, id) = held_run();
    let out = run(&h, &["reject", &id, "--hold", "promotion"]);
    ok(&out);
    assert_eq!(out.stderr, "", "no question was asked");
    assert_eq!(hold_state(&h, &id), HoldState::Rejected);
    let info = h.run(&id).unwrap();
    assert_eq!(t(&info, "t2").state, TaskState::Cancelled);
    assert_eq!(t(&info, "t1").state, TaskState::Working);
    assert_eq!(info.state, RunState::Running);
}

#[test]
fn approve_without_hold_names_waiting_holds() {
    let (h, id) = held_run();
    refused_with(
        &run(&h, &["approve", &id]),
        &format!("run {id} has holds waiting for approval: promotion; pass --hold <id>"),
    );
    assert_eq!(hold_state(&h, &id), HoldState::Awaiting);
}

/// The held run's JSON carries M9's fields, and its text shows the orchestrator, the
/// hold and the held task.
#[test]
fn status_json_carries_the_new_fields() {
    let (h, id) = held_run();
    let out = run(&h, &["status", &id, "--json"]);
    ok(&out);
    let snapshot: RunsSnapshot = serde_json::from_str(&out.stdout).expect("a RunsSnapshot");
    let info = &snapshot.runs[0];
    let o = info.orchestrator.as_ref().unwrap();
    let (window, model) = (o.window_id.unwrap(), o.route.model.clone());
    assert_eq!(info.holds[0].id, "promotion");
    assert_eq!(info.holds[0].tasks, vec!["t2".to_string()]);
    assert_eq!(t(info, "t2").hold.as_deref(), Some("promotion"));
    let json: Value = serde_json::from_str(&out.stdout).unwrap();
    let run_json = &json["runs"][0];
    for field in ["orchestrator", "holds", "planners", "research_report"] {
        assert!(run_json.get(field).is_some(), "{field} missing");
    }

    let out = run(&h, &["status", &id]);
    ok(&out);
    assert!(
        out.stdout.contains(&format!(
            "\n  orchestrator: window {window}, claude {model}, live\n"
        )),
        "{}",
        out.stdout
    );
    assert!(
        out.stdout
            .contains("\n  holds: promotion awaiting (1 task)\n"),
        "{}",
        out.stdout
    );
    let row = out.stdout.lines().find(|l| l.starts_with("  t2 ")).unwrap();
    assert!(row.contains(" (held) "), "{row}");
}

/// Decision 35: a run of a research task only merges nothing; `run accept` names the
/// research report and asks the nothing-to-merge question instead of M8a's.
#[test]
fn accept_prints_the_research_report_and_the_nothing_to_merge_question() {
    let h = RunHarness::new("");
    h.script(
        "scout-r1-1",
        &[json!({"mcp_call": {"tool": "submit_scout_report", "args": {
            "summary": "README holds the readme.",
            "files": [{"path": "README", "why": "the readme"}]}}})],
    );
    let id = h.start(&plan("", &[task("r1", &[], "kind = \"research\"")]), true);
    let done = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(done.run_head, done.base_sha);
    let report = done.research_report.clone().expect("a research report");
    let named = format!("research report: {}\n", report.display());
    let question = format!("nothing to merge; accept run {id} and remove its branches? [y/N] ");

    let out = run_in(&h, &["accept", &id], "n\n");
    assert_eq!(out.code, 1, "{}", out.stderr);
    assert_eq!(
        out.stderr,
        format!(
            "{named}stdin is not a terminal; pass --yes (and --base <head> for a moved base)\n{question}\nnot merged\n"
        )
    );
    assert_eq!(h.run(&id).unwrap().state, RunState::Complete);

    let out = run(&h, &["accept", &id, "--yes"]);
    ok(&out);
    assert_eq!(out.stderr, named);
    assert_eq!(h.run(&id).unwrap().state, RunState::Accepted);
    assert!(no_run_branches(&h.repo));
}

/// A fast-path run whose `t1` keeps working: its id.
fn fast_run(h: &RunHarness) -> String {
    h.decider("triage", 1, triage_single(&["a.txt"]));
    h.script("worker-t1-1", &[wait_for_file(&h.dir.path().join("go"))]);
    let id = h.start_goal_id("add a", &[]);
    h.wait_run(&id, |r| t(r, "t1").state == TaskState::Working, RUN_WAIT);
    id
}

#[test]
fn promote_with_orchestrator_choice() {
    let h = RunHarness::orch("", &[]);
    let id = fast_run(&h);
    let out = run(&h, &["promote", &id, "--orchestrator", "codex:"]);
    ok(&out);
    let info = h.run(&id).unwrap();
    let route = info.orchestrator.expect("promoted").route;
    assert_eq!((route.runtime, route.model.as_str()), (Runtime::Codex, ""));
    assert_eq!(info.path, Some(RunPath::Plan));
}

/// Pinning (decision 29, M9.7): the repeat reply carries no time.
#[test]
fn promote_twice_answers_without_a_time() {
    let h = RunHarness::orch("", &[]);
    let id = fast_run(&h);
    ok(&run(&h, &["promote", &id]));
    let out = run(&h, &["promote", &id]);
    ok(&out);
    assert_eq!(
        out.stdout,
        format!("run {id} was already marked for promotion\n")
    );
}

/// Decision 13 (M9.7 review fixes, ruling 5): `run edit --submit`, alone or with
/// `--file`, submits a planning run's plan; any other state is refused.
#[test]
fn run_edit_submit_submits_a_planning_run() {
    let h = RunHarness::orch("", &[]);
    h.script(ORCH, &[read_message()]);
    let id = h.start_goal_id("rework storage", &[]);
    let add = |task_id: &str| {
        let file = h.dir.path().join(format!("{task_id}.toml"));
        std::fs::write(
            &file,
            format!(
                "[[edit]]\nop = \"add_task\"\n[edit.task]\nid = \"{task_id}\"\ntitle = \"Task {task_id}\"\nbrief = \"Do it.\"\nacceptance = [\"done\"]\nowns = [\"{task_id}.txt\"]\nsize = \"S\"\ntest_mode = \"check\"\ntest_mode_reason = \"a text file\"\n"
            ),
        )
        .unwrap();
        file.display().to_string()
    };
    // Neither is clap's error.
    let out = run(&h, &["edit", &id]);
    assert_eq!(out.code, 2, "{}", out.stderr);

    // With a file: the batch, then the submit.
    let out = run(&h, &["edit", &id, "--file", &add("t1"), "--submit"]);
    ok(&out);
    assert_eq!(
        out.stdout,
        format!("applied 1 edit; the plan of run {id} was submitted: it awaits approval\n")
    );
    assert_eq!(h.run(&id).unwrap().state, RunState::AwaitingApproval);
    refused_with(
        &run(&h, &["edit", &id, "--submit"]),
        &format!("run {id} is awaiting_approval; only a run being planned can be submitted"),
    );

    // Alone, on a second planned run.
    h.decider("triage", 2, triage_plan());
    let second = h.start_goal_id("rework mail", &[]);
    let out = run(&h, &["edit", &second, "--file", &add("m1")]);
    ok(&out);
    assert_eq!(h.run(&second).unwrap().state, RunState::Planning);
    let out = run(&h, &["edit", &second, "--submit"]);
    ok(&out);
    assert_eq!(
        out.stdout,
        format!("the plan of run {second} was submitted: it awaits approval\n")
    );
    assert_eq!(h.run(&second).unwrap().state, RunState::AwaitingApproval);
}
