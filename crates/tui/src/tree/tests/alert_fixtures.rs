//! Milestone 9.0.5's alert fixture (task M9.0.5.8), shared by the alerts' reducer
//! tests and the Alerts box's render tests so the two never disagree.

use super::orch_fixtures::{hold, orchestrator_info};
use super::run_fixtures::{pty, run, run_ref, snapshot, task, worker};
use proto::{
    AgentRole, BlockInfo, BlockReason, HoldState, ProposalAlertInfo, RunInfo, RunState,
    RunsSnapshot, Runtime, Size, Status, TaskState, WindowInfo,
};

const PROJECT: &str = "/r/demo";

/// A PTY orchestrator window for `run_id`: Claude, `status`, `signals_seen` as given.
pub(crate) fn orch_window(id: u32, run_id: &str, status: Status, signals_seen: bool) -> WindowInfo {
    let mut window = pty(id, &format!("orch-{run_id}"), PROJECT, status);
    window.runtime = Runtime::Claude;
    window.run = Some(run_ref(run_id, None, AgentRole::Orchestrator, 1));
    window.signals_seen = signals_seen;
    window
}

pub(crate) fn at(id: &str, state: RunState, created_at: u64) -> RunInfo {
    let mut info = run(id, PROJECT, state);
    info.created_at = created_at;
    info
}

pub(crate) fn blocked(id: &str, reason: BlockReason, text: &str) -> proto::TaskInfo {
    let mut t = task(id, "work", Size::S, TaskState::Blocked);
    t.block = Some(BlockInfo::new(reason, text));
    t
}

pub(crate) fn with_orch(mut info: RunInfo, window: u32) -> RunInfo {
    info.orchestrator = Some(orchestrator_info(Some(window)));
    info
}

/// Every source at once (decision 18):
/// - `a-attn`: its orchestrator asks for permission;
/// - `b-gate`: its orchestrator waits at a start prompt, and its plan awaits approval
///   (two tasks, one cancelled);
/// - `c-held`: a held wake-up, a hold awaiting approval, a `Human` block, a `Question`
///   block with its orchestrator live (no alert), a paused task (no alert) and a
///   worker round that just ended a turn (no alert);
/// - `d-bare`: no orchestrator, a `Question` block;
/// - `e-halt`: halted, with a two-line reason;
/// - `f-done`: complete, two of three tasks merged, one cancelled;
/// - and a ready profile proposal for `/r/shop`.
pub(crate) fn every_source() -> (RunsSnapshot, Vec<WindowInfo>) {
    let a = with_orch(at("a-attn", RunState::Running, 1), 11);

    let mut b = with_orch(at("b-gate", RunState::AwaitingApproval, 2), 12);
    b.tasks = vec![
        task("t1", "one", Size::S, TaskState::Pending),
        task("t2", "two", Size::S, TaskState::Cancelled),
    ];

    let mut c = with_orch(at("c-held", RunState::Running, 3), 13);
    if let Some(orch) = &mut c.orchestrator {
        orch.wake_held = true;
    }
    let mut done_turn = task("t4", "turn", Size::S, TaskState::Working);
    let mut round = worker(1, Some(20), Runtime::Claude, 0);
    round.turns = 1;
    round.turn_open = false;
    done_turn.rounds = vec![round];
    let mut held = task("t5", "ui", Size::S, TaskState::Pending);
    held.hold = Some("epic:ui".into());
    c.tasks = vec![
        blocked("t1", BlockReason::Human, "needs a key\nsecond line"),
        blocked("t2", BlockReason::Question, "which db?"),
        blocked("t3", BlockReason::MessagePause, "hold on"),
        done_turn,
        held,
    ];
    c.holds = vec![hold("epic:ui", HoldState::Awaiting, &["t5"])];

    let mut d = at("d-bare", RunState::Running, 4);
    d.tasks = vec![blocked("t1", BlockReason::Question, "which db?")];

    let mut e = at("e-halt", RunState::Halted, 5);
    e.halted_reason = Some("disk full\nat /tmp".into());

    let mut f = at("f-done", RunState::Complete, 6);
    f.tasks = vec![
        task("t1", "one", Size::S, TaskState::Merged),
        task("t2", "two", Size::S, TaskState::Merged),
        task("t3", "three", Size::S, TaskState::Cancelled),
    ];

    let mut snap = snapshot(10_000, vec![f, e, d, c, b, a]);
    snap.proposals = vec![ProposalAlertInfo {
        project: "/r/shop".into(),
        updated_at: 9_000,
    }];
    let windows = vec![
        pty(1, "shell", PROJECT, Status::Idle),
        orch_window(11, "a-attn", Status::Attention, true),
        orch_window(12, "b-gate", Status::Idle, false),
        orch_window(13, "c-held", Status::Working, true),
    ];
    (snap, windows)
}
