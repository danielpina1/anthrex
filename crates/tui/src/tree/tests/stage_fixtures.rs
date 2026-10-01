//! Milestone 9.1 task M9.1.20: a staged run for the run view's stage node, stage
//! inspector, fix-task rows and the plan gate's stage field (decision 55).

use super::run_fixtures::{PROJECT, RUN_ID, run, snapshot, task};
use proto::{
    FullInfo, FullState, RunState, RunsSnapshot, Size, StageInfo, TaskOrigin, TaskState, WindowInfo,
};

pub(crate) fn stage(n: u16, head: Option<&str>, tasks: u32, merged: u32) -> StageInfo {
    StageInfo {
        actions: Vec::new(),
        n,
        branch: format!("anthrex/{RUN_ID}/stage-{n}"),
        head: head.map(str::to_owned),
        tasks,
        merged,
        full: FullInfo::default(),
        fix_tasks: vec![],
        propagate_red: None,
    }
}

/// Run `add-reset-3f9a`, `running`, in two stages. Stage 1 (created at `1111111…`,
/// 1/3 merged) is bisecting a red tier 3 on `2222222…` (41m12s, 2 shards, flaky
/// `a::flaky`, failing `a::works`) and made fix task `fix1` (bisect of t2). Stage 2 is
/// not created: `t3`, pending.
pub(crate) fn staged_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let mut fix = task("fix1", "fix t2", Size::S, TaskState::Queued);
    fix.origin = TaskOrigin::Bisect;
    fix.fixes = Some("bisect of t2".into());
    let mut t3 = task("t3", "reset view", Size::S, TaskState::Pending);
    t3.stage = 2;
    info.tasks = vec![
        task("t1", "reset model", Size::S, TaskState::Merged),
        task("t2", "reset endpoint", Size::S, TaskState::Working),
        t3,
        fix,
    ];
    let mut one = stage(1, Some(&"1".repeat(40)), 3, 1);
    one.full = FullInfo {
        state: FullState::Bisecting,
        at: Some(9_000),
        secs: Some(2472),
        commit: Some("2".repeat(40)),
        shards: 2,
        flaky: vec!["a::flaky".into()],
        failing: vec!["a::works".into()],
        bisect_fixes: 1,
        note: None,
    };
    one.fix_tasks = vec!["fix1".into()];
    info.stages = vec![one, stage(2, None, 1, 0)];
    (snapshot(10_000, vec![info]), vec![])
}

/// The staged plan at the gate: `t1` in stage 1 and `t2` in stage 2, neither stage
/// created yet (C-24: no head before approval).
pub(crate) fn staged_gate_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    let mut gate = run(RUN_ID, PROJECT, RunState::AwaitingApproval);
    let mut t2 = task("t2", "reset endpoint", Size::S, TaskState::Pending);
    t2.stage = 2;
    gate.tasks = vec![
        task("t1", "reset token model", Size::M, TaskState::Pending),
        t2,
    ];
    gate.stages = vec![stage(1, None, 1, 0), stage(2, None, 1, 0)];
    (snapshot(10_000, vec![gate]), vec![])
}
