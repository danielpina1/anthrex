//! Task M9.11: `get_context`, `task_result` and the routing of the writes (decisions
//! 15, 17 and 18), through a real daemon socket with no agent (`orch_read_rig.rs`).

use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

use proto::AgentRole;
use serde_json::{Value, json};

use super::read_rig::{ORCH, PLANNER, Rig, WORKER};
use crate::run::driver::gated_git::GatedGit;
use crate::run::driver::*;
use crate::run::orch::test_support::{scout, task_mut};
use crate::run::orch::{EpicRecord, PlannerPhase, PlannerSession, RunScoutState};

/// A repository whose task branch `anthrex/<run>/t0` has one commit past its start.
fn repo_with_branch(root: &Path, run_id: &str) -> String {
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .arg("-C")
            .arg(root)
            .args(args)
            .env_remove("GIT_DIR")
            .env_remove("GIT_WORK_TREE")
            .env_remove("GIT_INDEX_FILE")
            .output()
            .unwrap();
        assert!(out.status.success(), "git {args:?}: {out:?}");
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    std::fs::create_dir_all(root).unwrap();
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.name", "Test"]);
    git(&["config", "user.email", "test@example.com"]);
    std::fs::write(root.join("a.txt"), "one\n").unwrap();
    git(&["add", "a.txt"]);
    git(&["commit", "-q", "-m", "base"]);
    let start = git(&["rev-parse", "HEAD"]);
    git(&[
        "checkout",
        "-q",
        "-b",
        &crate::run::model::task_branch(run_id, "t0"),
    ]);
    std::fs::write(root.join("a.txt"), "one\ntwo\n").unwrap();
    git(&["commit", "-q", "-am", "add the second line"]);
    git(&["checkout", "-q", "main"]);
    start
}

/// Decisions 17 and 18: `get_context` and `task_result` are the driver's, for the
/// orchestrator and (the context) for the live sub-planner of an epic; any other
/// window is refused, and so is a tool outside the role.
#[tokio::test(flavor = "multi_thread")]
async fn get_context_and_task_result_answer_from_the_driver() {
    use crate::scout::report::report_path;
    let rig = Rig::new(|run, dir| {
        let start = repo_with_branch(&run.root, &run.id);
        task_mut(run, "t0").start_commit = Some(start);
        // One stored run scout's report, and a planning epic with a live session.
        run.repo_dir = dir.join("repo-data");
        run.orch.run_scouts = vec![scout(
            "3f9a-api",
            RunScoutState::Reported,
            &["crates/m0/**"],
        )];
        run.scout_reports = vec!["3f9a-api".into()];
        let run_dir = run.data_dir.clone();
        let path = report_path(&run.repo_dir, Some(&run_dir), "3f9a-api");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let report = proto::ScoutReport {
            id: "3f9a-api".into(),
            kind: proto::ScoutKind::Area,
            run_id: Some(run.id.clone()),
            question: "Where?".into(),
            summary: "The API is one crate.".into(),
            files: vec![proto::ScoutFile {
                path: "crates/m0/src/lib.rs".into(),
                why: "the entry".into(),
            }],
            modules: Vec::new(),
            interfaces: Vec::new(),
            risks: Vec::new(),
            profile: None,
            route: crate::run::orch::test_support::route(),
            window_id: None,
            started_at: 0,
            finished_at: 0,
            tool_calls: 0,
            usage: Default::default(),
        };
        std::fs::write(&path, serde_json::to_vec(&report).unwrap()).unwrap();
        let mut epic = EpicRecord::new("mail", PlannerPhase::Planning);
        epic.area = vec!["crates/m0/**".into()];
        epic.sessions.push(PlannerSession {
            session: 1,
            window_id: Some(PLANNER),
            op: Some(3),
            started_at: 1_000,
            ended_at: None,
            usage: Default::default(),
            rejections: 0,
        });
        run.orch.epics.push(epic);
    })
    .await;

    let (ok, context) = rig.orch("get_context", json!({})).await;
    assert!(ok, "{context}");
    assert_eq!(context["run"]["id"], json!(rig.run_id));
    assert_eq!(context["you"]["role"], "orchestrator");
    assert_eq!(context["you"]["epic"], Value::Null);
    assert_eq!(context["scouts"][0]["id"], "3f9a-api", "{context}");
    assert_eq!(context["scouts"][0]["summary"], "The API is one crate.");
    assert_eq!(context["plan"].as_array().unwrap().len(), 2);

    let planner = rig.opts(AgentRole::Planner, PLANNER, Some("mail"));
    let (ok, context) = rig.call(planner.clone(), "get_context", json!({})).await;
    assert!(ok, "{context}");
    assert_eq!(context["you"]["role"], "planner");
    assert_eq!(context["you"]["epic"]["epic"], "mail");
    assert_eq!(
        context["scouts"][0]["id"], "3f9a-api",
        "its area meets the report's"
    );

    let (ok, result) = rig.orch("task_result", json!({"task_id": "t0"})).await;
    assert!(ok, "{result}");
    assert_eq!(result["task"]["id"], "t0");
    assert_eq!(
        result["commits"][0]["subject"], "add the second line",
        "{result}"
    );
    assert!(
        result["diffstat"]
            .as_str()
            .unwrap()
            .contains("1 file changed"),
        "{result}"
    );
    let (ok, result) = rig.orch("task_result", json!({"task_id": "t1"})).await;
    assert!(ok, "{result}");
    assert!(result.get("commits").is_none() && result.get("git").is_none());
    let (ok, result) = rig.orch("task_result", json!({"task_id": "t9"})).await;
    assert!(!ok);
    assert_eq!(result, json!({"error": "unknown task t9"}));

    // The callers decision 15 refuses.
    let not_orch = format!("this window is not the orchestrator of run {}", rig.run_id);
    let wrong = rig.opts(AgentRole::Orchestrator, WORKER, None);
    for tool in ["get_context", "run_status", "task_result"] {
        let (ok, answer) = rig
            .call(wrong.clone(), tool, json!({"task_id": "t0"}))
            .await;
        assert!(!ok);
        assert_eq!(answer, json!({"error": not_orch}), "{tool}");
    }
    let stale = rig.opts(AgentRole::Planner, ORCH, Some("mail"));
    let (ok, answer) = rig.call(stale, "get_context", json!({})).await;
    assert!(!ok);
    let text = format!(
        "this window is not the sub-planner of epic mail of run {}",
        rig.run_id
    );
    assert_eq!(answer, json!({"error": text}));
    let other = rig.opts(AgentRole::Planner, PLANNER, Some("web"));
    let (_, answer) = rig.call(other, "get_context", json!({})).await;
    assert_eq!(answer, json!({"error": "unknown epic web"}));
    let (ok, answer) = rig
        .call(planner, "task_result", json!({"task_id": "t0"}))
        .await;
    assert!(!ok);
    assert_eq!(
        answer,
        json!({"error": "tool task_result is not available to the planner role"})
    );
    // Untrusted arguments are checked against the schema's bounds.
    let (ok, answer) = rig
        .orch("run_status", json!({"since": 1, "wait_secs": 51}))
        .await;
    assert!(!ok);
    assert_eq!(
        answer,
        json!({"error": "invalid arguments: wait_secs: must be 0 to 50"})
    );
    let (_, answer) = rig.orch("get_context", json!({"extra": true})).await;
    assert_eq!(
        answer,
        json!({"error": "invalid arguments: extra: unknown field"})
    );
}

