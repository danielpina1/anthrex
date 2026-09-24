//! Runs, rounds, pending ops and windows for the journal and reconcile tests. A run is
//! built from a plan through the public `build_run`, as `run start` builds one.

use daemon::headless::HeadlessSpec;
use daemon::run::engine::{BudgetEpoch, OpKind, OpResult, OverrideCount, ResolutionAt, TaskClock};
use daemon::run::journal::JournalLine;
use daemon::run::model::{
    AgentRound, BaseMoved, CheckRecord, DoneClaim, FailedTurn, FallbackState, FreshSession,
    LogEntry, OpId, Outgoing, PendingClaim, PendingOp, ProofRecord, ReviewRecord, Run, StallState,
    TaskEvent,
};
use daemon::run::plan::{BuildContext, Preflight, build_run, parse_plan};
use proto::{
    AgentRole, BlockInfo, BlockReason, DoneSignal, Effort, Finding, Route, RunRef, Runtime,
    Severity, Status, Strength, TokenUsage, Verdict, WindowInfo, WindowKind,
};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

pub const RUN_ID: &str = "journal-test-5a1e";

const PLAN: &str = r#"
goal = "Journal and reconcile"

[[task]]
id = "t1"
title = "First"
size = "S"
owns = ["src/a.txt"]
brief = "Write a."
acceptance = ["a exists"]

[[task]]
id = "t2"
title = "Second"
size = "S"
owns = ["src/b.txt"]
brief = "Write b."
acceptance = ["b exists"]
"#;

/// A fresh run of the two-task plan: repository `root` on `main` at `base_sha`, engine
/// worktrees under `wt_dir`, its own data directory `<data_dir>/runs/<id>`.
pub fn run_at(data_dir: &Path, root: &Path, wt_dir: &Path, base_sha: &str) -> Run {
    let plan = parse_plan(PLAN).expect("the fixture plan parses");
    let pre = Preflight {
        root: root.to_path_buf(),
        project: root.to_path_buf(),
        git_common_dir: root.join(".git"),
        base_branch: "main".into(),
        base_sha: base_sha.into(),
        protected_files: Vec::new(),
    };
    let config = config::Orchestrator::default();
    let ctx = BuildContext {
        id: RUN_ID.into(),
        wt_dir: wt_dir.to_path_buf(),
        data_dir: data_dir.join("runs").join(RUN_ID),
        config: &config,
        now: 1_000,
        yes: true,
    };
    build_run(plan, pre, ctx).unwrap_or_else(|e| panic!("the fixture run builds: {e:?}"))
}

/// A run with no repository behind it, for the journal's own tests.
pub fn plain_run(data_dir: &Path) -> Run {
    run_at(
        data_dir,
        Path::new("/tmp/nowhere"),
        Path::new("/tmp/nowhere-wt"),
        &"b".repeat(40),
    )
}

pub fn route() -> Route {
    Route {
        runtime: Runtime::Claude,
        model: "claude-sonnet-5".into(),
        strength: Strength::Standard,
        effort: Effort::High,
    }
}

/// A live (not ended) round with the given session id and pid.
pub fn round(
    role: AgentRole,
    session: u32,
    launch_op: OpId,
    session_id: Option<&str>,
    pid: Option<u32>,
) -> AgentRound {
    AgentRound {
        role,
        session,
        round: 1,
        window_id: Some(7),
        route: route(),
        launch_op,
        session_id: session_id.map(str::to_string),
        pid,
        ended: false,
        started_at: 1_000,
        ended_at: None,
        turn_open: true,
        turns: 1,
        turn_had_task_done: false,
        last_event: 1_010,
        tool_calls: 3,
        rate_limited_until: None,
        in_retry_streak: false,
        open_subagents: BTreeSet::new(),
        denials: 0,
        usage: TokenUsage::default(),
        deaths: 0,
        fallback: FallbackState::None,
        stall: StallState::Watching,
        failed_turn: FailedTurn::None,
        review_nudged: false,
        wrap_up_sent: false,
        retiring: false,
        delivery_failures: 0,
        delivery_retry_at: None,
        turn_denied: Vec::new(),
        excused_secs: 0,
        last_denial: None,
        fallback_waiting: false,
        carried: Vec::new(),
        failed_error: None,
        resume_op: None,
        count_op: None,
        count_failures: 0,
        count_retry_at: None,
        count_turn: 0,
        interrupted: false,
        relaunch: None,
    }
}

