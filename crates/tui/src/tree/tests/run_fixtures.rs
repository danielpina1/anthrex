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

pub(crate) fn round(
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
        note: None,
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
        signals_seen: false,
        placeholder: false,
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
        lane: None,
    }
}

pub(crate) fn snapshot(now: u64, runs: Vec<RunInfo>) -> RunsSnapshot {
    RunsSnapshot {
        revision: 1,
        runs,
        now,
        proposals: Vec::new(),
        idle_orchestrators: Vec::new(),
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
        lane: None,
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

/// A `running` run of 200 tasks `t0`…`t199`, each `review`, S, titled `work`, with a
/// Claude worker and two Codex reviewer rounds (the first finished), listed out of
/// start order: 801 rows and 600 leaves (review focus 2).
pub(crate) fn two_hundred_task_run() -> RunInfo {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    info.tasks = (0..200u64)
        .map(|index| {
            let id = format!("t{index}");
            let mut info = task(&id, "work", Size::S, TaskState::Review);
            let start = 1_000 + index * 10;
            let mut first = reviewer(1, None, Runtime::Codex, start + 1);
            first.ended_at = Some(start + 2);
            info.rounds = vec![
                reviewer(2, None, Runtime::Codex, start + 3),
                worker(1, None, Runtime::Claude, start),
                first,
            ];
            info
        })
        .collect();
    info
}

/// The Gemini fixture's `D`: a whole number of days in unix seconds, so `D + 39_720` is
/// 11:02 UTC.
pub(crate) const GEMINI_DAY: u64 = 20_000 * 86_400;
/// The Gemini fixture's `now` (12:40 UTC).
pub(crate) const GEMINI_NOW: u64 = GEMINI_DAY + 45_600;

fn usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> proto::TokenUsage {
    proto::TokenUsage {
        input,
        output,
        cache_read,
        cache_write,
    }
}

fn routed(runtime: Runtime, strength: proto::Strength, effort: proto::Effort) -> proto::Route {
    proto::Route {
        runtime,
        model: String::new(),
        strength,
        effort,
    }
}

fn finding(severity: proto::Severity, file: &str, line: u32, text: &str) -> proto::Finding {
    proto::Finding {
        severity,
        file: Some(file.into()),
        line: Some(line),
        input: None,
        text: text.into(),
    }
}

fn gemini_task(id: &str, state: TaskState, tool_calls: u32) -> TaskInfo {
    let mut info = task(id, &format!("{id} work"), Size::S, state);
    info.spent_total.tool_calls = tool_calls;
    info
}

/// Task `t2` of the Gemini fixture (Interfaces "Inspector contents, exact", the task
/// and agent-round mockups).
fn gemini_t2(now: u64) -> TaskInfo {
    use proto::{Effort, Severity, Strength};
    let mut t2 = gemini_task("t2", TaskState::Review, 104);
    t2.title = "map Gemini hook events to status".into();
    t2.size = Size::M;
    t2.deps = vec!["t0".into(), "t6".into()];
    t2.on_critical_path = true;
    t2.wave = 1;
    t2.route = routed(Runtime::Codex, Strength::Standard, Effort::High);
    let mut review_route = routed(Runtime::Claude, Strength::Frontier, Effort::High);
    review_route.model = "claude-opus-5".into();
    t2.review_route = Some(review_route.clone());
    t2.budget = proto::Budget {
        tool_calls: 150,
        minutes: 60,
        tokens: None,
    };
    t2.spent_session = proto::Spend {
        tool_calls: 104,
        secs: 2280,
        tokens: 410_000,
    };
    t2.bounces.review = 1;
    t2.rung = 1;
    t2.branch = "anthrex/r1/t2".into();
    t2.diff = Some(proto::DiffStats {
        files: 4,
        hunks: 9,
        added: 212,
        removed: 31,
    });
    t2.test = Some("status::gemini_stop_marks_idle".into());
    t2.red = Some("a1b2c3d9e8f7".into());
    t2.done_signal = Some(proto::DoneSignal::TaskDone);
    t2.last_proof = Some(proto::ProofInfo {
        at: now - 1500,
        test: "status::gemini_stop_marks_idle".into(),
        red: "a1b2c3d9e8f7".into(),
        red_failed: true,
        head_passed: true,
        matched: true,
        ok: true,
    });
    t2.last_check = Some(proto::CheckInfo {
        at: now - 1000,
        ok: true,
        code: Some(0),
        timed_out: false,
        secs: 40,
        summary: "test result: ok".into(),
        on_candidate: false,
        decider_summary: None,
        summary_source: None,
        tier: None,
    });
    let mut work = worker(1, Some(7), Runtime::Codex, now - 1560);
    work.route = t2.route.clone();
    work.sent_back_at = vec![now - 360];
    work.turns = 14;
    work.tool_calls = 41;
    work.usage = usage(100_000, 80_000, 500_000, 0);
    let mut first = reviewer(1, Some(9), Runtime::Claude, now - 900);
    first.route = review_route.clone();
    first.ended_at = Some(now - 360);
    let mut second = reviewer(2, None, Runtime::Claude, now - 60);
    second.route = review_route.clone();
    t2.rounds = vec![work, first, second];
    t2.reviews = vec![
        ReviewInfo {
            round: 1,
            route: review_route.clone(),
            verdict: Some(Verdict::Changes),
            summary: "one pairing bug".into(),
            findings: vec![
                finding(Severity::Minor, "crates/daemon/src/hooks.rs", 12, "naming"),
                finding(
                    Severity::Critical,
                    "crates/daemon/src/status.rs",
                    118,
                    "SubagentStop not paired",
                ),
                finding(Severity::Minor, "crates/daemon/src/hooks.rs", 40, "comment"),
            ],
            blocking: true,
            lane: None,
        },
        ReviewInfo {
            round: 2,
            route: review_route,
            verdict: None,
            summary: String::new(),
            findings: vec![],
            blocking: false,
            lane: None,
        },
    ];
    t2.history = [(45_060, "review r1 changes"), (44_400, "check passed")]
        .into_iter()
        .chain([(43_320, "started")])
        .map(|(at, text)| proto::TaskEventInfo {
            at: GEMINI_DAY + at,
            text: text.into(),
        })
        .collect();
    t2
}

/// The Gemini fixture (Interfaces "Inspector contents, exact"): run `r1`, goal `Add
/// Gemini runtime`, created 1h12m ago; `t0 t1 t4 t6 t8` merged, `t3 t7` working, `t2`
/// in review, `t5` blocked on a question; critical path `t0 t6 t2 t3`; writers 3/3,
/// readers 1/3; `t7`'s live Codex worker rate-limited since `now − 240` until
/// `now + 60`; the run's token usage and 612 tool calls; the M9.5 estimates; approved
/// at 11:02 UTC with two plan edits since. Scout `S1` reported after 180 s (window 4
/// retired). Windows: 7 (`t2`'s worker, `Working`, `apply_patch`), 10 (`t3`'s worker).
pub(crate) fn gemini_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    use proto::{BlockInfo, BlockReason, PlanEditInfo, RunUsage};
    let now = GEMINI_NOW;
    let mut r1 = run("r1", "/r/anthrex", RunState::Running);
    r1.goal = "Add Gemini runtime".into();
    r1.created_at = now - 4320;
    r1.approved_by = Some("user".into());
    r1.approved_at = Some(GEMINI_DAY + 39_720);
    r1.plan_edits = vec![
        PlanEditInfo {
            at: GEMINI_DAY + 42_000,
            text: "split t2".into(),
            source: String::new(),
            accepted: true,
            error: None,
            recipients: Vec::new(),
        },
        PlanEditInfo {
            at: GEMINI_DAY + 40_800,
            text: "amend t4".into(),
            source: String::new(),
            accepted: true,
            error: None,
            recipients: Vec::new(),
        },
    ];
    r1.plan_edits_since_approval = 2;
    r1.writers_busy = 3;
    r1.readers_busy = 1;
    r1.estimate_left_secs = Some(2400);
    r1.bound_ratio_permille = Some(1200);
    r1.critical_path = ["t0", "t6", "t2", "t3"].map(String::from).to_vec();
    r1.usage = Some(RunUsage {
        total: usage(200_000, 1_510_000, 710_000, 90_000),
        ..RunUsage::default()
    });
    r1.attention = vec!["t5 blocked (question): Gemini has no subagent-stop event".into()];

    let merged = |id: &str, calls| gemini_task(id, TaskState::Merged, calls);
    let mut t0 = merged("t0", 100);
    t0.on_critical_path = true;
    let mut t6 = merged("t6", 80);
    t6.on_critical_path = true;
    let mut t3 = gemini_task("t3", TaskState::Working, 60);
    t3.deps = vec!["t2".into()];
    t3.on_critical_path = true;
    t3.rounds = vec![worker(1, Some(10), Runtime::Claude, now - 600)];
    let mut t5 = gemini_task("t5", TaskState::Blocked, 40);
    t5.block = Some(BlockInfo {
        reason: BlockReason::Question,
        text: "Gemini has no subagent-stop event".into(),
    });
    let mut t7 = gemini_task("t7", TaskState::Working, 58);
    t7.deps = vec!["t2".into()];
    t7.route = routed(
        Runtime::Codex,
        proto::Strength::Standard,
        proto::Effort::Medium,
    );
    let mut limited = worker(1, None, Runtime::Codex, now - 900);
    limited.rate_limited = true;
    limited.rate_limited_since = Some(now - 240);
    limited.rate_limited_until = Some(now + 60);
    t7.rounds = vec![limited];
    r1.tasks = vec![
        t0,
        merged("t1", 50),
        gemini_t2(now),
        t3,
        merged("t4", 70),
        t5,
        t6,
        t7,
        merged("t8", 50),
    ];

    let mut s1 = scout(
        "S1",
        "where are Claude hook events parsed, and which of them fire in -p mode?",
        Runtime::Claude,
        now - 3900,
    );
    s1.state = proto::ScoutState::Reported;
    s1.window_id = Some(4);
    s1.ended_at = Some(now - 3720);
    s1.report_bytes = Some(1840);
    s1.files = vec![
        "crates/daemon/src/hooks.rs".into(),
        "crates/daemon/src/status.rs".into(),
    ];
    r1.scouts = vec![s1];

    let mut w7 = headless(
        7,
        "r1/t2.w1",
        "/r/anthrex",
        Some(run_ref("r1", Some("t2"), AgentRole::Worker, 1)),
    );
    w7.runtime = Runtime::Codex;
    w7.status = Status::Working;
    w7.tool = Some("apply_patch".into());
    let w10 = headless(
        10,
        "r1/t3.w1",
        "/r/anthrex",
        Some(run_ref("r1", Some("t3"), AgentRole::Worker, 1)),
    );
    (snapshot(now, vec![r1]), vec![w7, w10])
}

/// The planner mockup's run (Interfaces "Sub-planner"): planner `A` `daemon`, area
/// `crates/daemon/**` and `crates/cli/src/hook.rs`, finished after 120 s, its three
/// tasks merged, working and pending, edits 3 accepted and 1 rejected (`owns outside
/// area`), re-planned once (`t2 split`). Built on the Gemini fixture's clock.
pub(crate) fn planner_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    let now = GEMINI_NOW;
    let mut info = run("r2", "/r/anthrex", RunState::Running);
    info.created_at = now - 600;
    let mut a = planner("A", "daemon");
    a.area = vec!["crates/daemon/**".into(), "crates/cli/src/hook.rs".into()];
    a.state = proto::PlannerState::Finished;
    a.started_at = now - 500;
    a.ended_at = Some(now - 380);
    a.edits_accepted = 3;
    a.edits_rejected = 1;
    a.last_rejection = Some("owns outside area".into());
    a.replans = vec!["t2 split".into()];
    info.planners = vec![a];
    info.tasks = [
        ("t1", TaskState::Merged),
        ("t2", TaskState::Working),
        ("t3", TaskState::Pending),
    ]
    .into_iter()
    .map(|(id, state)| {
        let mut info = task(id, "daemon work", Size::S, state);
        info.epic = Some("A".into());
        info
    })
    .collect();
    (snapshot(now, vec![info]), vec![])
}

// Milestone 9.5 task 20: a race and a pair, kept apart so this file stays focused.
mod patterns;
pub(crate) use patterns::{lane_reviews_fixture, pair_fixture, race_fixture};
