//! M8b.17: `run stats` (decision 35), pure over hand-built records.

use std::path::Path;

use proto::{
    DeciderSource, DiffStats, Effort, GateCounts, GateTally, HISTORY_VERSION, HistoryLine,
    HistoryStats, PhaseSecs, RevertRecord, Route, RunRecord, RunUsage, Runtime, SeverityTally,
    Size, SizeCheckInfo, StatsRow, Strength, TaskKind, TaskOutcome, TaskRecord, TestMode,
    TokenUsage,
};

use super::{aggregate, render, tokens};

const PATH: &str = "/tmp/h/history.jsonl";

fn route() -> Route {
    Route {
        runtime: Runtime::Claude,
        model: "m".into(),
        strength: Strength::Standard,
        effort: Effort::Medium,
    }
}

/// A task record of class `size` (`hub`), `outcome`, after `sessions` sessions, with
/// nothing measured.
fn task(
    run: &str,
    id: &str,
    size: Size,
    hub: bool,
    outcome: TaskOutcome,
    sessions: u32,
) -> TaskRecord {
    TaskRecord {
        v: HISTORY_VERSION,
        record_id: format!("{run}/{id}"),
        at: 100,
        run_id: run.into(),
        task_id: id.into(),
        path: None,
        kind: TaskKind::Code,
        hub,
        test_mode: TestMode::Tdd,
        planned_size: size,
        final_size: size,
        size_check: None,
        route: route(),
        review_routes: Vec::new(),
        routing_decisions: Vec::new(),
        outcome,
        block: None,
        diff: None,
        tool_calls: 0,
        worker_usage: TokenUsage::default(),
        reviewer_usage: TokenUsage::default(),
        decider_usage: TokenUsage::default(),
        phases: PhaseSecs::default(),
        wall_secs: 0,
        gates: GateTally::default(),
        severities: SeverityTally::default(),
        bounces: GateCounts::default(),
        failures: 0,
        stalls: 0,
        budget_exceeded: 0,
        conflicts: 0,
        max_rung: 0,
        sessions,
        done_signal: None,
        merge_commit: None,
        stage: 1,
        origin: proto::TaskOrigin::Plan,
    }
}

/// A merged task: `lines` added, `tool_calls`, `billable` tokens split over the worker,
/// the reviewer and the deciders (cache reads, never billed, on top), `work` seconds
/// working.
fn merged(
    run: &str,
    id: &str,
    size: Size,
    (lines, tool_calls, billable, work): (u32, u32, u64, u64),
) -> TaskRecord {
    let mut record = task(run, id, size, false, TaskOutcome::Merged, 1);
    record.diff = Some(DiffStats {
        files: 1,
        hunks: 1,
        added: lines,
        removed: 0,
    });
    record.tool_calls = tool_calls;
    record.worker_usage = TokenUsage {
        input: billable / 2,
        cache_read: 1_000_000,
        ..TokenUsage::default()
    };
    record.reviewer_usage.output = billable / 4;
    record.decider_usage.cache_write = billable - billable / 2 - billable / 4;
    record.phases.working = work;
    record.merge_commit = Some(format!("{:0>40}", id.len()));
    record
}

fn run(id: &str, calls: u32, fallbacks: u32) -> RunRecord {
    RunRecord {
        v: HISTORY_VERSION,
        record_id: id.into(),
        at: 200,
        run_id: id.into(),
        goal: "g".into(),
        path: None,
        triage: None,
        profile_source: None,
        outcome: "accepted".into(),
        base_branch: "main".into(),
        accepted_commit: Some("a".repeat(40)),
        tasks: 1,
        usage: Some(RunUsage {
            decider_calls: calls,
            decider_fallbacks: fallbacks,
            ..RunUsage::default()
        }),
    }
}

fn check(engine: Size, decided: Option<Size>, source: DeciderSource) -> Option<SizeCheckInfo> {
    Some(SizeCheckInfo {
        engine,
        decided,
        agreed: decided.is_none_or(|d| d <= engine),
        reason: "r".into(),
        source,
    })
}

