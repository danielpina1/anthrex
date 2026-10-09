//! M8b.16: the diff measurement (decision 32) against real temporary repositories, and
//! `history.jsonl` on disk with its reconcile row (decision 33). M8b.17's revert
//! detection is in `history_reverts.rs`. Milestone 9 decision 43's `role_route` lines at
//! the end.

mod support;

use std::path::{Path, PathBuf};

use daemon::run::engine::{OpKind, OpResult};
use daemon::run::history_io::{
    append_line, append_once, contains_record, fill_accepted_commit, measure_diff, read_history,
};
use daemon::run::journal::JournalLine;
use daemon::run::model::{PendingOp, Run};
use daemon::run::plan::{BuildContext, Preflight, build_run, parse_plan};
use daemon::run::reconcile::{Reconciled, reconcile};
use proto::{DiffStats, HistoryLine};
use support::history::{file_lines, revert, run_line};
use support::run_git::{T, commit_file, head, out, real_git, repo, write};

/// Lines 1 to 10 of a text file, `changed` ones replaced.
fn lines(changed: &[usize]) -> String {
    (1..=10)
        .map(|n| {
            if changed.contains(&n) {
                format!("changed {n}\n")
            } else {
                format!("line {n}\n")
            }
        })
        .collect()
}

#[test]
fn measure_diff_counts_files_hunks_and_lines() {
    let repo = repo();
    let root = &repo.root;
    let from = commit_file(root, "a.txt", &lines(&[]), "a");
    // Two hunks in a.txt (lines 2 and 9), one new file of three lines, a binary file.
    write(root, "a.txt", &lines(&[2, 9]));
    write(root, "b.txt", "one\ntwo\nthree\n");
    std::fs::write(root.join("bin.dat"), [0u8, 1, 2, 0, 255, 0]).unwrap();
    out(root, &["add", "a.txt", "b.txt", "bin.dat"]);
    out(root, &["commit", "-q", "-m", "change"]);
    let to = head(root);
    let want = DiffStats {
        files: 3,
        hunks: 3,
        added: 5,
        removed: 2,
    };
    assert_eq!(
        measure_diff(real_git(), root, &from, &to, false, T),
        Ok(want)
    );
    // From an ancestor, three dots measure the same.
    assert_eq!(
        measure_diff(real_git(), root, &from, &to, true, T),
        Ok(want)
    );
    // Nothing between a commit and itself.
    assert_eq!(
        measure_diff(real_git(), root, &to, &to, false, T),
        Ok(DiffStats::default())
    );
    // A commit that does not exist is an error, not an empty diff.
    let missing = "0".repeat(40);
    assert!(measure_diff(real_git(), root, &missing, &to, false, T).is_err());
}

#[test]
fn measure_diff_of_a_merge_commit_is_the_tasks_contribution_after_a_hand_back() {
    let repo = repo();
    let root = &repo.root;
    let start = commit_file(root, "base.txt", "base\n", "base");
    out(root, &["branch", "task", &start]);
    // The run head moves on while the task works.
    let run_head = commit_file(root, "other.txt", "o1\no2\no3\no4\n", "another task");
    out(root, &["checkout", "-q", "task"]);
    commit_file(root, "t.txt", "t1\nt2\n", "the task");
    // The hand-back: the run head merged into the task's branch.
    out(root, &["merge", "-q", "--no-edit", "main"]);
    out(root, &["checkout", "-q", "main"]);
    out(root, &["merge", "-q", "--no-ff", "--no-edit", "task"]);
    let merge = head(root);
    assert_ne!(merge, run_head);
    let want = DiffStats {
        files: 1,
        hunks: 1,
        added: 2,
        removed: 0,
    };
    assert_eq!(
        measure_diff(real_git(), root, &run_head, &merge, false, T),
        Ok(want)
    );
}

#[test]
fn append_is_one_line_and_fsynced() {
    let dir = tempfile::tempdir().unwrap();
    // The repository's data directory may not exist yet.
    let path = dir.path().join("repos").join("x").join("history.jsonl");
    append_line(&path, &revert("revert/1", 1)).unwrap();
    append_line(&path, &revert("revert/2", 2)).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.ends_with('\n'), "{text}");
    let lines = file_lines(&path);
    assert_eq!(lines.len(), 2, "{text}");
    let back: Vec<HistoryLine> = lines
        .iter()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(back, vec![revert("revert/1", 1), revert("revert/2", 2)]);
}

