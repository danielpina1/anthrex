//! Task M9.11, review findings 1 and 2: calls that come before the launch they belong
//! to has named its window. The engine learns a sub-planner's window from its
//! `StartPlanner` result and the orchestrator's from its `CreateOrchestrator` result;
//! the session can call before either arrives. Through a real daemon socket with no
//! agent (`orch_read_rig.rs`); each launch's result is sent to the engine by the test.

use std::path::Path;
use std::process::Command;
use std::time::Duration;

use proto::AgentRole;
use serde_json::{Value, json};

use super::read_rig::{ANSWER, ORCH, PLANNER, Rig};
use crate::run::driver::*;
use crate::run::engine::{OpKind, OpResult};
use crate::run::model::{PendingOp, Run};
use crate::run::orch::launch::{orchestrator_role, orchestrator_window_spec, planner_spec};
use crate::run::orch::{EpicRecord, PlannerPhase, PlannerSession};

/// The pending ops of the launches.
const PLANNER_OP: u64 = 3;
const ORCH_OP: u64 = 4;

/// `mail`'s sub-planner, planning, its first session's `StartPlanner` in flight.
fn planner_launching(run: &mut Run) {
    let mut epic = EpicRecord::new("mail", PlannerPhase::Planning);
    epic.sessions.push(PlannerSession {
        session: 1,
        window_id: None,
        op: Some(PLANNER_OP),
        started_at: 1_000,
        ended_at: None,
        usage: Default::default(),
        rejections: 0,
    });
    let spec = planner_spec(run, &epic, 1);
    run.orch.epics.push(epic);
    let kind = OpKind::StartPlanner {
        spec: Box::new(spec),
    };
    pending(run, PLANNER_OP, kind);
}

/// The run's orchestrator with its `CreateOrchestrator` in flight and no window.
fn orchestrator_launching(run: &mut Run) {
    let record = run.orch.orchestrator.as_mut().unwrap();
    (record.window_id, record.launch_op) = (None, Some(ORCH_OP));
    let route = record.route.clone();
    let kind = OpKind::CreateOrchestrator {
        spec: Box::new(orchestrator_window_spec(run, &route, "plan")),
        role: Box::new(orchestrator_role(run, &route)),
        project: run.project.clone(),
    };
    pending(run, ORCH_OP, kind);
}

fn pending(run: &mut Run, op: u64, kind: OpKind) {
    let entry = PendingOp {
        op,
        task_id: None,
        kind,
    };
    run.pending_ops.insert(op, entry);
}

/// The launch's result, as the driver sends it once the op's `done` line is synced.
fn done(rig: &Rig, op: u64, result: OpResult) {
    rig.runs.send(EventKind::OpDone {
        run_id: rig.run_id.clone(),
        op,
        result,
    });
}

/// Starts `tool` from `opts` in the background.
fn spawn_call(
    rig: &Arc<Rig>,
    opts: mcp::McpOptions,
    tool: &'static str,
    args: Value,
) -> tokio::task::JoinHandle<(bool, Value)> {
    let rig = rig.clone();
    tokio::spawn(async move { rig.call(opts, tool, args).await })
}

async fn answered(call: tokio::task::JoinHandle<(bool, Value)>) -> (bool, Value) {
    tokio::time::timeout(ANSWER, call)
        .await
        .expect("answered once the launch resolved")
        .unwrap()
}

