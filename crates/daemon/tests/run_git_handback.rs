//! M8a.9: the conflict hand-back into a task worktree (decision 36 step 6, rulings
//! T11-N1 and T14-C1). Moved from `run_git_merge.rs` to keep both under AGENTS.md rule
//! 8's ~600 lines.

mod support;

use daemon::run::git::{HandBack, abort_merge, hand_back, prepare_worktree};
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, write, wt_dir};

/// Commits `path` on `branch` (created from the current `HEAD` if missing) and returns
/// to the branch `root` was on. Returns the new commit.
fn commit_on(root: &Path, branch: &str, path: &str, content: &str) -> String {
    let back = out(root, &["symbolic-ref", "--short", "HEAD"]);
    if try_git(root, &["rev-parse", "-q", "--verify", branch])
        .status
        .success()
    {
        out(root, &["checkout", "-q", branch]);
    } else {
        out(root, &["checkout", "-q", "-b", branch]);
    }
    let sha = commit_file(root, path, content, &format!("{branch}: {path}"));
    out(root, &["checkout", "-q", &back]);
    sha
}

fn parents(dir: &Path, commit: &str) -> Vec<String> {
    out(dir, &["rev-list", "--parents", "-n", "1", commit])
        .split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect()
}

/// A task worktree at `base` with `shared.txt`, and a task commit on it.
fn task_worktree(
    repo: &TempRepo,
    run: &str,
    file: &str,
    content: &str,
) -> (tempfile::TempDir, PathBuf) {
    let base = head(&repo.root);
    let (keep, wt) = wt_dir();
    let path = wt.join(format!("runs/{run}/t1"));
    prepare_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/t1"),
        &base,
        &path,
        T,
    )
    .unwrap();
    commit_file(&path, file, content, "task work");
    (keep, path)
}

#[test]
fn hand_back_leaves_markers_and_merge_head() {
    let repo = repo();
    commit_file(&repo.root, "shared.txt", "base\n", "shared");
    let (_keep, task) = task_worktree(&repo, "hb1", "shared.txt", "task\n");
    let run_head = commit_on(&repo.root, "anthrex/hb1/integration", "shared.txt", "run\n");

    let task_head = head(&task);
    let back = hand_back(real_git(), &task, &run_head, T).unwrap();
    assert_eq!(
        back,
        HandBack {
            onto: task_head.clone(),
            head: task_head,
            files: vec!["shared.txt".to_string()],
        }
    );
    assert_eq!(out(&task, &["rev-parse", "MERGE_HEAD"]), run_head);
    let text = std::fs::read_to_string(task.join("shared.txt")).unwrap();
    assert!(text.contains("<<<<<<<"), "{text}");
    assert!(text.contains(">>>>>>>"), "{text}");
}

#[test]
fn hand_back_that_is_clean_commits_the_merge() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "hb2", "b.txt", "task\n");
    let task_head = head(&task);
    let run_head = commit_on(&repo.root, "anthrex/hb2/integration", "a.txt", "run\n");

    let back = hand_back(real_git(), &task, &run_head, T).unwrap();
    assert!(back.files.is_empty(), "{back:?}");
    assert_eq!(back.onto, task_head);
    assert_eq!(back.head, head(&task));
    assert_eq!(parents(&task, "HEAD"), vec![task_head, run_head]);
    assert!(
        !try_git(&task, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .status
            .success()
    );
    assert_eq!(out(&task, &["status", "--porcelain"]), "");
    // Final fix batch F1b: the worktree stays detached; the engine moved its `HEAD`
    // and the task's branch to the merge.
    support::run_git::assert_detached(&task);
    assert_eq!(out(&repo.root, &["rev-parse", "anthrex/hb2/t1"]), back.head);
}

#[test]
fn hand_back_blocked_by_an_untracked_file_is_an_error() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "hb3", "b.txt", "task\n");
    let task_head = head(&task);
    write(&task, "a.txt", "the worker's own, untracked\n");
    let run_head = commit_on(&repo.root, "anthrex/hb3/integration", "a.txt", "run\n");

    let result = hand_back(real_git(), &task, &run_head, T);
    assert!(result.is_err(), "not a clean hand-back: {result:?}");
    assert_eq!(head(&task), task_head);
    assert_eq!(
        std::fs::read_to_string(task.join("a.txt")).unwrap(),
        "the worker's own, untracked\n"
    );
}

/// Ruling T11-N1(a), probe q1: a hand-back into a worktree still mid-merge from an
/// earlier conflicted hand-back is an error, not that earlier merge's files dressed up
/// as a new conflict; nothing in the worktree changes.
#[test]
fn hand_back_while_a_merge_is_in_progress_is_an_error() {
    let repo = repo();
    commit_file(&repo.root, "shared.txt", "base\n", "shared");
    let (_keep, task) = task_worktree(&repo, "hb4", "shared.txt", "task\n");
    let first = commit_on(&repo.root, "anthrex/hb4/integration", "shared.txt", "run\n");
    assert_eq!(
        hand_back(real_git(), &task, &first, T).unwrap().files,
        vec!["shared.txt"]
    );
    let newer = commit_on(&repo.root, "anthrex/hb4/integration", "other.txt", "more\n");

    let result = hand_back(real_git(), &task, &newer, T);
    let err = result.expect_err("a merge is already in progress");
    assert!(err.contains("already in progress"), "{err}");
    assert_eq!(out(&task, &["rev-parse", "MERGE_HEAD"]), first);
}

/// Ruling T11-N1(b): `abort_merge` undoes a conflicted hand-back (markers, index and
/// `MERGE_HEAD` all go back to the task's own commit) and is a no-op without one.
#[test]
fn abort_merge_undoes_a_conflicted_hand_back() {
    let repo = repo();
    commit_file(&repo.root, "shared.txt", "base\n", "shared");
    let (_keep, task) = task_worktree(&repo, "hb5", "shared.txt", "task\n");
    let task_head = head(&task);
    let run_head = commit_on(&repo.root, "anthrex/hb5/integration", "shared.txt", "run\n");
    hand_back(real_git(), &task, &run_head, T).unwrap();

    abort_merge(real_git(), &task, T).unwrap();
    assert_eq!(head(&task), task_head);
    assert!(
        !try_git(&task, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .status
            .success()
    );
    assert_eq!(out(&task, &["status", "--porcelain"]), "");
    assert_eq!(
        std::fs::read_to_string(task.join("shared.txt")).unwrap(),
        "task\n"
    );

    abort_merge(real_git(), &task, T).unwrap();
    assert_eq!(head(&task), task_head);
}

/// Ruling T14-C1: the hand-back reports the branch tip it merged onto. A commit the
/// worker made after its claim is that tip, so the engine can tell the merge was not
/// made onto the claimed head.
#[test]
fn hand_back_reports_the_tip_it_merged_onto() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "hb6", "b.txt", "task\n");
    let claimed = head(&task);
    let later = commit_file(&task, "c.txt", "after the claim\n", "post-claim work");
    let run_head = commit_on(&repo.root, "anthrex/hb6/integration", "a.txt", "run\n");

    let back = hand_back(real_git(), &task, &run_head, T).unwrap();
    assert!(back.files.is_empty(), "{back:?}");
    assert_eq!(back.onto, later);
    assert_ne!(back.onto, claimed);
    assert_eq!(back.head, head(&task));
    assert_eq!(parents(&task, &back.head), vec![later, run_head]);
}