fn row(stats: &HistoryStats, class: &str) -> StatsRow {
    stats
        .rows
        .iter()
        .find(|r| r.class == class)
        .unwrap_or_else(|| panic!("no {class} row in {stats:?}"))
        .clone()
}

#[test]
fn stats_medians_by_class_over_merged_tasks() {
    use Size::{L, M, S};
    use TaskOutcome::*;
    let mut lines = Vec::new();
    // S: four merged (an even count: the lower middle value), one cancelled after a
    // session, and one never dispatched (a plan rejected at the gate), which is left out.
    let mut a1 = merged("r1", "a1", S, (8, 5, 1_500, 60));
    a1.diff.as_mut().unwrap().removed = 2;
    a1.bounces.done = 1;
    a1.size_check = check(S, Some(S), DeciderSource::Decider);
    let mut a2 = merged("r1", "a2", S, (30, 7, 180_400, 120));
    a2.bounces.check = 1;
    a2.bounces.review = 1;
    a2.size_check = check(S, None, DeciderSource::Fallback);
    let mut a3 = merged("r1", "a3", S, (20, 9, 9_999, 180));
    a3.outcome = MergedWithoutApproval;
    let a4 = merged("r2", "a4", S, (40, 11, 2_000_000, 240));
    let mut a5 = task("r2", "a5", S, false, Cancelled, 1);
    a5.bounces.proof = 1;
    a5.tool_calls = 1_000;
    let mut a6 = task("r2", "a6", S, false, Unfinished, 0);
    a6.bounces.merge = 5;
    a6.size_check = check(S, Some(S), DeciderSource::Decider);
    // M: three merged (an odd count: the middle value); one raised from S by the
    // cross-check.
    let mut b1 = merged("r1", "b1", M, (100, 30, 1_150_000, 1_800));
    b1.planned_size = S;
    b1.size_check = check(S, Some(M), DeciderSource::Decider);
    let b2 = merged("r1", "b2", M, (50, 10, 999, 600));
    let b3 = merged("r2", "b3", M, (70, 20, 50_000, 1_230));
    // hub: one blocked after two sessions (no merged task: `-`), one never dispatched.
    let mut c1 = task("r2", "c1", M, true, Blocked, 2);
    c1.bounces.review = 2;
    let c2 = task("r2", "c2", S, true, Unfinished, 0);
    // A task blocked as L belongs to no row.
    let d1 = task("r2", "d1", L, false, Blocked, 1);
    for record in [a1, a2, a3, a4, a5, a6, b1, b2, b3, c1, c2, d1] {
        lines.push(HistoryLine::Task(record));
    }
    lines.push(HistoryLine::Run(run("r1", 5, 1)));
    lines.push(HistoryLine::Run(run("r2", 2, 0)));

    let stats = aggregate(&lines, Path::new(PATH));
    assert_eq!(stats.path, Path::new(PATH));
    assert_eq!((stats.task_records, stats.run_records), (12, 2));
    assert_eq!(
        stats
            .rows
            .iter()
            .map(|r| r.class.as_str())
            .collect::<Vec<_>>(),
        ["S", "M", "hub"]
    );
    assert_eq!(
        row(&stats, "S"),
        StatsRow {
            class: "S".into(),
            tasks: 5,
            merged: 4,
            median_lines: Some(20),
            median_tool_calls: Some(7),
            median_tokens: Some(9_999),
            median_work_secs: Some(120),
            bounces: 4,
            reverted: 0,
        }
    );
    assert_eq!(
        row(&stats, "M"),
        StatsRow {
            class: "M".into(),
            tasks: 3,
            merged: 3,
            median_lines: Some(70),
            median_tool_calls: Some(20),
            median_tokens: Some(50_000),
            median_work_secs: Some(1_230),
            bounces: 0,
            reverted: 0,
        }
    );
    assert_eq!(
        row(&stats, "hub"),
        StatsRow {
            class: "hub".into(),
            tasks: 1,
            merged: 0,
            median_lines: None,
            median_tool_calls: None,
            median_tokens: None,
            median_work_secs: None,
            bounces: 2,
            reverted: 0,
        }
    );
    assert_eq!((stats.decider_calls, stats.decider_fallbacks), (7, 1));
    assert_eq!((stats.size_checked, stats.size_raised), (3, 1));
    assert!(stats.problems.is_empty());

    let want = [
        "history: /tmp/h/history.jsonl  (12 task records, 2 runs)",
        "CLASS  TASKS  MERGED  LINES  TOOL CALLS  TOKENS  WORK MIN  BOUNCES  REVERTED",
        "S      5      4       20     7           9.9k    2         4        0",
        "M      3      3       70     20          50k     21        0        0",
        "hub    1      0       -      -           -       -         2        0",
        "deciders: 7 calls, 1 fallback · size cross-check: 3 checked, 1 raised",
    ];
    assert_eq!(render(&stats), want.map(|l| format!("{l}\n")).concat());

    // No history at all: three empty rows, every median `-`.
    let empty = aggregate(&[], Path::new(PATH));
    let want = [
        "history: /tmp/h/history.jsonl  (0 task records, 0 runs)",
        "CLASS  TASKS  MERGED  LINES  TOOL CALLS  TOKENS  WORK MIN  BOUNCES  REVERTED",
        "S      0      0       -      -           -       -         0        0",
        "M      0      0       -      -           -       -         0        0",
        "hub    0      0       -      -           -       -         0        0",
        "deciders: 0 calls, 0 fallbacks · size cross-check: 0 checked, 0 raised",
    ];
    assert_eq!(render(&empty), want.map(|l| format!("{l}\n")).concat());

    // One record of each: singular words.
    let one = aggregate(
        &[
            HistoryLine::Task(merged("r", "t", S, (1, 1, 1, 1))),
            HistoryLine::Run(run("r", 1, 1)),
        ],
        Path::new(PATH),
    );
    let text = render(&one);
    assert!(
        text.starts_with("history: /tmp/h/history.jsonl  (1 task record, 1 run)\n"),
        "{text}"
    );
    assert!(
        text.ends_with("deciders: 1 call, 1 fallback · size cross-check: 0 checked, 0 raised\n"),
        "{text}"
    );
}