/// Decision 15's routing of the writes: the orchestrator's `edit_plan` and a worker's
/// `task_note` reach the engine as `OrchEvent::Tool` (before this task every one went
/// to M8a's worker gate, which knows neither); an edit that would approve does not
/// parse.
#[tokio::test(flavor = "multi_thread")]
async fn writes_go_to_the_engine() {
    let rig = Rig::new(|_, _| {}).await;
    let (ok, answer) = rig.orch("edit_plan", json!({"edits": []})).await;
    assert!(ok, "{answer}");
    assert_eq!(answer["accepted"], json!(true), "{answer}");
    let (ok, answer) = rig
        .orch(
            "edit_plan",
            json!({"edits": [{"op": "override", "task_id": "t1"}]}),
        )
        .await;
    assert!(!ok);
    let text = answer["error"].as_str().unwrap_or_default();
    assert!(
        text.starts_with("invalid arguments: edits[0]: unknown variant `override`"),
        "{answer}"
    );
    let wrong = rig.opts(AgentRole::Orchestrator, WORKER, None);
    let (ok, answer) = rig.call(wrong, "edit_plan", json!({"edits": []})).await;
    assert!(!ok);
    let text = format!("this window is not the orchestrator of run {}", rig.run_id);
    assert_eq!(answer, json!({"error": text}));
    let mut worker = rig.opts(AgentRole::Worker, WORKER, None);
    worker.task_id = Some("t0".into());
    let (ok, answer) = rig
        .call(
            worker,
            "task_note",
            json!({"kind": "risk", "text": "flaky"}),
        )
        .await;
    // Task M9.13a: the engine records it (decision 42f).
    assert!(ok, "{answer}");
    assert_eq!(answer, json!("Note recorded. Keep working."));
}

/// The held case's deadline for `task_result`'s git reads. Nothing else can end them
/// (each call's own bound is an hour, and the stand-in holds the second until the test
/// releases it), so this length is only the test's running time.
const DEADLINE: Duration = Duration::from_secs(1);

/// M9.6 review M-4: `task_result`'s two git reads share one deadline; past it the
/// answer carries the git error, whatever the commands had left. Separate per-command
/// bounds alone would answer only once the held call ended, with no error.
#[tokio::test(flavor = "multi_thread")]
async fn task_result_answers_a_git_error_at_its_one_deadline() {
    let never = Duration::from_secs(3600);
    let wait = Duration::from_secs(30);
    let gate_dir = tempfile::tempdir().unwrap();
    // The first read (`git log`) answers an empty line; the second (`git diff`) is held.
    let gate = GatedGit::new(gate_dir.path(), "");
    let program = gate.program();
    let rig = Rig::with(
        |run, _| {
            run.limits.git_timeout_secs = never.as_secs();
            task_mut(run, "t0").start_commit = Some("a1b2c3d".into());
        },
        |_, ctx| {
            ctx.git = program;
            ctx.read_git = GitBudget {
                deadline: DEADLINE,
                each_cap: never,
            };
        },
    )
    .await;
    let started = Instant::now();
    let (ok, result) =
        tokio::time::timeout(wait, rig.orch("task_result", json!({"task_id": "t0"})))
            .await
            .expect("only the reads' deadline can end them");
    let took = started.elapsed();
    assert!(ok, "{result}");
    let text = format!("git did not answer within {} s", DEADLINE.as_secs());
    assert_eq!(result["git"], text, "{result}");
    assert!(result.get("commits").is_none(), "{result}");
    assert!(took >= DEADLINE, "{took:?}");
    // The read the deadline abandoned goes on: its first call answers and its second
    // is held until the release. (When the deadline strikes relative to the calls is
    // not asserted: under load the first spawn alone can outlast it.)
    gate.wait_held(wait).await;
    let calls = gate.calls();
    assert_eq!(calls.len(), 2, "{calls:?}");
    assert!(calls[0].contains(" log "), "{calls:?}");
    assert!(calls[1].contains(" diff "), "{calls:?}");
    gate.release();
    gate.wait_exited(wait).await;
}
