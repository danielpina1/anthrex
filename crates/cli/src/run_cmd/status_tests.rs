use super::*;
use proto::{
    AgentRole, AgentRoundInfo, Budget, Effort, Runtime, Spend, Strength, TaskKind, TaskState,
    TokenUsage,
};
use std::collections::BTreeMap;
use std::path::PathBuf;

fn route(runtime: Runtime, model: &str, effort: Effort) -> Route {
    Route {
        runtime,
        model: model.to_string(),
        strength: Strength::Standard,
        effort,
    }
}

fn round(window: u32) -> AgentRoundInfo {
    AgentRoundInfo {
        role: AgentRole::Worker,
        session: 1,
        round: 1,
        window_id: Some(window),
        route: route(Runtime::Claude, "", Effort::Low),
        session_id: None,
        started_at: 0,
        ended_at: None,
        tool_calls: 0,
        last_event: 0,
        turn_open: false,
        turns: 0,
        rate_limited: false,
        open_subagents: 0,
        denials: 0,
        usage: TokenUsage::default(),
    }
}

#[allow(clippy::too_many_arguments)]
fn task(
    id: &str,
    size: Size,
    hub: bool,
    mode: TestMode,
    state: TaskState,
    rung: u8,
    bounces: GateCounts,
    route: Route,
    windows: &[u32],
) -> TaskInfo {
    TaskInfo {
        id: id.to_string(),
        title: format!("Task {id}"),
        epic: None,
        kind: TaskKind::Code,
        size,
        hub,
        test_mode: mode,
        test_mode_reason: None,
        notes: vec![],
        owns: vec![],
        deps: vec![],
        implicit_deps: vec![],
        priority: 0,
        route,
        review_route: None,
        budget: Budget {
            tool_calls: 1,
            minutes: 1,
            tokens: None,
        },
        spent_session: Spend::default(),
        spent_total: Spend::default(),
        state,
        block: None,
        rung,
        failures: 0,
        bounces,
        stalls: 0,
        budget_exceeded: 0,
        conflicts: 0,
        branch: String::new(),
        worktree: PathBuf::new(),
        start_commit: None,
        head: None,
        test: None,
        red: None,
        done_signal: None,
        rounds: windows.iter().map(|w| round(*w)).collect(),
        reviews: vec![],
        last_check: None,
        last_proof: None,
        merge_commit: None,
        merged_without_approval: None,
        salvage_refs: vec![],
        on_critical_path: false,
        wave: 0,
        history: vec![],
    }
}

/// The Interfaces example's run.
pub(in crate::run_cmd) fn example() -> RunInfo {
    let review_1 = GateCounts {
        review: 1,
        ..GateCounts::default()
    };
    let check_3 = GateCounts {
        check: 3,
        ..GateCounts::default()
    };
    let none = GateCounts::default();
    let tasks = vec![
        task(
            "t1",
            Size::M,
            true,
            TestMode::Tdd,
            TaskState::Merged,
            0,
            none,
            route(Runtime::Claude, "claude-opus-5", Effort::High),
            &[4, 5],
        ),
        task(
            "t2",
            Size::M,
            false,
            TestMode::Tdd,
            TaskState::Review,
            1,
            review_1,
            route(Runtime::Codex, "", Effort::Medium),
            &[6, 9, 6],
        ),
        task(
            "t3",
            Size::S,
            false,
            TestMode::Check,
            TaskState::Queued,
            0,
            none,
            route(Runtime::Claude, "claude-sonnet-5", Effort::Low),
            &[],
        ),
        task(
            "t4",
            Size::S,
            false,
            TestMode::None,
            TaskState::Blocked,
            3,
            check_3,
            route(Runtime::Claude, "claude-sonnet-5", Effort::Low),
            &[7],
        ),
    ];
    RunInfo {
        run_id: "add-reset-3f9a".into(),
        goal: "Add password reset".into(),
        project: PathBuf::from("/r/p"),
        root: PathBuf::from("/r/p"),
        state: RunState::Running,
        paused_from: None,
        halted_reason: None,
        approved_by: Some("user".into()),
        base_branch: "main".into(),
        base_sha: "1a2b3c4d5e6f".into(),
        run_branch: "anthrex/add-reset-3f9a/integration".into(),
        run_head: "ffff".into(),
        base_moved: None,
        revision: 57,
        max_writers: 3,
        max_readers: 3,
        max_bounces: 3,
        writers_busy: 2,
        readers_busy: 1,
        unverified: false,
        worker_sandbox: true,
        trusted_project: vec![],
        rate_limits: BTreeMap::new(),
        tasks,
        critical_path: vec![],
        attention: vec!["t4 blocked (mis_sized): check failed 3 times".into()],
        report_path: PathBuf::from(
            "/Users/me/Library/Application Support/anthrex/runs/add-reset-3f9a/REPORT.md",
        ),
        outcome: None,
        created_at: 100,
    }
}

