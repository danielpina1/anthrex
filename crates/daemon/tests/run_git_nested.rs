//! Milestone 9.11 decision 14 (A14): a nested repository in a task checkout whose work
//! the salvage cannot keep (uncommitted changes, or commits the user's repository does
//! not have) is listed by `nested_work`, so the checkout is kept rather than removed.
//! The nested repository's own (worker-written) configuration is never read.

mod support;

use daemon::run::git::{nested_work, prepare_worktree};
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, write, wt_dir};

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

/// `git init` of `dir/name`, with its own identity (CI has none).
fn nested(dir: &Path, name: &str) -> PathBuf {
    let path = dir.join(name);
    std::fs::create_dir_all(&path).unwrap();
    out(&path, &["init", "-q"]);
    out(&path, &["config", "user.name", "Nested"]);
    out(&path, &["config", "user.email", "nested@example.test"]);
    path
}

#[test]
fn a_nested_repository_with_uncommitted_work_is_listed() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "r", "t1");
    let sub = nested(&task, "nested");
    commit_file(&sub, "c.txt", "committed\n", "nested commit");
    write(&sub, "p.txt", "uncommitted\n");
    assert_eq!(
        nested_work(real_git(), &task, T).unwrap(),
        vec!["nested".to_string()]
    );
}

#[test]
fn a_nested_repositorys_commits_alone_are_work() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "r", "t1");
    // Deeper than the top, under an untracked directory.
    let sub = nested(&task, "vendor/lib");
    commit_file(&sub, "c.txt", "only here\n", "nested commit");
    assert_eq!(
        nested_work(real_git(), &task, T).unwrap(),
        vec!["vendor/lib".to_string()]
    );
}

#[test]
fn a_clean_nested_repository_is_not_work() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "r", "t1");
    // Empty: no commit, no file.
    nested(&task, "empty");
    // A clone of the user's repository, unchanged: every commit it has, the user's
    // repository has.
    let clone = task.join("clone");
    out(
        &task,
        &[
            "clone",
            "-q",
            repo.root.to_str().unwrap(),
            clone.to_str().unwrap(),
        ],
    );
    assert_eq!(
        nested_work(real_git(), &task, T).unwrap(),
        Vec::<String>::new()
    );
}

#[test]
fn a_nested_repositorys_own_config_is_never_read() {
    let repo = repo();
    let (keep, task) = task_worktree(&repo, "r", "t1");
    let sub = nested(&task, "nested");
    // If the probe read this config, `p.txt` would be ignored and the repository
    // would look clean.
    let excludes = keep.path().join("worker-excludes");
    std::fs::write(&excludes, "p.txt\n").unwrap();
    out(
        &sub,
        &["config", "core.excludesFile", excludes.to_str().unwrap()],
    );
    write(&sub, "p.txt", "uncommitted\n");
    assert_eq!(
        nested_work(real_git(), &task, T).unwrap(),
        vec!["nested".to_string()]
    );
}

/// Fix round 1 (I4): a `packed-refs` the probe cannot read as a plain file under its
/// cap (a FIFO, an oversized file) counts as work, and nothing blocks on it.
#[test]
fn an_unreadable_packed_refs_is_work_and_never_blocks() {
    use std::os::unix::ffi::OsStrExt;
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "r", "t1");
    let fifo = nested(&task, "fifo");
    let path =
        std::ffi::CString::new(fifo.join(".git/packed-refs").as_os_str().as_bytes()).unwrap();
    // SAFETY: `path` is a valid NUL-terminated path.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let big = nested(&task, "big");
    std::fs::write(big.join(".git/packed-refs"), "# filler\n".repeat(10_000)).unwrap();
    assert_eq!(
        nested_work(real_git(), &task, T).unwrap(),
        vec!["big".to_string(), "fifo".to_string()]
    );
}
