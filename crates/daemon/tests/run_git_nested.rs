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

/// `git init --separate-git-dir` of `<task>/<name>` into `git_dir`, then the user's
/// repository's `main` checked out there (detached): an initialized submodule's
/// shape, its `.git` a file naming `git_dir`.
fn separate(task: &Path, name: &str, git_dir: &Path, user: &Path) -> PathBuf {
    let path = task.join(name);
    std::fs::create_dir_all(&path).unwrap();
    std::fs::create_dir_all(git_dir.parent().unwrap()).unwrap();
    out(
        &path,
        &[
            "init",
            "-q",
            &format!("--separate-git-dir={}", git_dir.display()),
        ],
    );
    out(&path, &["fetch", "-q", user.to_str().unwrap(), "HEAD"]);
    out(&path, &["checkout", "-q", "--detach", "FETCH_HEAD"]);
    path
}

/// Fix round 1 (I3): an initialized submodule (its `.git` a file naming a directory
/// under the checkout's own git directory's `modules/`) is probed like a nested
/// repository: clean at a commit the user has, it is not work; with an uncommitted
/// file it is. A `.git` file naming a directory anywhere else still counts as work.
#[test]
fn an_initialized_submodule_is_probed_like_a_nested_repository() {
    let repo = repo();
    let (keep, task) = task_worktree(&repo, "r", "t1");
    let admin = PathBuf::from(out(&task, &["rev-parse", "--absolute-git-dir"]))
        .canonicalize()
        .unwrap();
    let sub = separate(&task, "sub", &admin.join("modules/sub"), &repo.root);
    assert!(sub.join(".git").is_file());
    assert_eq!(
        nested_work(real_git(), &task, T).unwrap(),
        Vec::<String>::new()
    );
    write(&sub, "p.txt", "uncommitted\n");
    assert_eq!(
        nested_work(real_git(), &task, T).unwrap(),
        vec!["sub".to_string()]
    );
    std::fs::remove_file(sub.join("p.txt")).unwrap();
    // Its git directory outside the checkout's own: not probed, kept.
    separate(&task, "away", &keep.path().join("away.git"), &repo.root);
    assert_eq!(
        nested_work(real_git(), &task, T).unwrap(),
        vec!["away".to_string()]
    );
}

/// Fix round 1 (I3), the everyday shape: `git submodule add` of another repository.
/// Its commits are not the user's repository's, but each is its remote's (a
/// remote-tracking tip) or the commit the checkout records for it (its gitlink), so it
/// holds no unsaved work; a commit made in it afterwards does.
#[test]
fn a_submodule_of_another_repository_at_its_upstream_commit_is_not_work() {
    let repo = repo();
    let lib = support::run_git::repo();
    commit_file(&lib.root, "lib.txt", "lib\n", "lib work");
    let (_keep, task) = task_worktree(&repo, "r", "t1");
    // As `git submodule update --init` lays it out (its git directory under the
    // checkout's own `modules/`), without the config entries it would also write,
    // which the pin check refuses in a task checkout.
    let admin = PathBuf::from(out(&task, &["rev-parse", "--absolute-git-dir"]))
        .canonicalize()
        .unwrap();
    std::fs::create_dir_all(admin.join("modules")).unwrap();
    let separate = format!("--separate-git-dir={}", admin.join("modules/sub").display());
    out(
        &task,
        &["clone", "-q", &separate, lib.root.to_str().unwrap(), "sub"],
    );
    out(&task, &["add", "sub"]);
    assert!(task.join("sub/.git").is_file());
    assert_eq!(
        nested_work(real_git(), &task, T).unwrap(),
        Vec::<String>::new()
    );
    commit_file(&task.join("sub"), "new.txt", "local\n", "unpushed");
    assert_eq!(
        nested_work(real_git(), &task, T).unwrap(),
        vec!["sub".to_string()]
    );
}

/// Review m10: a nested repository whose `HEAD` reads as unborn (as a reftable
/// repository's does, `ref: refs/heads/.invalid`) but whose object store holds
/// objects counts as work, even with an empty index and working tree.
#[test]
fn an_unborn_looking_repository_with_objects_is_work() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "r", "t1");
    let sub = nested(&task, "rt");
    commit_file(&sub, "gone.txt", "only here\n", "a commit");
    out(&sub, &["rm", "-q", "gone.txt"]);
    out(&sub, &["commit", "-q", "-m", "empty tree"]);
    std::fs::remove_dir_all(sub.join(".git/refs/heads")).unwrap();
    std::fs::create_dir_all(sub.join(".git/refs/heads")).unwrap();
    std::fs::write(sub.join(".git/HEAD"), "ref: refs/heads/.invalid\n").unwrap();
    assert_eq!(
        nested_work(real_git(), &task, T).unwrap(),
        vec!["rt".to_string()]
    );
}

/// Review m11: a commit reachable only from a nested repository's own linked
/// worktree's `HEAD` counts.
#[test]
fn a_nested_repositorys_linked_worktree_head_counts() {
    let repo = repo();
    let (keep, task) = task_worktree(&repo, "r", "t1");
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
    let elsewhere = keep.path().join("linked");
    out(
        &clone,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            elsewhere.to_str().unwrap(),
        ],
    );
    commit_file(
        &elsewhere,
        "w.txt",
        "only in the linked worktree\n",
        "linked",
    );
    assert_eq!(
        nested_work(real_git(), &task, T).unwrap(),
        vec!["clone".to_string()]
    );
}