#[test]
fn read_history_keeps_the_last_line_per_record_id_and_skips_a_torn_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    for line in [revert("a", 1), revert("b", 2), revert("a", 3)] {
        append_line(&path, &line).unwrap();
    }
    // A line that does not parse in the middle, then a torn last line.
    let mut text = std::fs::read_to_string(&path).unwrap();
    text.push_str("not json\n");
    text.push_str(&serde_json::to_string(&revert("c", 4)).unwrap());
    text.push('\n');
    text.push_str(r#"{"type":"revert","v":1,"record_id":"d""#);
    std::fs::write(&path, &text).unwrap();
    let (lines, problems) = read_history(&path);
    assert_eq!(lines, vec![revert("b", 2), revert("a", 3), revert("c", 4)]);
    assert_eq!(problems.len(), 2, "{problems:?}");
    assert!(problems[0].contains("line 4"), "{problems:?}");
    assert!(problems[1].contains("line 6"), "{problems:?}");
    // M8b.17 review, m6: `run stats` says "skipped" itself; the problem does not.
    assert!(!problems[0].contains("skipped"), "{problems:?}");
    // No file: no history, no problem.
    let (none, problems) = read_history(&dir.path().join("absent.jsonl"));
    assert!(none.is_empty() && problems.is_empty());
    // A record id is matched whole, never as a prefix.
    assert!(contains_record(&path, "a").unwrap());
    assert!(!contains_record(&path, "revert").unwrap());
    assert!(!contains_record(&dir.path().join("absent.jsonl"), "a").unwrap());
}

/// A design phase's line (milestone 9.6 decision 32).
fn phase_line() -> HistoryLine {
    HistoryLine::Phase(proto::PhaseRecord {
        v: proto::HISTORY_VERSION,
        record_id: "r/phase/1/brainstorming".into(),
        at: 5,
        run_id: "r".into(),
        round: 1,
        phase: "brainstorming".into(),
        secs: 60,
        agents: Vec::new(),
        gate_versions: 1,
        disputed: 0,
    })
}

/// Milestone 9.6 decision 32 (task M9.6.13): `HISTORY_VERSION` stays 5, because a
/// reader that does not know the `phase` line skips it with a problem and keeps every
/// other line. Pins `read_history`'s existing behaviour: 9.5's line types do not
/// include `phase` (an enum of them refuses it), and a line of a type the reader does
/// not know is dropped with one problem naming its line, the rest read.
#[test]
fn older_readers_skip_a_phase_line_with_a_problem() {
    #[derive(Debug, serde::Deserialize)]
    #[serde(tag = "type", rename_all = "snake_case")]
    #[allow(dead_code)]
    enum NineFiveLine {
        Task(serde_json::Value),
        Run(serde_json::Value),
        Revert(serde_json::Value),
        RoleRoute(serde_json::Value),
        Tier(serde_json::Value),
        Flaky(serde_json::Value),
        Bisect(serde_json::Value),
        Stage(serde_json::Value),
        Round(serde_json::Value),
    }
    assert_eq!(proto::HISTORY_VERSION, 5);
    let phase = serde_json::to_string(&phase_line()).unwrap();
    let older = serde_json::from_str::<NineFiveLine>(&phase).unwrap_err();
    assert!(
        older.to_string().contains("unknown variant `phase`"),
        "{older}"
    );
    assert!(
        serde_json::from_str::<NineFiveLine>(&serde_json::to_string(&revert("a", 1)).unwrap())
            .is_ok()
    );

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    for line in [revert("a", 1), phase_line(), revert("b", 2)] {
        append_line(&path, &line).unwrap();
    }
    let (lines, problems) = read_history(&path);
    assert_eq!(lines, vec![revert("a", 1), phase_line(), revert("b", 2)]);
    assert!(problems.is_empty(), "{problems:?}");
    // What a reader without the `phase` type sees: a line of a type it does not know.
    let text = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        text.replace(r#""type":"phase""#, r#""type":"later_kind""#),
    )
    .unwrap();
    let (lines, problems) = read_history(&path);
    assert_eq!(lines, vec![revert("a", 1), revert("b", 2)]);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("line 2"), "{problems:?}");
    assert!(
        problems[0].contains("unknown variant `later_kind`"),
        "{problems:?}"
    );
}

