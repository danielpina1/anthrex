//! M8a.14 fix round 2 (ruling T14-R2): the hand-back reports the tip it actually merged
//! onto (N4), and `resolution_only` tells a pure conflict resolution from a claim that
//! carries more (the reviewer's rule for decision 36's straight-to-the-queue pass).

mod support;

use daemon::run::git::{hand_back, prepare_worktree, resolution_only};
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, wrapper_git, write, wt_dir};

/// Commits `path` on `branch` (created from `HEAD` if missing) and goes back.
fn commit_on(root: &Path, branch: &str, path: &str, content: &str) -> String {
    let back = out(root, &["symbolic-ref", "--short", "HEAD"]);
    let exists = support::run_git::try_git(root, &["rev-parse", "-q", "--verify", branch])
        .status
        .success();
    if exists {
        out(root, &["checkout", "-q", branch]);
    } else {
        out(root, &["checkout", "-q", "-b", branch]);
    }
    let sha = commit_file(root, path, content, &format!("{branch}: {path}"));
    out(root, &["checkout", "-q", &back]);
    sha
}

/// A task worktree for run `run` at the repository's head.
fn task_worktree(repo: &TempRepo, run: &str) -> (tempfile::TempDir, PathBuf) {
    let base = head(&repo.root);
    let (keep, wt) = wt_dir();
    let path = wt.join(format!("runs/{run}/t1"));
    let branch = format!("anthrex/{run}/t1");
    prepare_worktree(real_git(), &repo.root, &branch, &base, &path, T).unwrap();
    (keep, path)
}

/// Review N4: a commit that lands in the worktree between the hand-back's read of
/// `HEAD` and its merge is the tip the merge was made onto, and `onto` says so.
#[test]
fn hand_back_reports_the_tip_it_actually_merged_onto() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "hb7");
    let claimed = commit_file(&task, "b.txt", "task\n", "task work");
    let run_head = commit_on(&repo.root, "anthrex/hb7/integration", "a.txt", "run\n");
    let tools = tempfile::tempdir().unwrap();
    let git = wrapper_git(
        tools.path(),
        r#"for a in "$@"; do
  if [ "$a" = "--no-ff" ]; then
    "$REAL" -C "$2" -c user.name=t -c user.email=t@t -c commit.gpgsign=false \
      -c core.hooksPath=/dev/null commit -q --allow-empty -m sneak || exit 99
    break
  fi
done"#,
    );

    let back = hand_back(git.as_os_str(), &task, &run_head, T).unwrap();
    assert!(back.files.is_empty(), "{back:?}");
    let sneak = out(&task, &["rev-parse", "HEAD^1"]);
    assert_ne!(sneak, claimed);
    assert_eq!(back.onto, sneak);
    assert_eq!(back.head, head(&task));
}

/// A task and a run branch that both change `shared.txt` (and one other file each);
/// the conflicted hand-back is in the worktree. Returns the worktree, `onto` and the
/// run head.
fn conflicted(repo: &TempRepo, run: &str) -> (tempfile::TempDir, PathBuf, String, String) {
    commit_file(&repo.root, "shared.txt", "base\n", "shared");
    let (keep, task) = task_worktree(repo, run);
    commit_file(&task, "b.txt", "task side\n", "task b");
    commit_file(&task, "shared.txt", "task\n", "task shared");
    let integration = format!("anthrex/{run}/integration");
    commit_on(&repo.root, &integration, "a.txt", "run side\n");
    let run_head = commit_on(&repo.root, &integration, "shared.txt", "run\n");
    let back = hand_back(real_git(), &task, &run_head, T).unwrap();
    assert_eq!(back.files, vec!["shared.txt".to_string()]);
    (keep, task, back.onto, run_head)
}

/// Resolves `shared.txt` and commits the merge, with `extra` files changed in it.
fn resolve(task: &Path, extra: &[(&str, &str)]) -> String {
    write(task, "shared.txt", "resolved\n");
    for (path, content) in extra {
        write(task, path, content);
    }
    out(task, &["add", "-A"]);
    out(
        task,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit",
            "-q",
            "--no-edit",
        ],
    );
    head(task)
}

fn only(task: &Path, head: &str, onto: &str, run_head: &str) -> bool {
    let files = vec!["shared.txt".to_string()];
    resolution_only(real_git(), task, head, onto, run_head, &files, T).unwrap()
}

#[test]
fn resolution_only_accepts_a_pure_conflict_resolution() {
    let repo = repo();
    let (_keep, task, onto, run_head) = conflicted(&repo, "ro1");
    let resolved = resolve(&task, &[]);
    assert!(only(&task, &resolved, &onto, &run_head));
}

#[test]
fn resolution_only_refuses_a_claim_that_carries_more() {
    let fresh = repo;
    // An extra feature commit after the resolution.
    let repo = fresh();
    let (_keep, task, onto, run_head) = conflicted(&repo, "ro2");
    resolve(&task, &[]);
    let extra = commit_file(&task, "c.txt", "a feature\n", "feature");
    assert!(!only(&task, &extra, &onto, &run_head));

    // A merge commit that also changes a file of the task's that did not conflict.
    let repo = fresh();
    let (_keep, task, onto, run_head) = conflicted(&repo, "ro3");
    let evil = resolve(&task, &[("b.txt", "changed in the merge\n")]);
    assert!(!only(&task, &evil, &onto, &run_head));

    // A merge commit that undoes the run side's auto-merged file.
    let repo = fresh();
    let (_keep, task, onto, run_head) = conflicted(&repo, "ro4");
    let evil = resolve(&task, &[("a.txt", "not the run's\n")]);
    assert!(!only(&task, &evil, &onto, &run_head));

    // Not a merge at all: an ordinary commit on the claimed tip.
    let repo = fresh();
    let (_keep, task, onto, run_head) = conflicted(&repo, "ro5");
    out(&task, &["merge", "--abort"]);
    let plain = commit_file(&task, "shared.txt", "resolved\n", "not a merge");
    assert!(!only(&task, &plain, &onto, &run_head));

    // The resolution squashed into an ordinary commit: its tree is the resolution's,
    // but the run head is not a parent.
    let repo = fresh();
    let (_keep, task, onto, run_head) = conflicted(&repo, "ro6");
    write(&task, "shared.txt", "resolved\n");
    out(&task, &["add", "-A"]);
    out(&task, &["merge", "--quit"]);
    let squash = commit_file(&task, "shared.txt", "resolved\n", "squashed");
    assert_eq!(out(&task, &["rev-parse", "HEAD^1"]), onto);
    assert!(!only(&task, &squash, &onto, &run_head));

    // The resolution's tree on the parents in the other order.
    let repo = fresh();
    let (_keep, task, onto, run_head) = conflicted(&repo, "ro7");
    let resolved = resolve(&task, &[]);
    let tree = format!("{resolved}^{{tree}}");
    let swapped = out(
        &task,
        &[
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "commit-tree",
            &tree,
            "-p",
            &run_head,
            "-p",
            &onto,
            "-m",
            "swapped",
        ],
    );
    assert!(only(&task, &resolved, &onto, &run_head));
    assert!(!only(&task, &swapped, &onto, &run_head));
}
