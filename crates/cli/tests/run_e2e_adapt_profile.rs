//! Milestone 8b, task 19 (II): the stored repository profile and the run history end to
//! end, through a real daemon with `fake-agent` as both runtimes: the stored profile
//! wins over the plan's, a corrupt or confinement-widening one refuses the run, its
//! `generated` and `protected` lists bounce at rung 1, and `history.jsonl` records a
//! fast-path run, survives a crash after its append intent and counts a revert.

mod support;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use daemon::profile::store::{PROFILE_FILE, Stored};
use daemon::run::contract::{generated_files_message, protected_file_message};
use daemon::run::history_io::read_history;
use proto::{
    HistoryLine, HistoryStats, RunPath, RunRecord, RunState, TaskOutcome, TaskRecord, TaskState,
};
use serde_json::Value;
use support::run_adapt::{ADAPT_FILES, STORED_PROFILE, triage_single};
use support::run_harness::{REQUEST_WAIT, RUN_WAIT, RunHarness};
use support::run_plans::*;

/// A harness running deciders in `mode`, with the brief's stored profile plus
/// `profile` lines, and the base files it needs plus `files`.
fn harness(mode: &str, profile: &str, env: &[(&str, &str)], files: &[(&str, &str)]) -> RunHarness {
    let mut all: Vec<(&str, &str)> = ADAPT_FILES.to_vec();
    all.extend_from_slice(files);
    let h = RunHarness::adapt(mode, "", env, &all);
    h.stored_profile(&format!("{STORED_PROFILE}{profile}"));
    h
}

/// The one-task plan every plan-path test starts: `t1` owning `owns`.
fn one_task_plan(owns: &[&str]) -> String {
    plan("", &[task("t1", owns, "")])
}

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// `anthrex run accept <id> --yes`, which must succeed.
fn accept(h: &RunHarness, id: &str) {
    let repo = h.repo.display().to_string();
    let out = h.anthrex_input(&["run", "accept", id, "--yes", "--dir", &repo], "");
    assert!(
        out.status.success(),
        "stdout: {}\nstderr: {}",
        stdout(&out),
        stderr(&out)
    );
}

/// Task `id`'s entry in the run's `run.json`.
fn task_json(run: &proto::RunInfo, id: &str) -> Value {
    run_json(run)["tasks"]
        .as_array()
        .expect("run.json lists its tasks")
        .iter()
        .find(|t| t["spec"]["id"] == id)
        .cloned()
        .expect("the task is in run.json")
}

/// Whether task `id`'s failure log in `run.json` holds `message` exactly.
fn failure_logged(run: &proto::RunInfo, id: &str, message: &str) -> bool {
    task_json(run, id)["failure_log"]
        .as_array()
        .is_some_and(|log| log.iter().any(|entry| entry == message))
}

/// Decision 6's refusal for the stored profile of `repo_dir`, whose file the test just
/// broke: the message `run start` must answer, with the parse error `store::load`
/// itself reports for that file.
fn unparseable_refusal(repo_dir: &Path) -> (String, String) {
    match daemon::profile::store::load(repo_dir) {
        Stored::Unparseable { path, error } => {
            assert_eq!(path, repo_dir.join(PROFILE_FILE));
            let message = format!(
                "the stored profile at {} does not parse: {error}; fix it with anthrex profile edit or re-detect it with anthrex profile detect",
                path.display()
            );
            (message, error)
        }
        other => panic!("the broken profile loaded as {other:?}"),
    }
}

/// A plan-path `run start` refused with exactly `message`, and nothing created.
fn start_refused(h: &RunHarness, message: &str) {
    let reply = h.start_reply(&h.repo, &one_task_plan(&["a.txt"]), true, false);
    assert_eq!(refused(reply), message);
    assert!(no_run_branches(&h.repo), "a run branch was created");
    assert!(h.snapshot().runs.is_empty());
}