pub fn run_ref(task: &str, role: AgentRole, session: u32) -> RunRef {
    RunRef {
        run_id: RUN_ID.into(),
        task_id: Some(task.into()),
        role,
        session,
    }
}

/// A worker's session spec carrying `run_ref`.
pub fn spec(cwd: &Path, run_ref: RunRef) -> HeadlessSpec {
    HeadlessSpec {
        runtime: Runtime::Claude,
        model: "claude-sonnet-5".into(),
        effort: Effort::High,
        cwd: cwd.to_path_buf(),
        instructions: "the contract".into(),
        mcp: None,
        allowed_tools: vec!["Read".into()],
        claude_permission_mode: Some("acceptEdits".into()),
        claude_disallowed_tools: Vec::new(),
        claude_sandbox: None,
        codex_sandbox: "workspace-write".into(),
        codex_writable_roots: Vec::new(),
        env: Vec::new(),
        claude_auth: config::ClaudeAuth::Login,
        api_key_helper: None,
        run_ref: Some(run_ref),
    }
}

pub fn create_window(cwd: &Path, run_ref: RunRef, uuid: Option<&str>) -> OpKind {
    OpKind::CreateWindow {
        name: "t1 worker".into(),
        spec: Box::new(spec(cwd, run_ref)),
        session_uuid: uuid.map(str::to_string),
        first_turn: "Do the task.".into(),
        project: cwd.to_path_buf(),
        worktree: cwd.to_path_buf(),
        jitter_ms: 0,
    }
}

/// Records `kind` as pending op `op` of `task`, as the engine's `emit_op` does.
pub fn pend(run: &mut Run, op: OpId, task: Option<&str>, kind: OpKind) {
    run.pending_ops.insert(
        op,
        PendingOp {
            op,
            task_id: task.map(str::to_string),
            kind,
        },
    );
    run.next_op = run.next_op.max(op + 1);
}

/// A restored headless window, `Exited`, as decision 28 leaves one.
pub fn window(id: u32, run_ref: Option<RunRef>, kind: WindowKind) -> WindowInfo {
    WindowInfo {
        id,
        name: format!("window {id}"),
        runtime: Runtime::Claude,
        cwd: PathBuf::from("/tmp"),
        project: PathBuf::from("/tmp"),
        worktree: None,
        branch: None,
        status: Status::Exited,
        tool: None,
        since_secs: 0,
        last_output_secs: 0,
        session_id: None,
        model: None,
        subagents: Vec::new(),
        exit: None,
        kind,
        run: run_ref,
    }
}

