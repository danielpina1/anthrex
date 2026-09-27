//! M8b.16: the diff measurement (decision 32) against real temporary repositories, and
//! `history.jsonl` on disk with its reconcile row (decision 33). M8b.17's revert
//! detection is in `history_reverts.rs`.

mod support;

use std::path::{Path, PathBuf};

use daemon::run::engine::{OpKind, OpResult};
use daemon::run::history_io::{
    append_line, contains_record, fill_accepted_commit, measure_diff, read_history,
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
