//! Shared run fixtures for milestone 8c's tests (tree, graph, inspector, app, ui).
//!
//! Built once here so two test files never disagree about the same run (Risks 8).
//! `run` and `task` deserialize a minimal JSON object, so a test lists only the fields
//! it cares about and every other field takes a neutral value.

use proto::{
    AgentRole, AgentRoundInfo, PlannerInfo, ReviewInfo, RunInfo, RunRef, RunState, RunsSnapshot,
    Runtime, ScoutInfo, Size, Status, TaskInfo, TaskState, Verdict, WindowInfo, WindowKind,
};

/// The gate and three-task fixtures' run id, project and goal.
pub(crate) const RUN_ID: &str = "add-reset-3f9a";
pub(crate) const PROJECT: &str = "/r/demo";
pub(crate) const GOAL: &str = "Add password reset";

fn route_json(runtime: Runtime) -> serde_json::Value {
    serde_json::json!({
        "runtime": runtime, "model": "", "strength": "standard", "effort": "medium",
    })
}

/// A run with no tasks, created at 0, in `state`.
pub(crate) fn run(id: &str, project: &str, state: RunState) -> RunInfo {
    serde_json::from_value(serde_json::json!({
        "run_id": id, "goal": GOAL, "project": project, "root": project, "state": state,
        "paused_from": null, "halted_reason": null, "approved_by": null,
        "base_branch": "main", "base_sha": "", "run_branch": "", "run_head": "",
        "base_moved": null, "revision": 1, "max_writers": 3, "max_readers": 3,
        "max_bounces": 2, "writers_busy": 0, "readers_busy": 0, "unverified": false,
        "worker_sandbox": true, "trusted_project": [], "rate_limits": {}, "tasks": [],
        "critical_path": [], "attention": [], "report_path": "/tmp/report.md",
        "outcome": null, "created_at": 0,
    }))
    .expect("a minimal RunInfo")
}

/// A Claude-routed code task, wave 0, no deps and no rounds.
pub(crate) fn task(id: &str, title: &str, size: Size, state: TaskState) -> TaskInfo {
    let spend = serde_json::json!({ "tool_calls": 0, "secs": 0, "tokens": 0 });
    let mut value = serde_json::json!({
        "id": id, "title": title, "epic": null, "kind": "code", "size": size, "hub": false,
        "test_mode": "tdd", "test_mode_reason": null, "notes": [], "owns": [], "deps": [],
        "implicit_deps": [], "priority": 0, "route": route_json(Runtime::Claude),
        "review_route": null, "budget": { "tool_calls": 100, "minutes": 30 },
        "spent_session": spend, "spent_total": spend, "state": state, "block": null,
        "rung": 0, "failures": 0,
    });
    let rest = serde_json::json!({
        "bounces": { "done": 0, "proof": 0, "check": 0, "review": 0, "merge": 0 },
        "stalls": 0, "budget_exceeded": 0, "conflicts": 0, "branch": "", "worktree": "/tmp",
        "start_commit": null, "head": null, "test": null, "red": null, "done_signal": null,
        "rounds": [], "reviews": [], "last_check": null, "last_proof": null,
        "merge_commit": null, "merged_without_approval": null, "salvage_refs": [],
        "on_critical_path": false, "wave": 0,
    });
    if let (Some(value), serde_json::Value::Object(rest)) = (value.as_object_mut(), rest) {
        value.extend(rest);
    }
    serde_json::from_value(value).expect("a minimal TaskInfo")
}

fn round(
    role: AgentRole,
    session: u32,
    window: Option<u32>,
    runtime: Runtime,
    started: u64,
) -> AgentRoundInfo {
    serde_json::from_value(serde_json::json!({
        "role": role, "session": session, "round": session, "window_id": window,
        "route": route_json(runtime), "session_id": null, "started_at": started,
        "ended_at": null, "tool_calls": 0, "last_event": started, "turn_open": false,
        "turns": 0, "rate_limited": false, "open_subagents": 0, "denials": 0,
        "usage": { "input": 0, "output": 0, "cache_read": 0, "cache_write": 0 },
    }))
    .expect("a minimal AgentRoundInfo")
}

/// A live worker session: its `round` equals its `session` (`engine/dispatch.rs:367`).
pub(crate) fn worker(
    session: u32,
    window: Option<u32>,
    runtime: Runtime,
    started: u64,
) -> AgentRoundInfo {
    round(AgentRole::Worker, session, window, runtime, started)
}

/// A live review round: its `round` and `session` are the review round.
pub(crate) fn reviewer(
    round_number: u32,
    window: Option<u32>,
    runtime: Runtime,
    started: u64,
) -> AgentRoundInfo {
    round(AgentRole::Reviewer, round_number, window, runtime, started)
}

/// An area scout, `working`, with no window.
pub(crate) fn scout(id: &str, question: &str, runtime: Runtime, started: u64) -> ScoutInfo {
    serde_json::from_value(serde_json::json!({
        "id": id, "kind": "area", "question": question, "state": "working", "failure": null,
        "window_id": null, "route": route_json(runtime), "started_at": started,
        "ended_at": null, "tool_calls": 0, "report_bytes": null, "files": [],
        "usage": { "input": 0, "output": 0, "cache_read": 0, "cache_write": 0 },
    }))
    .expect("a minimal ScoutInfo")
}