#[test]
fn tokens_are_shown_as_n_k_or_m() {
    for (n, shown) in [
        (0, "0"),
        (999, "999"),
        (1_000, "1.0k"),
        (9_999, "9.9k"),
        (10_000, "10k"),
        (180_400, "180k"),
        (999_999, "999k"),
        (1_000_000, "1.0M"),
        (1_150_000, "1.1M"),
        (12_345_678, "12.3M"),
    ] {
        assert_eq!(tokens(n), shown, "{n}");
    }
}

fn revert(run: &str, task: Option<&str>, n: u32) -> HistoryLine {
    HistoryLine::Revert(RevertRecord {
        v: HISTORY_VERSION,
        record_id: format!("revert/{n:0>40}"),
        at: 300,
        run_id: run.into(),
        task_id: task.map(str::to_string),
        reverted: format!("{:0>40}", n + 100),
        revert_commit: format!("{n:0>40}"),
    })
}

#[test]
fn reverted_counts_join_task_and_run_reverts() {
    use Size::{M, S};
    let work = (10, 1, 1, 60);
    let mut lines = vec![
        HistoryLine::Task(merged("ra", "t1", S, work)),
        HistoryLine::Task(merged("ra", "t2", S, work)),
        HistoryLine::Task(merged("ra", "t3", M, work)),
        HistoryLine::Task(merged("rb", "t1", S, work)),
        HistoryLine::Task(task("rb", "t2", M, false, TaskOutcome::Cancelled, 1)),
        HistoryLine::Run(run("ra", 0, 0)),
        HistoryLine::Run(run("rb", 0, 0)),
    ];
    let reverted = |lines: &[HistoryLine]| {
        let stats = aggregate(lines, Path::new(PATH));
        (
            row(&stats, "S").reverted,
            row(&stats, "M").reverted,
            row(&stats, "hub").reverted,
        )
    };
    assert_eq!(reverted(&lines), (0, 0, 0));
    // A task's own revert counts that task; a run's accept merge reverted counts every
    // merged task of that run (not its cancelled one).
    lines.push(revert("ra", Some("t1"), 1));
    lines.push(revert("rb", None, 2));
    assert_eq!(reverted(&lines), (2, 0, 0));
    // A task reverted on its own and with its run counts once.
    lines.push(revert("ra", Some("t3"), 3));
    lines.push(revert("ra", None, 4));
    assert_eq!(reverted(&lines), (3, 1, 0));
    // A revert of a task no record names counts nothing.
    lines.push(revert("rc", Some("t1"), 5));
    assert_eq!(reverted(&lines), (3, 1, 0));
}