/// A run with one task whose record is being appended, as the engine leaves it.
fn run_appending(data: &Path, history: &Path) -> (Run, OpKind) {
    let plan = parse_plan(
        "goal = \"History\"\n[[task]]\nid = \"t1\"\ntitle = \"T\"\nsize = \"S\"\nowns = [\"a\"]\nbrief = \"b\"\nacceptance = [\"a\"]\n",
    )
    .unwrap();
    let pre = Preflight {
        root: PathBuf::from("/tmp/nowhere"),
        project: PathBuf::from("/tmp/nowhere"),
        git_common_dir: PathBuf::from("/tmp/nowhere/.git"),
        base_branch: "main".into(),
        base_sha: "b".repeat(40),
        protected_files: Vec::new(),
    };
    let config = config::Orchestrator::default();
    let ctx = BuildContext {
        tuning: Default::default(),
        id: "history-5a1e".into(),
        wt_dir: PathBuf::from("/tmp/nowhere-wt"),
        data_dir: data.join("runs").join("history-5a1e"),
        config: &config,
        models: daemon::run::model_roles::RunModels::resolve(&config.roles, None),
        models_log: Vec::new(),
        testing: &config::Testing::default(),
        now: 1_000,
        yes: true,
        delivery: &config::Delivery::default(),
    };
    let mut run = build_run(plan, pre, ctx).unwrap();
    assert!(
        run.tasks.iter().all(|t| t.rounds.is_empty()),
        "no session to kill"
    );
    let kind = OpKind::AppendHistory {
        path: history.to_path_buf(),
        record_id: "history-5a1e/t1".into(),
        line: Box::new(revert("history-5a1e/t1", 9)),
    };
    run.pending_ops.insert(
        7,
        PendingOp {
            op: 7,
            task_id: Some("t1".into()),
            kind: kind.clone(),
            lane: None,
        },
    );
    (run, kind)
}

#[test]
fn history_append_is_reconciled_exactly_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    let (run, kind) = run_appending(dir.path(), &path);
    let journal = vec![JournalLine::Intent { op: 7, kind }];
    let git = std::ffi::OsStr::new("/nonexistent/anthrex-test/git");
    let answer = |run: &Run| reconcile(git, run, &journal, &[], T).ops;
    // The daemon died before the line was written: it is appended again, once.
    assert_eq!(answer(&run), vec![(7, Reconciled::NotStarted)]);
    append_line(&path, &revert("history-5a1e/t1", 9)).unwrap();
    // It died after: the result is replayed, and nothing is appended again.
    assert_eq!(
        answer(&run),
        vec![(7, Reconciled::Replay(OpResult::HistoryAppended))]
    );
    let lines = file_lines(&path);
    assert_eq!(lines.len(), 1, "{lines:?}");
    // A record of another id does not count.
    let other = dir.path().join("other.jsonl");
    append_line(&other, &revert("history-5a1e/t10", 9)).unwrap();
    let (run, kind) = run_appending(dir.path(), &other);
    let journal = vec![JournalLine::Intent { op: 7, kind }];
    let ops = reconcile(git, &run, &journal, &[], T).ops;
    assert_eq!(ops, vec![(7, Reconciled::NotStarted)]);
}

