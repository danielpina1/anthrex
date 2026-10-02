//! Milestone 9.2 task M9.2.15: `pr`-mode runs for the run view's stage rows, the stage
//! and run inspectors and the keys (decisions 36, 41 and 42). ASCII text only, so the
//! render audit can check an ASCII frame whole.

use super::run_fixtures::{PROJECT, RUN_ID, run, snapshot, task};
use super::stage_fixtures::stage;
use proto::{
    CheckRunInfo, CiState, DeliveryInfo, DeliveryMode, FullState, PrState, RunState, RunsSnapshot,
    Size, StagePrInfo, TaskOrigin, TaskState, ThreadCounts, WindowInfo,
};

/// `pr` to `origin` (`fake/app`), watched every 60 s.
pub(crate) fn delivery(delivering: bool) -> DeliveryInfo {
    DeliveryInfo {
        mode: DeliveryMode::Pr,
        remote: "origin".into(),
        repo: "fake/app".into(),
        watching: true,
        delivering,
        poll_secs: 60,
        skipped_stages: Vec::new(),
        alerts: Vec::new(),
    }
}

/// PR `#<number>` on `main`, opened at 02:30 (9,000 s, UTC), with no checks or threads.
pub(crate) fn pr(number: u64, state: PrState, ci: CiState) -> StagePrInfo {
    StagePrInfo {
        number,
        url: format!("https://github.com/fake/app/pull/{number}"),
        state,
        base: "main".into(),
        opened_at: 9_000,
        head: "1a2b3c4".into(),
        ci,
        checks: Vec::new(),
        threads: ThreadCounts::default(),
        fix_tasks: Vec::new(),
        paused: false,
        human_review_secs: 0,
        merged_at: None,
        merge_commit: None,
    }
}

pub(crate) fn check(name: &str, state: CiState, fix_task: Option<&str>) -> CheckRunInfo {
    CheckRunInfo {
        name: name.into(),
        state,
        fix_task: fix_task.map(str::to_owned),
    }
}

/// Run `add-reset-3f9a`, `running` and delivering, in three stages, every one green on
/// its head. Stage 1's `#141` merged at 03:00; stage 2's `#142` (based on stage 1's
/// branch) is open with CI red (`build` green, `test` red and fixed by `fix3`), 2 new,
/// 1 tasked, 1 replied and 4 ignored threads, and the fix tasks `fix3` (ci, working)
/// and `fix4` (review, merged); stage 3's `#143` is open with CI pending.
pub(crate) fn pr_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    let in_stage = |id: &str, title: &str, state, n: u16| {
        let mut t = task(id, title, Size::S, state);
        t.stage = n;
        t
    };
    let mut fix3 = in_stage("fix3", "fix the red test", TaskState::Working, 2);
    fix3.origin = TaskOrigin::Ci;
    fix3.fixes = Some("CI run 77".into());
    let mut fix4 = in_stage("fix4", "rename per review", TaskState::Merged, 2);
    fix4.origin = TaskOrigin::Review;
    fix4.fixes = Some("thread by @alice".into());
    info.tasks = vec![
        in_stage("t1", "reset model", TaskState::Merged, 1),
        in_stage("t2", "reset endpoint", TaskState::Merged, 2),
        in_stage("t3", "reset view", TaskState::Merged, 3),
        fix3,
        fix4,
    ];
    let mut one = pr(141, PrState::Merged, CiState::Green);
    one.merged_at = Some(10_800);
    let mut two = pr(142, PrState::Open, CiState::Red);
    two.base = format!("anthrex/{RUN_ID}/stage-1");
    two.checks = vec![
        check("build", CiState::Green, None),
        check("test", CiState::Red, Some("fix3")),
    ];
    two.threads = ThreadCounts {
        new: 2,
        tasked: 1,
        replied: 1,
        ignored: 4,
    };
    two.fix_tasks = vec!["fix3 ci working".into(), "fix4 review merged".into()];
    let mut three = pr(143, PrState::Open, CiState::Pending);
    three.base = format!("anthrex/{RUN_ID}/stage-2");
    let mut stages = vec![
        stage(1, Some(&"1".repeat(40)), 1, 1),
        stage(2, Some(&"2".repeat(40)), 3, 2),
        stage(3, Some(&"3".repeat(40)), 1, 1),
    ];
    for (s, pr) in stages.iter_mut().zip([one, two, three]) {
        s.full.state = FullState::Green;
        s.full.secs = Some(38);
        s.pr = Some(pr);
    }
    info.stages = stages;
    info.delivery = Some(delivery(true));
    (snapshot(10_000, vec![info]), vec![])
}

/// Run `add-reset-3f9a`, `running` and delivering, in one stage (so no stage node):
/// `t1` merged, its `#7` open on `main` with CI green on `build`.
pub(crate) fn single_pr_fixture() -> (RunsSnapshot, Vec<WindowInfo>) {
    let mut info = run(RUN_ID, PROJECT, RunState::Running);
    info.tasks = vec![task("t1", "reset model", Size::S, TaskState::Merged)];
    let mut one = stage(1, Some(&"1".repeat(40)), 1, 1);
    one.full.state = FullState::Green;
    let mut seven = pr(7, PrState::Open, CiState::Green);
    seven.checks = vec![check("build", CiState::Green, None)];
    one.pr = Some(seven);
    info.stages = vec![one];
    info.delivery = Some(delivery(true));
    (snapshot(10_000, vec![info]), vec![])
}