/// Finding 1: a sub-planner's first `get_context`, made before its `StartPlanner`
/// result, waits for the launch and is answered once the window is bound; when the
/// launch fails it is refused as any other window's call.
#[tokio::test(flavor = "multi_thread")]
async fn a_planners_first_read_waits_for_its_launch() {
    for bound in [true, false] {
        let rig = Arc::new(Rig::new(|run, _| planner_launching(run)).await);
        let opts = rig.opts(AgentRole::Planner, PLANNER, Some("mail"));
        let call = spawn_call(&rig, opts, "get_context", json!({}));
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert!(!call.is_finished(), "the read waits for the launch");
        let result = if bound {
            OpResult::PlannerStarted { window_id: PLANNER }
        } else {
            OpResult::Failed {
                message: "no binary".into(),
            }
        };
        done(&rig, PLANNER_OP, result);
        let (ok, answer) = answered(call).await;
        if bound {
            assert!(ok, "{answer}");
            assert_eq!(answer["you"]["epic"]["epic"], "mail", "{answer}");
        } else {
            assert!(!ok);
            let text = format!(
                "this window is not the sub-planner of epic mail of run {}",
                rig.run_id
            );
            assert_eq!(answer, json!({ "error": text }));
        }
    }
}

/// Finding 1: the orchestrator's first calls, a read and a write, made before its
/// `CreateOrchestrator` result, wait for it and are answered once it is bound.
#[tokio::test(flavor = "multi_thread")]
async fn an_orchestrators_first_calls_wait_for_its_launch() {
    let rig = Arc::new(Rig::new(|run, _| orchestrator_launching(run)).await);
    let opts = rig.opts(AgentRole::Orchestrator, ORCH, None);
    let read = spawn_call(&rig, opts.clone(), "run_status", json!({}));
    let write = spawn_call(&rig, opts, "edit_plan", json!({"edits": []}));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!read.is_finished() && !write.is_finished(), "both wait");
    let bound = OpResult::Window {
        window_id: ORCH,
        pid: None,
    };
    done(&rig, ORCH_OP, bound);
    let (ok, digest) = answered(read).await;
    assert!(ok, "{digest}");
    assert_eq!(digest["run"]["id"], json!(rig.run_id));
    let (ok, answer) = answered(write).await;
    assert!(ok, "{answer}");
    assert_eq!(answer["accepted"], json!(true));
}

/// A repository whose one commit tracks `.mcp.json`: project settings a headless
/// Claude session would load unasked (decision 53). Its commit.
fn repo_with_project_settings(root: &Path) -> String {
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
    std::fs::write(root.join(".mcp.json"), "{}\n").unwrap();
    git(&["add", ".mcp.json"]);
    git(&["commit", "-q", "-m", "base"]);
    git(&["rev-parse", "HEAD"])
}

/// Finding 2 (ruling T22-I1b): a sub-planner's `submit_epic` made before its window is
/// bound is held by the engine, and its runtime refusals are computed before it is
/// sent, so the replay refuses a batch that reaches a runtime whose checks fail. The
/// run reaches Codex only (no task; the orchestrator, its planners and its scouts on
/// Codex); the batch adds a Claude task, and Claude sessions would load the base's
/// `.mcp.json`.
#[tokio::test(flavor = "multi_thread")]
async fn a_held_submit_carries_its_runtime_refusals() {
    let rig = Arc::new(
        Rig::with(
            |run, _| {
                run.tasks.clear();
                run.state = proto::RunState::Planning;
                run.base_sha = repo_with_project_settings(&run.root);
                run.trusted_project.clear();
                let record = run.orch.orchestrator.as_mut().unwrap();
                record.route.runtime = proto::Runtime::Codex;
                record.route.model = String::new();
                run.limits.orch.planners.runtime = None;
                // Codex has no frontier model; at standard its planners stay on Codex.
                run.limits.orch.planners.strength = proto::Strength::Standard;
                // Whole-branch review, item 1: its scouts count too; on Codex.
                let scouts = run.limits.orch.scouts.as_mut().unwrap();
                scouts.runtime = Some(proto::Runtime::Codex);
                planner_launching(run);
                let reached = crate::run::reach::reachable_runtimes(run);
                assert_eq!(reached, [proto::Runtime::Codex]);
            },
            |_, ctx| ctx.cli_caps.claude_user_settings_only = None,
        )
        .await,
    );
    let task = json!({
        "id": "m1", "title": "Mail", "size": "S", "owns": ["crates/mail/src/**"],
        "brief": "Send mail.", "acceptance": ["mail is sent"],
        "route": {"runtime": "claude"},
    });
    let args = json!({"edits": [{"op": "add_task", "task": task}]});
    let opts = rig.opts(AgentRole::Planner, PLANNER, Some("mail"));
    let call = spawn_call(&rig, opts, "submit_epic", args);
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(!call.is_finished(), "held by the engine");
    done(
        &rig,
        PLANNER_OP,
        OpResult::PlannerStarted { window_id: PLANNER },
    );
    let (ok, answer) = answered(call).await;
    assert!(!ok, "the batch reaches Claude: {answer}");
    assert!(answer.to_string().contains(".mcp.json"), "{answer}");
    assert!(rig.run(|run| run.task("m1").is_none()));
}