/// Milestone 9.6 ruling T13-4: a phase approved again after a back is appended under
/// its own id (`…/v<n>`), so a restart with that append pending writes it although the
/// first approval's line is there; the refit then counts the latest approval only.
#[test]
fn a_reapproved_phase_pending_at_a_restart_is_appended_and_counted_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    let agent = |calls: u32| proto::PhaseAgent {
        role: proto::AgentRole::DocReviewer,
        route: proto::Route {
            runtime: proto::Runtime::Codex,
            model: "gpt-6".into(),
            effort: proto::Effort::HIGH,
        },
        calls,
        tokens: 0,
        outcome: "ok".into(),
        secs: 240,
        sessions: 1,
    };
    let record = |run: u32, n: u32, calls: u32| {
        HistoryLine::Phase(proto::PhaseRecord {
            v: proto::HISTORY_VERSION,
            record_id: format!("history-{run}/phase/1/specifying/v{n}"),
            at: u64::from(run) * 10 + u64::from(n),
            run_id: format!("history-{run}"),
            round: 1,
            phase: "specifying".into(),
            secs: 60,
            agents: vec![agent(calls)],
            gate_versions: n,
            disputed: 0,
        })
    };
    for run in 0..30 {
        append_line(&path, &record(run, 1, 2)).unwrap();
        append_line(&path, &record(run, 2, 12)).unwrap();
    }
    append_line(&path, &record(30, 1, 2)).unwrap();
    // Run 30's re-approval was pending when the daemon stopped.
    let (mut run, _) = run_appending(dir.path(), &path);
    let kind = OpKind::AppendHistory {
        path: path.clone(),
        record_id: "history-30/phase/1/specifying/v2".into(),
        line: Box::new(record(30, 2, 12)),
    };
    run.pending_ops.get_mut(&7).unwrap().kind = kind.clone();
    let journal = vec![JournalLine::Intent { op: 7, kind }];
    let git = std::ffi::OsStr::new("/nonexistent/anthrex-test/git");
    let ops = reconcile(git, &run, &journal, &[], T).ops;
    assert_eq!(ops, vec![(7, Reconciled::NotStarted)], "appended again");
    append_line(&path, &record(30, 2, 12)).unwrap();
    let (lines, problems) = read_history(&path);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(lines.len(), 62, "both approvals of every run");
    let cfg = config::Orchestrator::default();
    let (file, _) = daemon::run::refit::refit(&lines, &proto::TuningFile::default(), &cfg, 1);
    let fit = &file.budgets["doc_review"];
    // The 31 latest approvals at 12 calls (× 250% = 30); the first ones (2 calls) would
    // have pulled the median to the floor.
    assert_eq!((fit.samples, fit.tool_calls), (31, 30));
}

/// Decision 33: the driver fills an accepted run's `accepted_commit` from the base
/// branch; nothing else is touched, and a failed read leaves it unset.
#[test]
fn an_accepted_run_record_gets_the_base_head() {
    let repo = repo();
    let root = &repo.root;
    out(root, &["branch", "-M", "main"]);
    let base = head(root);
    let HistoryLine::Run(filled) = fill_accepted_commit(real_git(), root, run_line("accepted"), T)
    else {
        panic!("a run line");
    };
    assert_eq!(filled.accepted_commit, Some(base));
    for outcome in ["discarded", "failed", "complete"] {
        let line = fill_accepted_commit(real_git(), root, run_line(outcome), T);
        assert_eq!(line, run_line(outcome), "{outcome}");
    }
    let revert = revert("x", 1);
    assert_eq!(
        fill_accepted_commit(real_git(), root, revert.clone(), T),
        revert
    );
    let nowhere = Path::new("/nonexistent/anthrex-test");
    let line = fill_accepted_commit(real_git(), nowhere, run_line("accepted"), T);
    assert_eq!(line, run_line("accepted"));
}

/// M8b.17 review, m6: a file that cannot be read is one problem saying so, not a line.
#[test]
fn an_unreadable_history_is_one_problem_of_its_own() {
    let dir = tempfile::tempdir().unwrap();
    // A directory reads as an error, whoever runs the test.
    let (lines, problems) = read_history(dir.path());
    assert!(lines.is_empty());
    assert_eq!(problems.len(), 1, "{problems:?}");
    let want = format!(
        "{}{}: ",
        daemon::run::history_io::UNREADABLE,
        dir.path().display()
    );
    assert!(problems[0].starts_with(&want), "{problems:?}");
}

/// M8b.17 review, m7: a torn last line (a crash or a full disk mid-append) is ended
/// before the next record, so it cannot swallow that record.
#[test]
fn an_append_after_a_torn_last_line_starts_a_new_line() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    std::fs::write(&path, r#"{"type":"revert","v":1,"record_id":"torn""#).unwrap();
    append_line(&path, &revert("revert/1", 1)).unwrap();
    let (lines, problems) = read_history(&path);
    assert_eq!(lines, vec![revert("revert/1", 1)]);
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("line 1"), "{problems:?}");
    // A file that ends in a newline gets no blank line.
    append_line(&path, &revert("revert/2", 2)).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(file_lines(&path).len(), 3, "{text}");
    assert!(!text.contains("\n\n"), "{text}");
    // An empty file gets no leading newline either.
    let empty = dir.path().join("empty.jsonl");
    std::fs::write(&empty, "").unwrap();
    append_line(&empty, &revert("revert/3", 3)).unwrap();
    assert!(std::fs::read_to_string(&empty).unwrap().starts_with('{'));
}

