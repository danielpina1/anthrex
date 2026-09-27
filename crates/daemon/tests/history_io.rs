//! M8b.16: the diff measurement (decision 32) against real temporary repositories, and
//! `history.jsonl` on disk with its reconcile row (decision 33). M8b.17: revert
//! detection (decision 34) and `run stats`' blocking core (decision 35).

mod support;

use std::path::{Path, PathBuf};

use daemon::run::engine::{OpKind, OpResult};
use daemon::run::history_io::{
    append_line, contains_record, detect_reverts, fill_accepted_commit, measure_diff, read_history,
    summarise,
};
use daemon::run::journal::JournalLine;
use daemon::run::model::{PendingOp, Run};
use daemon::run::plan::{BuildContext, Preflight, build_run, parse_plan};
use daemon::run::reconcile::{Reconciled, reconcile};
use proto::{DiffStats, HISTORY_VERSION, HistoryLine, RevertRecord, RunRecord, TaskRecord};
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

fn revert(id: &str, n: u64) -> HistoryLine {
    HistoryLine::Revert(RevertRecord {
        v: HISTORY_VERSION,
        record_id: id.into(),
        at: n,
        run_id: "r".into(),
        task_id: None,
        reverted: "a".repeat(40),
        revert_commit: "b".repeat(40),
    })
}

fn file_lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
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
    // No file: no history, no problem.
    let (none, problems) = read_history(&dir.path().join("absent.jsonl"));
    assert!(none.is_empty() && problems.is_empty());
    // A record id is matched whole, never as a prefix.
    assert!(contains_record(&path, "a").unwrap());
    assert!(!contains_record(&path, "revert").unwrap());
    assert!(!contains_record(&dir.path().join("absent.jsonl"), "a").unwrap());
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
        id: "history-5a1e".into(),
        wt_dir: PathBuf::from("/tmp/nowhere-wt"),
        data_dir: data.join("runs").join("history-5a1e"),
        config: &config,
        now: 1_000,
        yes: true,
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