/// The launch wait's limit in the re-review tests: `HOLD_LIMIT_SECS` shortened.
const SHORT_LIMIT: Duration = Duration::from_millis(300);

/// `tool` from `window` in `role`, as `anthrex mcp` sends it.
fn tool_call(rig: &Rig, role: AgentRole, window: u32, tool: &str, args: Value) -> proto::ToolCall {
    proto::ToolCall {
        run_id: rig.run_id.clone(),
        task_id: None,
        role,
        window_id: window,
        tool: tool.into(),
        args,
        scout_id: None,
        epic: (role == AgentRole::Planner).then(|| "mail".to_string()),
        chain: None,
    }
}

/// The text of a `ToolResult`.
fn reply_text(reply: proto::RunReply) -> (bool, Value) {
    let proto::RunReply::ToolResult { ok, text, .. } = reply else {
        panic!("not a tool result: {reply:?}");
    };
    (
        ok,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

/// Re-review finding 2: a launch that never resolves ends the wait at its limit (the
/// production limit is `HOLD_LIMIT_SECS`), and the caller is then checked as usual.
#[tokio::test(flavor = "multi_thread")]
async fn a_launch_wait_ends_at_its_limit() {
    let rig = Rig::new(|run, _| {
        orchestrator_launching(run);
        planner_launching(run);
    })
    .await;
    for (role, window, text) in [
        (
            AgentRole::Orchestrator,
            ORCH,
            format!("this window is not the orchestrator of run {}", rig.run_id),
        ),
        (
            AgentRole::Planner,
            PLANNER,
            format!(
                "this window is not the sub-planner of epic mail of run {}",
                rig.run_id
            ),
        ),
    ] {
        let call = tool_call(&rig, role, window, "get_context", json!({}));
        let reply = tokio::time::timeout(ANSWER, rig.runs.orch_tool_within(call, SHORT_LIMIT))
            .await
            .expect("the wait ends at its limit");
        assert_eq!(reply_text(reply), (false, json!({ "error": text })));
    }
}

/// Re-review finding 5: an orchestrator write whose wait ends with its launch still in
/// flight is refused in the driver, never sent to the engine without its runtime
/// refusals (a `Window` result reduced just before it would let it through).
#[tokio::test(flavor = "multi_thread")]
async fn an_orchestrator_write_whose_launch_never_finishes_is_refused() {
    let rig = Rig::new(|run, _| orchestrator_launching(run)).await;
    let edits = json!({"edits": [{"op": "cancel_task", "task_id": "t1"}]});
    let call = tool_call(&rig, AgentRole::Orchestrator, ORCH, "edit_plan", edits);
    let reply = tokio::time::timeout(ANSWER, rig.runs.orch_tool_within(call, SHORT_LIMIT))
        .await
        .expect("the wait ends at its limit");
    let text = super::ORCHESTRATOR_LAUNCH_PENDING;
    assert_eq!(reply_text(reply), (false, json!({ "error": text })));
    assert!(
        rig.run(|run| run.plan_edits.is_empty()),
        "the engine saw no batch"
    );
}