/// `run` with every nested record the model has set to something non-default, so a
/// round trip that dropped any of them would show.
pub fn full_run(data_dir: &Path) -> Run {
    let mut run = plain_run(data_dir);
    run.last_green_candidate = Some("c".repeat(40));
    run.base_moved = Some(BaseMoved {
        from: "b".repeat(40),
        to: "d".repeat(40),
        commits: 2,
        seen_at: 1_500,
    });
    run.halted_reason = Some("a reason".into());
    run.approved_by = Some("--yes".into());
    run.merge_queue = vec!["t2".into()];
    run.outbox.push(Outgoing {
        id: 4,
        window_id: 7,
        task_id: "t1".into(),
        text: "a message".into(),
        queued_at: 1_200,
        delivered_at: Some(1_201),
    });
    run.next_message = 5;
    run.log.push(LogEntry {
        at: 1_100,
        text: "a log line".into(),
    });
    run.rate_limits.insert("claude".into(), 2);
    run.trusted_project = vec![".mcp.json".into()];
    run.protected_files = vec!["AGENTS.md".into()];
    run.outcome = Some("an outcome".into());
    run.restored = Some(1_300);
    run.finish_reply = Some(9);
    pend(
        &mut run,
        11,
        Some("t1"),
        create_window(
            Path::new("/tmp/t1"),
            run_ref("t1", AgentRole::Worker, 1),
            Some("00000000-0000-4000-8000-000000000011"),
        ),
    );
    pend(
        &mut run,
        12,
        Some("t2"),
        OpKind::HandBack {
            worktree: PathBuf::from("/tmp/t2"),
            run_head: "e".repeat(40),
            task_head: Some("f".repeat(40)),
        },
    );

    let task = &mut run.tasks[0];
    task.block = Some(BlockInfo {
        reason: BlockReason::Question,
        text: "which?".into(),
    });
    task.rung = 2;
    task.failures = 1;
    task.start_commit = Some("b".repeat(40));
    task.head = Some("f".repeat(40));
    task.gate_op = Some(10);
    task.merge_op = Some(12);
    task.resolution = Some(ResolutionAt {
        onto: "f".repeat(40),
        run_head: "e".repeat(40),
        files: vec!["src/a.txt".into()],
    });
    task.override_count = Some(OverrideCount {
        op: 13,
        reply: 14,
        reason: "because".into(),
    });
    task.clock = TaskClock {
        stopped: Some(1_400),
        restarted: 1_100,
    };
    task.epoch = Some(BudgetEpoch {
        round: 1,
        tool_calls: 5,
        tokens: 600,
    });
    let claim = DoneClaim {
        summary: "done".into(),
        test: Some("a_test".into()),
        red: Some("a".repeat(40)),
        signal: DoneSignal::TaskDone,
    };
    task.done = Some(claim.clone());
    task.claim = Some(PendingClaim {
        reply: Some(15),
        claim,
        window_id: Some(7),
        op: Some(16),
        turn: 1,
    });
    task.fresh_session = Some(FreshSession {
        reason: "stalled".into(),
        append: Some("more".into()),
    });
    let mut worker = round(
        AgentRole::Worker,
        1,
        11,
        Some("00000000-0000-4000-8000-000000000011"),
        Some(4242),
    );
    worker.open_subagents.insert("sub-1".into());
    worker.usage = TokenUsage {
        input: 1,
        output: 2,
        cache_read: 3,
        cache_write: 4,
    };
    worker.fallback = FallbackState::Nudged { had_commits: true };
    worker.stall = StallState::Interrupted { deadline: 1_600 };
    worker.failed_turn = FailedTurn::WaitingContinue {
        at: 1_700,
        rate_limit: true,
    };
    worker.relaunch = Some(Box::new(create_window(
        Path::new("/tmp/t1"),
        run_ref("t1", AgentRole::Worker, 1),
        None,
    )));
    worker.carried = vec![4];
    task.rounds.push(worker);
    task.reviews.push(ReviewRecord {
        round: 1,
        route: route(),
        base: "b".repeat(40),
        head: "f".repeat(40),
        verdict: Some(Verdict::Changes),
        summary: "fix it".into(),
        findings: vec![Finding {
            severity: Severity::Important,
            file: Some("src/a.txt".into()),
            line: Some(3),
            input: None,
            text: "wrong".into(),
        }],
    });
    task.checks.push(CheckRecord {
        at: 1_250,
        ok: false,
        code: Some(1),
        timed_out: false,
        tail: "FAILED".into(),
        secs: 4,
        on_candidate: true,
    });
    task.proofs.push(ProofRecord {
        at: 1_260,
        test: "a_test".into(),
        red: "a".repeat(40),
        head: "f".repeat(40),
        red_failed: true,
        head_passed: true,
        matched: true,
        red_tail: "red".into(),
        head_tail: "green".into(),
    });
    task.salvage_refs = vec!["refs/anthrex/salvage/x".into()];
    task.failure_log = vec!["a failure".into()];
    task.history.push(TaskEvent {
        at: 1_050,
        text: "dispatched".into(),
    });
    run
}

/// The results `OpResult` can carry, one of each shape the journal must round-trip.
pub fn some_results() -> Vec<OpResult> {
    vec![
        OpResult::Worktree {
            head: "a".repeat(40),
        },
        OpResult::Window { window_id: 3 },
        OpResult::MergeAborted,
        OpResult::HandedBack {
            files: vec!["x".into()],
            head: Some("b".repeat(40)),
            onto: None,
        },
        OpResult::Finished {
            outcome: "accepted as abcdef1".into(),
            kept_branches: vec!["anthrex/r/t1".into()],
        },
    ]
}

/// An intent line for every pending op, as decision 43 writes them.
pub fn intents(run: &Run) -> Vec<JournalLine> {
    run.pending_ops
        .values()
        .map(|p| JournalLine::Intent {
            op: p.op,
            kind: p.kind.clone(),
        })
        .collect()
}