const EXAMPLE: &str = "\
add-reset-3f9a  running  2/4 merged  base main@1a2b3c4  writers 2/3  readers 1/3  rev 57
  goal: Add password reset
  report: /Users/me/Library/Application Support/anthrex/runs/add-reset-3f9a/REPORT.md
  ID   SIZE MODE   STATE          RUNG BOUNCES        ROUTE                          WINDOWS
  t1   M◆   tdd    merged         0    -              claude claude-opus-5 high      4 5
  t2   M    tdd    review         1    review 1       codex (default) medium         6 9
  t3   S    check  queued         0    -              claude claude-sonnet-5 low
  t4   S    none   blocked        3    check 3        claude claude-sonnet-5 low     7
  attention: t4 blocked (mis_sized): check failed 3 times
";

#[test]
fn status_text_matches_the_layout() {
    // The example's header says `2/4 merged` while its table shows one merged task
    // (t1); the count is the table's, so the fixture's header reads `1/4` (recorded
    // in the brief's M8a.23 notes). With a second task merged it reads `2/4`.
    let mut run = example();
    let expected = EXAMPLE.replacen("2/4 merged", "1/4 merged", 1);
    assert_eq!(run_block(&run), expected);

    run.tasks[2].state = TaskState::Merged;
    let text = run_block(&run);
    assert_eq!(text.lines().next(), EXAMPLE.lines().next(), "{text}");

    // Newest first, one block each.
    let mut older = example();
    older.run_id = "older-0000".into();
    older.created_at = 50;
    let both = render(&[older.clone(), example()]);
    assert!(both.starts_with("add-reset-3f9a  "), "{both}");
    assert!(both.contains("\nolder-0000  running"), "{both}");
}

#[test]
fn paused_and_halted_lines() {
    let mut run = example();
    run.state = RunState::Paused;
    run.paused_from = Some(RunState::Running);
    let text = run_block(&run);
    assert!(
        text.starts_with("add-reset-3f9a  paused (from running)  1/4 merged  "),
        "{text}"
    );
    assert!(!text.contains("halted"), "{text}");

    let mut run = example();
    run.state = RunState::Halted;
    run.halted_reason = Some("refs/heads/main was rewritten".into());
    let text = run_block(&run);
    let lines: Vec<&str> = text.lines().collect();
    assert!(
        lines[0].starts_with("add-reset-3f9a  halted  1/4 merged  "),
        "{text}"
    );
    assert_eq!(lines[1], "  halted: refs/heads/main was rewritten");
    assert_eq!(lines[2], "  goal: Add password reset");
}

#[test]
fn base_moved_prompt_lists_the_commits() {
    let commits: Vec<String> = (0..53)
        .map(|i| format!("{i:07x} Ann: change {i}"))
        .collect();
    let info = BaseMovedInfo {
        from: "1111111aaaa".into(),
        to: "2222222bbbb".into(),
        commits,
        total: 53,
    };
    let text = base_moved_listing("main", &info);
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(
        lines[0],
        "main moved since the run started (1111111..2222222, 53 commits):"
    );
    assert_eq!(lines.len(), 1 + 50 + 1, "{text}");
    assert_eq!(lines[1], "  0000000 Ann: change 0");
    assert_eq!(lines[50], "  0000031 Ann: change 49");
    assert_eq!(lines[51], "  … and 3 more");
    assert_eq!(
        base_moved_question("main", &info),
        "merge onto main at 2222222 including these commits? [y/N] "
    );

    // The daemon's own cap: 50 listed of 120.
    let info = BaseMovedInfo {
        commits: info.commits[..50].to_vec(),
        total: 120,
        ..info
    };
    let text = base_moved_listing("main", &info);
    assert!(text.ends_with("  … and 70 more\n"), "{text}");

    // Under the cap: no "more" line.
    // Under the cap: no "more" line; one commit is singular (ruling T23-minors, M7).
    let info = BaseMovedInfo {
        commits: vec!["abc1234 Bo: one".into()],
        total: 1,
        ..info
    };
    assert_eq!(
        base_moved_listing("main", &info),
        "main moved since the run started (1111111..2222222, 1 commit):\n  abc1234 Bo: one\n"
    );
}

#[test]
fn bounces_column_text() {
    assert_eq!(bounces_text(&GateCounts::default()), "-");
    let counts = GateCounts {
        done: 1,
        proof: 0,
        check: 2,
        review: 3,
        merge: 1,
    };
    assert_eq!(bounces_text(&counts), "done 1, check 2, review 3, merge 1");
    let counts = GateCounts {
        proof: 4,
        ..GateCounts::default()
    };
    assert_eq!(bounces_text(&counts), "proof 4");
}

/// Ruling T23-minors, M4: a cell as wide as its column or wider still ends in a space,
/// so it never runs into the next one.
#[test]
fn long_cells_keep_a_space() {
    let mut run = example();
    run.tasks[3].bounces = GateCounts {
        done: 1,
        check: 2,
        review: 3,
        ..GateCounts::default()
    };
    run.tasks[3].route.model = "claude-sonnet-5-with-a-very-long-name".into();
    run.tasks[3].id = "t-long".into();
    let text = run_block(&run);
    let row = text.lines().find(|l| l.contains("t-long")).unwrap();
    assert_eq!(
        row,
        "  t-long S    none   blocked        3    done 1, check 2, review 3 claude claude-sonnet-5-with-a-very-long-name low 7"
    );
    // Cells that fit keep the spec's columns exactly.
    assert_eq!(
        run_block(&example()),
        EXAMPLE.replacen("2/4 merged", "1/4 merged", 1)
    );
}
