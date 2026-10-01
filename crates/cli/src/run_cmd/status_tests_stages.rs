//! Milestone 9.1 task M9.1.20: `run status`'s stage lines and task suffixes for a
//! `Multi` run (Interfaces "CLI").

use proto::{FullInfo, FullState, StageInfo, TaskOrigin, TaskState};

use super::run_block;
use super::tests::example;

fn stage(n: u16, head: Option<&str>, (merged, tasks): (u32, u32), full: FullInfo) -> StageInfo {
    StageInfo {
        actions: Vec::new(),
        n,
        branch: format!("anthrex/add-reset-3f9a/stage-{n}"),
        head: head.map(str::to_string),
        tasks,
        merged,
        full,
        fix_tasks: Vec::new(),
        propagate_red: None,
    }
}

fn full(state: FullState, secs: Option<u64>) -> FullInfo {
    FullInfo {
        state,
        secs,
        ..FullInfo::default()
    }
}

/// Pinning: a `Single` run (one stage) prints exactly milestone 9's text, whatever its
/// one stage and its tasks' new fields hold.
#[test]
fn status_prints_stage_lines_for_a_multi_run_only() {
    let mut single = example();
    single.stages = vec![StageInfo {
        branch: single.run_branch.clone(),
        ..stage(1, Some("ffff"), (1, 4), full(FullState::Green, Some(2472)))
    }];
    single.tasks[0].stage = 1;
    assert_eq!(
        run_block(&single, 0),
        include_str!("status_single_golden.txt")
    );

    let mut multi = example();
    multi.tasks[2].stage = 2;
    multi.tasks[3].stage = 2;
    multi.tasks[3].origin = TaskOrigin::Bisect;
    multi.tasks[3].fixes = Some("bisect of t2".into());
    multi.tasks[3].id = "fix1".into();
    multi.tasks[2].state = TaskState::Merged;
    let mut s2 = stage(2, Some("2222"), (2, 4), full(FullState::Bisecting, None));
    s2.fix_tasks = vec!["fix1".into()];
    multi.stages = vec![
        stage(1, Some("1111"), (5, 5), full(FullState::Green, Some(2472))),
        s2,
        stage(3, None, (0, 0), FullInfo::default()),
    ];
    let text = run_block(&multi, 0);
    let expected = "\
add-reset-3f9a  running  2/4 merged  base main@1a2b3c4  writers 2/3  readers 1/3  rev 57
  goal: Add password reset
  report: /Users/me/Library/Application Support/anthrex/runs/add-reset-3f9a/REPORT.md
stage 1/3  anthrex/add-reset-3f9a/stage-1  5/5 merged  tier 3 green (41m 12s)
stage 2/3  anthrex/add-reset-3f9a/stage-2  2/4 merged  tier 3 bisecting (fix1)
stage 3/3  not created
  ID   SIZE MODE   STATE          RUNG BOUNCES        ROUTE                          WINDOWS
  t1   M◆   tdd    merged         0    -              claude claude-opus-5 high      4 5 [stage 1]
  t2   M    tdd    review         1    review 1       codex (default) medium         6 9 [stage 1]
  t3   S    check  merged         0    -              claude claude-sonnet-5 low [stage 2]
  fix1 S    none   blocked        3    check 3        claude claude-sonnet-5 low     7 [stage 2] (fix: bisect of t2)
  attention: t4 blocked (mis_sized): check failed 3 times
";
    assert_eq!(text, expected);
}

/// Durations as the CLI prints them, and the running, red and none states.
#[test]
fn stage_lines_show_each_tier3_state() {
    let mut multi = example();
    multi.tasks[3].stage = 2;
    multi.stages = vec![
        stage(1, Some("1"), (1, 1), full(FullState::Red, Some(42))),
        stage(2, Some("2"), (0, 1), full(FullState::Running, None)),
        stage(
            3,
            Some("3"),
            (0, 0),
            full(FullState::Green, Some(3 * 3600 + 120)),
        ),
        stage(4, Some("4"), (0, 0), full(FullState::None, None)),
    ];
    let text = run_block(&multi, 0);
    let lines: Vec<&str> = text.lines().filter(|l| l.starts_with("stage ")).collect();
    assert_eq!(
        lines,
        [
            "stage 1/4  anthrex/add-reset-3f9a/stage-1  1/1 merged  tier 3 red (42s)",
            "stage 2/4  anthrex/add-reset-3f9a/stage-2  0/1 merged  tier 3 running",
            "stage 3/4  anthrex/add-reset-3f9a/stage-3  0/0 merged  tier 3 green (3h 2m)",
            "stage 4/4  anthrex/add-reset-3f9a/stage-4  0/0 merged  tier 3 none",
        ]
    );
}