fn run_line(outcome: &str) -> HistoryLine {
    HistoryLine::Run(RunRecord {
        v: HISTORY_VERSION,
        record_id: "r".into(),
        at: 1,
        run_id: "r".into(),
        goal: "g".into(),
        path: None,
        triage: None,
        profile_source: None,
        outcome: outcome.into(),
        base_branch: "main".into(),
        accepted_commit: None,
        tasks: 1,
        usage: None,
    })
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

/// A merged task record of `run` whose merge commit is `merge`.
fn merged_task(run: &str, task: &str, merge: &str) -> HistoryLine {
    // Built through JSON so this test does not spell every field of the record.
    let line = serde_json::json!({
        "type": "task", "v": 1, "record_id": format!("{run}/{task}"), "at": 1,
        "run_id": run, "task_id": task, "path": null, "kind": "code", "hub": false,
        "test_mode": "tdd", "planned_size": "S", "final_size": "S", "size_check": null,
        "route": {"runtime": "claude", "model": "m", "strength": "standard", "effort": "medium"},
        "review_routes": [], "outcome": "merged", "block": null,
        "diff": {"files": 1, "hunks": 1, "added": 1, "removed": 0}, "tool_calls": 1,
        "worker_usage": {"input": 1, "output": 1, "cache_read": 0, "cache_write": 0},
        "reviewer_usage": {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0},
        "decider_usage": {"input": 0, "output": 0, "cache_read": 0, "cache_write": 0},
        "phases": {"queued": 0, "preparing": 0, "working": 60, "proof": 0, "check": 0,
                   "review": 0, "merge": 0, "blocked": 0},
        "wall_secs": 60,
        "gates": {"proofs": 0, "proofs_failed": 0, "checks": 0, "checks_failed": 0,
                  "review_rounds": 0, "reviews_rejected": 0, "candidates_red": 0,
                  "generated_bounces": 0},
        "severities": {"critical": 0, "important": 0, "minor": 0},
        "bounces": {"done": 0, "proof": 0, "check": 0, "review": 0, "merge": 0},
        "failures": 0, "stalls": 0, "budget_exceeded": 0, "conflicts": 0, "max_rung": 0,
        "sessions": 1, "done_signal": null, "merge_commit": merge,
    });
    HistoryLine::Task(serde_json::from_value::<TaskRecord>(line).unwrap())
}

/// An accepted run record of `run`, `at`, whose accept merge is `accepted`.
fn accepted_run(run: &str, at: u64, accepted: &str) -> HistoryLine {
    HistoryLine::Run(RunRecord {
        record_id: run.into(),
        run_id: run.into(),
        at,
        accepted_commit: Some(accepted.into()),
        ..match run_line("accepted") {
            HistoryLine::Run(r) => r,
            _ => unreachable!(),
        }
    })
}

const DAY: u64 = 86_400;
/// The test's `now`.
const NOW: u64 = 2_000_000_000;

/// The repository an accepted two-task run leaves, and its commits.
struct Accepted {
    repo: support::TempRepo,
    old: String,
    merges: [String; 2],
    accept: String,
}

/// `main` with an old commit (the accept of a run 91 days old), then run `r1`: tasks
/// `t1` and `t2` merged into its branch with `--no-ff` (two-parent merges, as the
/// engine's `commit-tree` makes), and the branch merged into `main` with `--no-ff`
/// (as `run accept` does).
fn accepted() -> Accepted {
    let repo = repo();
    let root = repo.root.clone();
    out(&root, &["branch", "-M", "main"]);
    let old = commit_file(&root, "old.txt", "old\n", "an old run's work");
    out(&root, &["checkout", "-q", "-b", "anthrex/r1"]);
    let mut merges = Vec::new();
    for task in ["t1", "t2"] {
        out(&root, &["checkout", "-q", "-b", task, "anthrex/r1"]);
        commit_file(&root, &format!("{task}.txt"), "work\n", task);
        out(&root, &["checkout", "-q", "anthrex/r1"]);
        out(&root, &["merge", "-q", "--no-ff", "--no-edit", task]);
        merges.push(head(&root));
    }
    out(&root, &["checkout", "-q", "main"]);
    out(
        &root,
        &["merge", "-q", "--no-ff", "--no-edit", "anthrex/r1"],
    );
    let accept = head(&root);
    Accepted {
        repo,
        old,
        merges: [merges[0].clone(), merges[1].clone()],
        accept,
    }
}

/// Every ref and what it points at, and `HEAD`.
fn refs(root: &Path) -> String {
    format!(
        "{}\n{}",
        out(root, &["for-each-ref", "--format=%(refname) %(objectname)"]),
        head(root)
    )
}

#[test]
fn detect_reverts_matches_merge_and_accept_commits() {
    let Accepted {
        repo,
        old,
        merges,
        accept,
    } = accepted();
    let root = &repo.root;
    let history = vec![
        merged_task("r1", "t1", &merges[0]),
        merged_task("r1", "t2", &merges[1]),
        accepted_run("r1", NOW - 10 * DAY, &accept),
        accepted_run("r0", NOW - 91 * DAY, &old),
    ];
    // Nothing reverted yet.
    assert_eq!(
        detect_reverts(real_git(), root, "main", &history, NOW, T),
        Ok(Vec::new())
    );
    // A task's merge (a two-parent merge needs `-m 1`), the run's accept merge, and the
    // 91-day-old run's accept, each reverted on `main`.
    out(root, &["revert", "-m", "1", "--no-edit", &merges[0]]);
    let task_revert = head(root);
    out(root, &["revert", "-m", "1", "--no-edit", &accept]);
    let run_revert = head(root);
    out(root, &["revert", "--no-edit", &old]);
    let before = refs(root);
    let found = detect_reverts(real_git(), root, "main", &history, NOW, T).unwrap();
    let want = vec![
        RevertRecord {
            v: HISTORY_VERSION,
            record_id: format!("revert/{task_revert}"),
            at: NOW,
            run_id: "r1".into(),
            task_id: Some("t1".into()),
            reverted: merges[0].clone(),
            revert_commit: task_revert.clone(),
        },
        RevertRecord {
            v: HISTORY_VERSION,
            record_id: format!("revert/{run_revert}"),
            at: NOW,
            run_id: "r1".into(),
            task_id: None,
            reverted: accept.clone(),
            revert_commit: run_revert.clone(),
        },
    ];
    assert_eq!(found, want);
    // It only reads: no ref and no HEAD moved.
    assert_eq!(refs(root), before);
    // Run again with those records in the history: nothing new.
    let mut again = history.clone();
    again.extend(found.into_iter().map(HistoryLine::Revert));
    assert_eq!(
        detect_reverts(real_git(), root, "main", &again, NOW, T),
        Ok(Vec::new())
    );
    // Another base branch is not this run's: nothing is looked for there.
    out(root, &["branch", "other", "main"]);
    assert_eq!(
        detect_reverts(real_git(), root, "other", &history, NOW, T),
        Ok(Vec::new())
    );
    // A run accepted into a branch that no longer exists: an error, not "nothing
    // reverted".
    let mut gone = history.clone();
    if let HistoryLine::Run(r) = &mut gone[2] {
        r.base_branch = "absent".into();
    }
    assert!(detect_reverts(real_git(), root, "absent", &gone, NOW, T).is_err());
}

/// Decision 34 at `run stats`: `summarise` appends each revert once, then counts it.
#[test]
fn summarise_appends_each_revert_once_and_counts_it() {
    let Accepted {
        repo,
        merges,
        accept,
        ..
    } = accepted();
    let root = &repo.root;
    let data = tempfile::tempdir().unwrap();
    let path = data.path().join("history.jsonl");
    for line in [
        merged_task("r1", "t1", &merges[0]),
        merged_task("r1", "t2", &merges[1]),
        accepted_run("r1", NOW - DAY, &accept),
    ] {
        append_line(&path, &line).unwrap();
    }
    let s_row = |stats: &proto::HistoryStats| {
        let row = stats.rows.iter().find(|r| r.class == "S").unwrap();
        (row.merged, row.reverted)
    };
    let stats = summarise(real_git(), root, &path, NOW, T);
    assert_eq!(s_row(&stats), (2, 0));
    assert_eq!(file_lines(&path).len(), 3);
    out(root, &["revert", "-m", "1", "--no-edit", &accept]);
    let stats = summarise(real_git(), root, &path, NOW, T);
    assert_eq!(s_row(&stats), (2, 2), "{stats:?}");
    assert!(stats.problems.is_empty(), "{:?}", stats.problems);
    assert_eq!(file_lines(&path).len(), 4);
    // A second `run stats` finds the same revert recorded and appends nothing.
    let stats = summarise(real_git(), root, &path, NOW, T);
    assert_eq!(s_row(&stats), (2, 2));
    assert_eq!(file_lines(&path).len(), 4);
}