#[test]
fn e2e_stored_profile_wins_over_the_plan_profile() {
    let h = harness("off", "", &[], &[]);
    green_scripts(&h.repo);
    let plan = one_task_plan(&["a.txt"]).replace("check = \"true\"", "check = \"false\"");
    assert!(plan.contains("check = \"false\""), "{plan}");
    let id = h.start(&plan, true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    assert_eq!(run.profile_source, Some(proto::ProfileSource::Stored));

    let json = run_json(&run);
    assert_eq!(json["profile"]["check"], "sh check.sh");
    let note = format!(
        "profile.check from the plan file is ignored: this repository has a stored profile ({})",
        h.repo_dir().join(PROFILE_FILE).display()
    );
    let logged = json["log"]
        .as_array()
        .expect("run.json has a log")
        .iter()
        .any(|entry| entry["text"] == note.as_str());
    assert!(logged, "no {note:?} in {}", json["log"]);
    // Every check the run made passed: the plan's `false` never ran.
    let checks = task_json(&run, "t1")["checks"].clone();
    let checks = checks.as_array().expect("t1's checks");
    assert!(!checks.is_empty());
    assert!(checks.iter().all(|c| c["code"] == 0), "{checks:?}");
}

#[test]
fn e2e_a_corrupt_stored_profile_refuses_the_run() {
    let h = harness("off", "", &[], &[]);
    let repo_dir = h.repo_dir();
    std::fs::write(repo_dir.join(PROFILE_FILE), "check = \n").unwrap();
    let (message, _) = unparseable_refusal(&repo_dir);
    assert!(
        message.contains(&repo_dir.join(PROFILE_FILE).display().to_string()),
        "{message}"
    );
    start_refused(&h, &message);
}

#[test]
fn e2e_a_stored_profile_cannot_widen_confinement() {
    let h = harness("off", "", &[], &[]);
    let repo_dir = h.repo_dir();
    std::fs::write(
        repo_dir.join(PROFILE_FILE),
        format!("{STORED_PROFILE}cache_dirs = [\"/\"]\n"),
    )
    .unwrap();
    let (message, error) = unparseable_refusal(&repo_dir);
    assert!(error.contains("cache_dirs"), "{error}");
    start_refused(&h, &message);
}

#[test]
fn e2e_generated_lock_file_bounces_at_rung_1() {
    let h = harness("off", "", &[], &[("Cargo.lock", "version = 3\n")]);
    h.script(
        "worker-t1-1",
        &[
            sh("printf 'a\\n' > a.txt && printf 'changed\\n' >> Cargo.lock && git add -A && git commit -qm work"),
            done_expecting_error(),
            sh("git checkout HEAD~1 -- Cargo.lock && git commit -qm \"revert Cargo.lock\""),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    // The plan names no `generated`: the list is the stored profile's.
    let id = h.start(&one_task_plan(&["a.txt"]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!((t1.bounces.done, t1.rung), (1, 1));
    let message = generated_files_message(&["Cargo.lock".to_string()]);
    assert!(
        failure_logged(&run, "t1", &message),
        "{}",
        task_json(&run, "t1")["failure_log"]
    );
    let lock = h.git(&["show", &format!("anthrex/{id}/integration:Cargo.lock")]);
    assert_eq!(lock, "version = 3");
}

#[test]
fn e2e_protected_file_from_the_stored_profile_bounces_at_rung_1() {
    let h = harness(
        "off",
        "protected = [\"docs/agents.txt\"]\n",
        &[],
        &[("docs/agents.txt", "base\n")],
    );
    h.script(
        "worker-t1-1",
        &[
            commit("a.txt", "a\n"),
            commit("docs/agents.txt", "changed\n"),
            done_expecting_error(),
            sh(
                "printf '%s' \"$FAKE_AGENT_RESULT\" | grep -qF 'docs/agents.txt configures or instructs future agents' && git checkout HEAD~1 -- docs/agents.txt && git commit -qm revert",
            ),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&one_task_plan(&["**"]), true);
    let run = h.wait_run(&id, complete, RUN_WAIT);
    // The protected path is the stored profile's: the plan names none.
    let protected = run_json(&run)["protected_files"].clone();
    assert!(
        protected
            .as_array()
            .is_some_and(|p| p.iter().any(|f| f == "docs/agents.txt")),
        "{protected}"
    );
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!((t1.bounces.done, t1.rung), (1, 1));
    let message = protected_file_message(&["docs/agents.txt".to_string()]);
    assert!(
        failure_logged(&run, "t1", &message),
        "{}",
        task_json(&run, "t1")["failure_log"]
    );
    let integration = format!("anthrex/{id}/integration");
    assert_eq!(
        h.git(&["show", &format!("{integration}:docs/agents.txt")]),
        "base"
    );
    assert_eq!(h.git(&["show", &format!("{integration}:a.txt")]), "a");
}

/// `<repo_dir>/history.jsonl`.
fn history_path(h: &RunHarness) -> PathBuf {
    h.repo_dir().join(daemon::run::engine::HISTORY_FILE)
}

/// The history's task and run records, read as `run stats` reads them (the last line
/// per `record_id`), with no problem reported.
fn history(path: &Path) -> (Vec<TaskRecord>, Vec<RunRecord>) {
    let (lines, problems) = read_history(path);
    assert!(problems.is_empty(), "{problems:?}");
    let mut tasks = Vec::new();
    let mut runs = Vec::new();
    for line in lines {
        match line {
            HistoryLine::Task(record) => tasks.push(record),
            HistoryLine::Run(record) => runs.push(record),
            HistoryLine::Revert(_)
            | HistoryLine::RoleRoute(_)
            | HistoryLine::Tier(_)
            | HistoryLine::Flaky(_)
            | HistoryLine::Bisect(_)
            | HistoryLine::Stage(_)
            | HistoryLine::Round(_)
            | HistoryLine::Phase(_) => {}
        }
    }
    (tasks, runs)
}

/// Waits, at most `REQUEST_WAIT`, for the run record of `id`: `run accept` answers
/// before its `AppendHistory` lines are written.
fn wait_run_record(h: &RunHarness, id: &str) -> (Vec<TaskRecord>, Vec<RunRecord>) {
    let path = history_path(h);
    until("the run's history record", REQUEST_WAIT, || {
        let (tasks, runs) = history(&path);
        runs.iter().any(|r| r.run_id == id).then_some((tasks, runs))
    })
}

#[test]
fn e2e_history_records_the_fast_path_run() {
    let h = harness("claude", "", &[], &[]);
    h.decider("triage", 1, triage_single(&["a.txt"]));
    green_scripts(&h.repo);
    let out = h.start_goal("add a", &[]);
    assert!(out.status.success(), "{}\n{}", stderr(&out), h.log_tail());
    assert!(stderr(&out).contains("fast path"), "{}", stderr(&out));
    let id = stdout(&out).trim().to_string();
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(run.path, Some(RunPath::Fast));
    accept(&h, &id);

    let (tasks, runs) = wait_run_record(&h, &id);
    assert_eq!(tasks.len(), 1, "{tasks:?}");
    let task = &tasks[0];
    assert_eq!(
        (task.run_id.as_str(), task.task_id.as_str()),
        (id.as_str(), "t1")
    );
    assert_eq!(task.path, Some(RunPath::Fast));
    assert_eq!(task.outcome, TaskOutcome::Merged);
    assert_eq!(task.diff.as_ref().map(|d| d.files), Some(1), "{task:?}");
    assert_eq!(runs.len(), 1, "{runs:?}");
    assert_eq!(runs[0].outcome, "accepted");
    assert_eq!(runs[0].path, Some(RunPath::Fast));
    let main = h.git(&["rev-parse", "main"]);
    assert_eq!(runs[0].accepted_commit.as_deref(), Some(main.as_str()));
}

/// Waits, at most one `RUN_WAIT` (the run's task path up to its merge), until the
/// daemon's socket refuses connections: it aborted at the injected intent.
fn wait_dead(h: &RunHarness) {
    let deadline = Instant::now() + RUN_WAIT;
    while std::os::unix::net::UnixStream::connect(h.socket()).is_ok() {
        assert!(
            Instant::now() < deadline,
            "the daemon never reached its crash:\n{}",
            h.log_tail()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn e2e_history_survives_a_crash_after_the_append_intent() {
    let mut h = harness(
        "off",
        "",
        &[("ANTHREX_TEST_ABORT_AFTER_INTENT", "AppendHistory")],
        &[],
    );
    green_scripts(&h.repo);
    let path = history_path(&h);
    let id = h.start(&one_task_plan(&["a.txt"]), true);
    wait_dead(&h);
    h.forget_dead_daemon();
    // The daemon died between t1's intent and its line: no task record yet.
    assert!(history(&path).0.is_empty(), "a record was written first");

    h.unset_env("ANTHREX_TEST_ABORT_AFTER_INTENT");
    h.restart_daemon(&[]);
    if h.run(&id).expect("the run was restored").state == RunState::Paused {
        let out = h.anthrex(&["run", "resume", &id]);
        assert!(out.status.success(), "run resume: {}", stderr(&out));
    }
    // The part before the crash and the part after it are a path each (k = 2).
    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    accept(&h, &id);

    let (tasks, runs) = wait_run_record(&h, &id);
    let ids: Vec<&str> = tasks.iter().map(|t| t.record_id.as_str()).collect();
    assert_eq!(ids, vec![format!("{id}/t1").as_str()]);
    assert_eq!(runs.len(), 1, "{runs:?}");

    // The raw file: every line whole, at most one per record id.
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.ends_with('\n'), "a torn last line: {raw:?}");
    let mut seen = std::collections::BTreeMap::<String, u32>::new();
    for line in raw.lines() {
        let value: Value = serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"));
        let record = value["record_id"]
            .as_str()
            .expect("a record_id")
            .to_string();
        *seen.entry(record).or_default() += 1;
    }
    assert!(seen.values().all(|&n| n == 1), "{seen:?}");
    assert_eq!(seen.len(), 2, "{seen:?}");
}

#[test]
fn e2e_revert_after_accept_is_recorded_and_counted() {
    let h = harness("off", "", &[], &[]);
    green_scripts(&h.repo);
    let id = h.start(&one_task_plan(&["a.txt"]), true);
    h.wait_run(&id, complete, RUN_WAIT);
    accept(&h, &id);
    wait_run_record(&h, &id);
    assert_eq!(h.git(&["status", "--porcelain"]), "");

    h.git(&["revert", "-m", "1", "--no-edit", "HEAD"]);
    assert!(!h.repo.join("a.txt").exists());
    let repo = h.repo.display().to_string();
    let out = h.anthrex(&["run", "stats", "--json", "--dir", &repo]);
    assert!(out.status.success(), "{}", stderr(&out));
    let stats: HistoryStats = serde_json::from_str(&stdout(&out)).expect("HistoryStats");
    let row = stats
        .rows
        .iter()
        .find(|r| r.class == "S")
        .expect("an S row");
    assert_eq!((row.tasks, row.merged, row.reverted), (1, 1, 1), "{row:?}");
    let (lines, _) = read_history(&history_path(&h));
    let reverts = lines
        .iter()
        .filter(|l| matches!(l, HistoryLine::Revert(r) if r.run_id == id))
        .count();
    assert_eq!(reverts, 1);
}
