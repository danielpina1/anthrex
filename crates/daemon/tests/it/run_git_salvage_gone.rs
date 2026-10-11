//! Milestone 9.11 decision 13 (A3): a task checkout the user deleted by hand still has
//! its repository in anthrex's data directory, whose `HEAD` and private objects hold
//! the worker's commits. `salvage_gone` imports them and keeps them under the salvage
//! ref before the repository is removed.

use crate::support;

use daemon::run::git::{default_repo_dir, prepare_worktree, remove_worktree, salvage_gone, sync};
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, wt_dir};

fn task_worktree(repo: &TempRepo, run: &str, task: &str) -> (tempfile::TempDir, PathBuf) {
    let base = head(&repo.root);
    let (keep, wt) = wt_dir();
    let path = wt.join(format!("runs/{run}/{task}"));
    prepare_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/{task}"),
        &base,
        &path,
        T,
    )
    .unwrap();
    (keep, path)
}

fn has_commit(root: &Path, id: &str) -> bool {
    try_git(root, &["cat-file", "-e", &format!("{id}^{{commit}}")])
        .status
        .success()
}

fn salvage_refs(root: &Path) -> String {
    out(
        root,
        &["for-each-ref", "--format=%(refname)", "refs/anthrex"],
    )
}

#[test]
fn a_deleted_task_checkout_keeps_its_commits() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "r", "t1");
    // The worker's commit: its objects are in the checkout's private directory only.
    let worker = commit_file(&task, "w.txt", "the worker's work\n", "worker commit");
    assert!(
        !has_commit(&repo.root, &worker),
        "the fixture's commit leaked"
    );
    let repo_dir = default_repo_dir(&task);
    std::fs::remove_dir_all(&task).unwrap();

    let reference = "refs/anthrex/salvage/r/t1/1";
    let saved = salvage_gone(real_git(), &repo.root, &repo_dir, reference, "m", T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));
    assert_eq!(out(&repo.root, &["rev-parse", reference]), worker);
    let names = out(&repo.root, &["ls-tree", "-r", "--name-only", reference]);
    assert!(names.lines().any(|name| name == "w.txt"), "{names}");
    assert_eq!(
        out(&repo.root, &["show", &format!("{reference}:w.txt")]),
        "the worker's work"
    );
    // A replay (the op re-run after a crash) gives the same ref again.
    let again = salvage_gone(real_git(), &repo.root, &repo_dir, reference, "m", T).unwrap();
    assert_eq!(again.as_deref(), Some(reference));

    remove_worktree(real_git(), &repo.root, &task, T).unwrap();
    assert!(!repo_dir.exists(), "the checkout's repository was kept");
    assert_eq!(
        out(&repo.root, &["rev-parse", "--verify", reference]),
        worker
    );
    out(
        &repo.root,
        &["fsck", "--connectivity-only", "--no-dangling"],
    );
}

#[test]
fn a_deleted_checkout_with_nothing_new_salvages_nothing() {
    let repo = repo();
    // `HEAD` still at the base the checkout was made from.
    let (_keep, task) = task_worktree(&repo, "r", "t1");
    let repo_dir = default_repo_dir(&task);
    std::fs::remove_dir_all(&task).unwrap();
    let saved = salvage_gone(
        real_git(),
        &repo.root,
        &repo_dir,
        "refs/anthrex/salvage/r/t1/1",
        "m",
        T,
    )
    .unwrap();
    assert_eq!(saved, None);

    // `HEAD` at the task branch's tip: the worker's commit was already recorded.
    let (_keep2, task) = task_worktree(&repo, "r", "t2");
    let worker = commit_file(&task, "w.txt", "recorded\n", "worker commit");
    assert_eq!(sync(real_git(), &task, T).unwrap(), worker);
    assert_eq!(out(&repo.root, &["rev-parse", "anthrex/r/t2"]), worker);
    let repo_dir = default_repo_dir(&task);
    std::fs::remove_dir_all(&task).unwrap();
    let saved = salvage_gone(
        real_git(),
        &repo.root,
        &repo_dir,
        "refs/anthrex/salvage/r/t2/1",
        "m",
        T,
    )
    .unwrap();
    assert_eq!(saved, None);
    assert_eq!(salvage_refs(&repo.root), "");
}

#[test]
fn a_repository_with_no_head_salvages_nothing() {
    let repo = repo();
    let (keep, _wt) = wt_dir();
    let missing = keep.path().join("never-made");
    let saved = salvage_gone(
        real_git(),
        &repo.root,
        &missing,
        "refs/anthrex/salvage/r/t1/1",
        "m",
        T,
    )
    .unwrap();
    assert_eq!(saved, None);
}