/// A pre-run triage record, finished with `outcome`.
fn role_line(n: u64, outcome: proto::RoleOutcome) -> HistoryLine {
    let route = proto::Route {
        runtime: proto::Runtime::Claude,
        model: "m".into(),
        effort: proto::Effort::LOW,
    };
    let input = proto::RoleRoutingInput::default();
    let session = format!("{n}/1");
    let mut d = daemon::run::orch::roles::decider_record(
        None,
        (&session, "triage"),
        &[],
        (&route, Vec::new()),
        input,
        n,
    );
    daemon::run::orch::roles::finish(&mut d, outcome, None);
    HistoryLine::RoleRoute(d)
}

/// Milestone 9 decision 43: a `role_route` line is written once per record id, by the
/// engine's `AppendHistory` (reconciled after a restart as any line is) and by the
/// driver's own append of pre-run triage's; the reader keeps one line per record.
#[test]
fn role_route_append_is_idempotent_by_record_id() {
    use proto::RoleOutcome::{Fallback, Interrupted};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("repos").join("x").join("history.jsonl");
    let line = role_line(7, Fallback);
    assert!(append_once(&path, &line).unwrap(), "written");
    assert!(!append_once(&path, &line).unwrap(), "held already");
    assert_eq!(file_lines(&path).len(), 1);
    // The engine's append of a run's record, across a restart.
    let (mut run, _) = run_appending(dir.path(), &path);
    let other = role_line(8, Interrupted);
    let HistoryLine::RoleRoute(d) = &other else {
        unreachable!()
    };
    let kind = OpKind::AppendHistory {
        path: path.clone(),
        record_id: d.record_id.clone(),
        line: Box::new(other.clone()),
    };
    run.pending_ops.insert(
        7,
        PendingOp {
            op: 7,
            task_id: None,
            kind: kind.clone(),
            lane: None,
        },
    );
    let journal = vec![JournalLine::Intent { op: 7, kind }];
    let git = std::ffi::OsStr::new("/nonexistent/anthrex-test/git");
    let answer = |run: &Run| reconcile(git, run, &journal, &[], T).ops;
    assert_eq!(answer(&run), vec![(7, Reconciled::NotStarted)]);
    append_line(&path, &other).unwrap();
    assert_eq!(
        answer(&run),
        vec![(7, Reconciled::Replay(OpResult::HistoryAppended))]
    );
    // A record written twice anyway is read once, its last line kept.
    append_line(&path, &other).unwrap();
    let (lines, problems) = read_history(&path);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(lines, vec![line, other]);
}

/// Decision 43: a version-1 history (M8b's lines, with no `routing_decisions` and no
/// `role_route` line) still reads, and so does a run persisted before any role record
/// (M8b's `run.json`) and an orchestrator record made before its routing snapshot.
#[test]
fn version_1_history_and_an_old_run_json_still_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("history.jsonl");
    let mut task = serde_json::to_value(support::history::merged_task("r", "t1", "c1")).unwrap();
    task.as_object_mut().unwrap().remove("routing_decisions");
    let mut run_record = serde_json::to_value(run_line("accepted")).unwrap();
    run_record["v"] = 1.into();
    let mut revert_record = serde_json::to_value(revert("revert/1", 1)).unwrap();
    revert_record["v"] = 1.into();
    let text: String = [task, run_record, revert_record]
        .iter()
        .map(|v| format!("{v}\n"))
        .collect();
    assert!(!text.contains("routing_decisions") && !text.contains("role_route"));
    std::fs::write(&path, &text).unwrap();
    append_line(&path, &role_line(9, proto::RoleOutcome::Completed)).unwrap();
    let (lines, problems) = read_history(&path);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(lines.len(), 4, "{lines:#?}");
    let HistoryLine::Task(t) = &lines[0] else {
        panic!("{lines:#?}")
    };
    assert_eq!((t.v, t.routing_decisions.len()), (1, 0));
    assert!(matches!(lines[3], HistoryLine::RoleRoute(_)));

    let old: Run =
        serde_json::from_str(include_str!("../src/run/engine/tests/m8b_run.json")).unwrap();
    assert!(old.role_routing_decisions.is_empty());
    let record = daemon::run::orch::OrchestratorRecord::new(
        proto::Route {
            runtime: proto::Runtime::Claude,
            model: String::new(),
            effort: proto::Effort::HIGH,
        },
        1,
    );
    let mut value = serde_json::to_value(&record).unwrap();
    value.as_object_mut().unwrap().remove("routing");
    let back: daemon::run::orch::OrchestratorRecord = serde_json::from_value(value).unwrap();
    assert_eq!(back, record);
}