/// A sub-planner of `epic`, `planning`, Claude-routed, with no window.
pub(crate) fn planner(epic: &str, title: &str) -> PlannerInfo {
    PlannerInfo {
        epic: epic.into(),
        title: title.into(),
        area: vec![],
        route: serde_json::from_value(route_json(Runtime::Claude)).expect("a Route"),
        window_id: None,
        state: proto::PlannerState::Planning,
        started_at: 0,
        ended_at: None,
        edits_accepted: 0,
        edits_rejected: 0,
        last_rejection: None,
        replans: vec![],
    }
}

/// A plain PTY shell window.
pub(crate) fn pty(id: u32, name: &str, project: &str, status: Status) -> WindowInfo {
    WindowInfo {
        id,
        name: name.into(),
        runtime: Runtime::Shell,
        cwd: project.into(),
        project: project.into(),
        worktree: None,
        branch: None,
        status,
        tool: None,
        since_secs: 0,
        last_output_secs: 0,
        session_id: None,
        model: None,
        subagents: vec![],
        exit: None,
        kind: WindowKind::Pty,
        run: None,
    }
}

/// A headless Claude window, `Idle`, with `run_ref` as its `WindowInfo.run`.
pub(crate) fn headless(id: u32, name: &str, project: &str, run_ref: Option<RunRef>) -> WindowInfo {
    WindowInfo {
        runtime: Runtime::Claude,
        kind: WindowKind::Headless,
        run: run_ref,
        ..pty(id, name, project, Status::Idle)
    }
}

pub(crate) fn run_ref(run_id: &str, task: Option<&str>, role: AgentRole, session: u32) -> RunRef {
    RunRef {
        run_id: run_id.into(),
        task_id: task.map(str::to_owned),
        role,
        session,
    }
}

pub(crate) fn snapshot(now: u64, runs: Vec<RunInfo>) -> RunsSnapshot {
    RunsSnapshot {
        revision: 1,
        runs,
        now,
    }
}

/// The gate fixture: project `/r/demo` with one plain PTY shell window `1` `shell`,
/// `Idle`, and run `add-reset-3f9a` in `awaiting_approval` with tasks `t1`, `t2` and
/// no windows.
pub(crate) fn gate_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    let mut gate = run(RUN_ID, PROJECT, RunState::AwaitingApproval);
    gate.tasks = vec![
        task("t1", "reset token model", Size::M, TaskState::Pending),
        task("t2", "reset endpoint", Size::S, TaskState::Pending),
    ];
    (
        snapshot(10_000, vec![gate]),
        vec![pty(1, "shell", PROJECT, Status::Idle)],
    )
}

/// The three-task fixture: run `add-reset-3f9a`, `running`, orchestrator window `3`
/// listed; `t0 proto` M hub `merged`, wave 0, a finished Claude worker round (window 4,
/// retired) and a finished Codex reviewer round 1 that approved (window 5, retired);
/// `t1 spawn` M, deps `[t0]`, `working`, wave 1, on the critical path, a live Claude
/// worker round on window 6 (`Headless`, `Idle`); `t2 status` S, deps `[t0]`, `queued`,
/// wave 1. The plain shell window `1` of the gate fixture is listed too.
pub(crate) fn three_task_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    let now = 10_000;
    let mut three = run(RUN_ID, PROJECT, RunState::Running);
    three.created_at = now - 600;
    three.critical_path = vec!["t0".into(), "t1".into()];

    let mut t0 = task("t0", "proto", Size::M, TaskState::Merged);
    t0.hub = true;
    let mut t0_worker = worker(1, Some(4), Runtime::Claude, now - 500);
    t0_worker.ended_at = Some(now - 400);
    let mut t0_review = reviewer(1, Some(5), Runtime::Codex, now - 390);
    t0_review.ended_at = Some(now - 350);
    t0.rounds = vec![t0_worker, t0_review];
    t0.reviews = vec![ReviewInfo {
        round: 1,
        route: t0.rounds[1].route.clone(),
        verdict: Some(Verdict::Approve),
        summary: "looks right".into(),
        findings: vec![],
        blocking: false,
    }];

    let mut t1 = task("t1", "spawn", Size::M, TaskState::Working);
    t1.deps = vec!["t0".into()];
    t1.wave = 1;
    t1.on_critical_path = true;
    t1.rounds = vec![worker(1, Some(6), Runtime::Claude, now - 300)];

    let mut t2 = task("t2", "status", Size::S, TaskState::Queued);
    t2.deps = vec!["t0".into()];
    t2.wave = 1;

    three.tasks = vec![t0, t1, t2];

    let mut orchestrator = pty(3, "orchestrator", PROJECT, Status::Idle);
    orchestrator.runtime = Runtime::Claude;
    orchestrator.run = Some(run_ref(RUN_ID, None, AgentRole::Orchestrator, 1));
    let worker_window = headless(
        6,
        "3f9a/t1.w1",
        PROJECT,
        Some(run_ref(RUN_ID, Some("t1"), AgentRole::Worker, 1)),
    );
    (
        snapshot(now, vec![three]),
        vec![
            pty(1, "shell", PROJECT, Status::Idle),
            orchestrator,
            worker_window,
        ],
    )
}