#[test]
fn a_skipped_line_is_reported_once() {
    let mut stats = aggregate(&[], Path::new(PATH));
    stats.problems = vec!["line 3: bad".into(), "line 9: torn".into()];
    let text = render(&stats);
    let skipped: Vec<&str> = text.lines().filter(|l| l.contains("skipped")).collect();
    assert_eq!(skipped, ["history: 2 lines skipped: line 3: bad"], "{text}");
    assert!(
        text.ends_with("history: 2 lines skipped: line 3: bad\n"),
        "{text}"
    );
    stats.problems.truncate(1);
    assert!(render(&stats).ends_with("history: 1 line skipped: line 3: bad\n"));

    // The daemon's whole `stats` (revert detection, then the read): one bad line in the
    // file is one problem, though the file is read twice, and "skipped" is said once
    // (M8b.17 review, m6).
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    let good =
        serde_json::to_string(&HistoryLine::Task(merged("r", "t", Size::S, (1, 1, 1, 1)))).unwrap();
    let accepted = serde_json::to_string(&HistoryLine::Run(run("r", 0, 0))).unwrap();
    std::fs::write(&path, format!("{good}\nnot json\n{accepted}\n")).unwrap();
    let stats = summarise_at(dir.path(), &path);
    assert_eq!(stats.problems.len(), 1, "{:?}", stats.problems);
    assert!(stats.problems[0].contains("line 2"), "{:?}", stats.problems);
    assert_eq!((stats.task_records, stats.run_records), (1, 1));
    let text = render(&stats);
    assert_eq!(text.matches("skipped").count(), 1, "{text}");
    assert!(text.contains("history: 1 line skipped: "), "{text}");
}

/// `summarise` in `dir` with a `git` that does not exist.
fn summarise_at(dir: &Path, path: &Path) -> HistoryStats {
    let git = std::ffi::OsStr::new("/nonexistent/anthrex-test/git");
    let testing = config::Testing::default();
    let timeout = std::time::Duration::from_secs(5);
    crate::run::history_io::summarise(git, dir, path, 1_000, timeout, &testing)
}

/// M8b.17 review, m6: a history file that cannot be read is said as it is, never as
/// "1 line skipped".
#[test]
fn an_unreadable_history_is_not_a_skipped_line() {
    let dir = tempfile::tempdir().unwrap();
    // A directory reads as an error, whoever runs the test.
    let stats = summarise_at(dir.path(), dir.path());
    assert_eq!(stats.problems.len(), 1, "{:?}", stats.problems);
    let text = render(&stats);
    assert!(!text.contains("skipped"), "{text}");
    let want = format!("history: could not read {}: ", dir.path().display());
    assert!(text.lines().any(|l| l.starts_with(&want)), "{text}");
}

/// M9.9 review fixes, M5: a research or review task's record (`reported`) is neither
/// merged nor unmerged work: it is left out of the rows, though still a task record.
#[test]
fn reported_tasks_are_left_out_of_the_rows() {
    let mut research = task("r1", "q1", Size::S, false, TaskOutcome::Reported, 1);
    research.kind = TaskKind::Research;
    research.bounces.review = 2;
    let mut review = task("r1", "v1", Size::M, false, TaskOutcome::Reported, 1);
    review.kind = TaskKind::Review;
    let lines: Vec<HistoryLine> = [
        merged("r1", "a1", Size::S, (8, 5, 1_500, 60)),
        research,
        review,
    ]
    .into_iter()
    .map(HistoryLine::Task)
    .collect();
    let stats = aggregate(&lines, Path::new(PATH));
    assert_eq!(stats.task_records, 3);
    let s = row(&stats, "S");
    assert_eq!((s.tasks, s.merged, s.bounces), (1, 1, 0));
    let m = row(&stats, "M");
    assert_eq!((m.tasks, m.merged), (0, 0));
}

