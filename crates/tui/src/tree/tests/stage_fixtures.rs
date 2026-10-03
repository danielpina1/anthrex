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
        pr: None,
        round: 1,
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

/// Milestone 9.0.7 task 10: run `add-mul-0723` (`Add mul()`), `running`, in two stages,
/// with no windows but the plain shell `1`. Stage 1 is green in 38 s with `t1 add mul()
/// to a` (S, tdd) merged; stage 2 runs tier 3 with `t2 report_product in c` (M, tdd) in
/// review round 1 after `t1`, and `t3 docs for report_product` (S, none) waiting after
/// `t2`.
pub(crate) fn two_stage_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    let id = "add-mul-0723";
    let mut info = run(id, PROJECT, RunState::Running);
    info.goal = "Add mul()".into();
    let t1 = task("t1", "add mul() to a", Size::S, TaskState::Merged);
    let mut t2 = task("t2", "report_product in c", Size::M, TaskState::Review);
    t2.stage = 2;
    t2.deps = vec!["t1".into()];
    t2.reviews = vec![proto::ReviewInfo {
        round: 1,
        route: t2.route.clone(),
        verdict: None,
        summary: String::new(),
        findings: vec![],
        blocking: false,
    }];
    t2.review_route = Some(t2.route.clone());
    let mut t3 = task("t3", "docs for report_product", Size::S, TaskState::Pending);
    t3.stage = 2;
    t3.deps = vec!["t2".into()];
    t3.test_mode = proto::TestMode::None;
    info.tasks = vec![t1, t2, t3];
    let mut one = stage(1, Some(&"1".repeat(40)), 1, 1);
    one.full.state = FullState::Green;
    one.full.secs = Some(38);
    let mut two = stage(2, Some(&"2".repeat(40)), 2, 0);
    two.full.state = FullState::Running;
    for s in [&mut one, &mut two] {
        s.branch = format!("anthrex/{id}/stage-{}", s.n);
    }
    info.stages = vec![one, two];
    let shell = super::run_fixtures::pty(1, "shell", PROJECT, proto::Status::Idle);
    (snapshot(10_000, vec![info]), vec![shell])
}

/// Milestone 9.3 task 10b: the two-stage run as round 2 of two. Round 1 (`Add mul()`,
/// completed, summary `mul is in a`) made stage 1 and `t1`; round 2 (`also report the
/// product`, running) made stage 2 with `t2` and `t3`.
pub(crate) fn two_round_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    let (mut snap, windows) = two_stage_fixture();
    let run = &mut snap.runs[0];
    let round = |n, head: &str, outcome, summary: Option<&str>| proto::RoundInfo {
        n,
        goal_head: head.into(),
        origin: proto::RoundOrigin::User,
        outcome,
        summary_head: summary.map(str::to_owned),
        ended: outcome.is_some(),
    };
    run.round = 2;
    run.rounds = vec![
        round(
            1,
            "Add mul()",
            Some(proto::RoundOutcome::Completed),
            Some("mul is in a"),
        ),
        round(2, "also report the product", None, None),
    ];
    for task in &mut run.tasks[1..] {
        task.round = 2;
    }
    run.stages[1].round = 2;
    (snap, windows)
}
