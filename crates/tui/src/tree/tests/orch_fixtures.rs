//! M9.15's fixtures: milestone 8c's three-task run with milestone 9's fields — the
//! orchestrator's record, approval holds, a `paused(message)` task, a reported task,
//! messages and task notes.

use super::run_fixtures::{RUN_ID, task, three_task_fixture};
use proto::{
    BlockInfo, BlockReason, HoldInfo, HoldKind, HoldState, OrchestratorInfo, RunState,
    RunsSnapshot, Size, TaskKind, TaskState, WindowInfo,
};

/// The three-task fixture's `now`.
pub(crate) const ORCH_NOW: u64 = 10_000;

/// The orchestrator's record for window `window`: Claude, live, the plan not yet
/// submitted, no summary and no wakes.
pub(crate) fn orchestrator_info(window: Option<u32>) -> OrchestratorInfo {
    OrchestratorInfo {
        route: serde_json::from_value(serde_json::json!({
            "runtime": "claude", "model": "claude-opus-5", "strength": "frontier",
            "effort": "high",
        }))
        .expect("a Route"),
        window_id: window,
        live: true,
        started_at: ORCH_NOW - 600,
        plan_submitted: false,
        summary: None,
        notes: vec![],
        wakes: 0,
    }
}

/// A hold of an epic, in `state`, over `tasks`.
pub(crate) fn hold(id: &str, state: HoldState, tasks: &[&str]) -> HoldInfo {
    HoldInfo {
        id: id.into(),
        kind: HoldKind::Epic {
            epic: id.trim_start_matches("epic:").into(),
        },
        state,
        tasks: tasks.iter().map(|t| (*t).to_string()).collect(),
        created_at: ORCH_NOW - 60,
        decided_at: None,
        decided_by: None,
    }
}

/// The three-task fixture in `state`, with the orchestrator's record on window 3.
pub(crate) fn orch_fixture(state: RunState) -> (RunsSnapshot, Vec<WindowInfo>) {
    let (mut snapshot, windows) = three_task_fixture();
    let run = &mut snapshot.runs[0];
    assert_eq!(run.run_id, RUN_ID);
    run.state = state;
    run.orchestrator = Some(orchestrator_info(Some(3)));
    (snapshot, windows)
}

/// A `running` run with every milestone 9 task look: `t1` is `paused(message)`, `t2`
/// is held in hold `epic:ui` (awaiting approval), `t3 findings` is a reported research
/// task.
pub(crate) fn held_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    let (mut snapshot, windows) = orch_fixture(RunState::Running);
    let run = &mut snapshot.runs[0];
    let t1 = &mut run.tasks[1];
    t1.state = TaskState::Blocked;
    t1.block = Some(BlockInfo {
        reason: BlockReason::MessagePause,
        text: "asked to stop and wait: hold on".into(),
    });
    run.tasks[2].hold = Some("epic:ui".into());
    let mut t3 = task("t3", "findings", Size::S, TaskState::Reported);
    t3.kind = TaskKind::Research;
    run.tasks.push(t3);
    run.holds = vec![hold("epic:ui", HoldState::Awaiting, &["t2"])];
    (snapshot, windows)
}