/// Milestone 9 decision 43: `role_route` lines (the orchestrator's, sub-planners',
/// scouts' and deciders' records, pre-run triage's included) change no aggregate.
#[test]
fn stats_ignores_role_route_lines() {
    let mut lines = vec![
        HistoryLine::Task(merged("r1", "a1", Size::S, (8, 5, 1_500, 60))),
        HistoryLine::Run(run("r1", 3, 1)),
    ];
    let without = aggregate(&lines, Path::new(PATH));
    let decider = crate::run::orch::roles::decider_record(
        None,
        ("5/1", "triage"),
        &[],
        (&route(), Vec::new()),
        proto::RoleRoutingInput::default(),
        300,
    );
    let mut planner = decider.clone();
    planner.record_id = "r1/planner/mail/1".into();
    planner.run_id = Some("r1".into());
    planner.role = proto::AgentRole::Planner;
    planner.outcome = Some(proto::RoleOutcome::Failed);
    for d in [decider, planner] {
        lines.push(HistoryLine::RoleRoute(d));
    }
    let with = aggregate(&lines, Path::new(PATH));
    assert_eq!(with, without);
    assert_eq!((with.task_records, with.run_records), (1, 1));
    assert_eq!((with.decider_calls, with.decider_fallbacks), (3, 1));
}

fn round(run: &str, n: u32, outcome: proto::RoundOutcome) -> HistoryLine {
    HistoryLine::Round(proto::RoundLine {
        v: HISTORY_VERSION,
        record_id: format!("{run}/round/{n}"),
        at: 1_700_000_000 + u64::from(n),
        run_id: run.into(),
        round: n,
        origin: proto::RoundOrigin::User,
        outcome,
        tasks: 1,
        merged: 1,
        calls: 9,
        minutes: 3,
    })
}

/// Milestone 9.3 (KG §2.6, design decision 31): `rounds: <total> · <iterated runs>`, after
/// the deciders' line. `<total>` is the `round` lines read; a run is iterated once a round
/// after its first has ended. Round 1's line is written when round 2 starts (task M9.3.4b's
/// ruling), so `r2`, whose round 2 is still open, has one line and is not yet counted. A
/// history with no `round` line (no run ever iterated) prints no line. The rows are
/// unchanged by round lines.
#[test]
fn stats_counts_rounds_and_iterated_runs() {
    use proto::RoundOutcome::{Completed, Rejected};
    let mut lines = vec![
        HistoryLine::Task(merged("r1", "a1", Size::S, (8, 5, 1_500, 60))),
        HistoryLine::Run(run("r1", 3, 1)),
    ];
    let without = aggregate(&lines, Path::new(PATH));
    assert_eq!((without.rounds, without.iterated_runs), (0, 0));
    let text = render(&without);
    assert!(!text.contains("rounds:"), "{text}");

    lines.extend([
        round("r1", 1, Completed),
        round("r1", 2, Rejected),
        round("r2", 1, Completed),
    ]);
    let with = aggregate(&lines, Path::new(PATH));
    assert_eq!((with.rounds, with.iterated_runs), (3, 1));
    assert_eq!(with.rows, without.rows);
    assert_eq!(
        (with.task_records, with.run_records),
        (without.task_records, without.run_records)
    );
    let text = render(&with);
    let deciders = text
        .lines()
        .position(|l| l.starts_with("deciders: "))
        .expect("the deciders' line");
    assert_eq!(
        text.lines().nth(deciders + 1),
        Some("rounds: 3 · 1"),
        "{text}"
    );
    assert_eq!(text.matches("rounds:").count(), 1, "{text}");
    assert_eq!(render(&without).lines().count() + 1, text.lines().count());

    // A second iterated run, and one with three rounds, count once each.
    lines.extend([round("r2", 2, Completed), round("r1", 3, Completed)]);
    let more = aggregate(&lines, Path::new(PATH));
    assert_eq!((more.rounds, more.iterated_runs), (5, 2));
    assert!(render(&more).contains("\nrounds: 5 · 2\n"));
}
