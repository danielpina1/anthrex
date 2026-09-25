//! M8a final fix batch F1c (re-review 4, concern 3a): the user's own git works while a
//! run's workers hold commits the engine has not imported yet.
//!
//! Before F1c a task checkout was a linked worktree of the user's repository. Git walks
//! every linked worktree's `HEAD` and index for `fetch`'s and `pull`'s connectivity
//! check, for `gc`, `repack`, `prune`, `fsck` and `log --all`; a worker's `HEAD` whose
//! objects were only in its private store made each of them fail with `bad object
//! worktrees/<task>/HEAD`, from the worker's first commit until the next import. Now
//! each task checkout is its own repository in anthrex's data directory, and the
//! user's `.git` has no entry for it.

mod support;

use daemon::run::git::{create_run_branch, prepare_worktree, sync};
use std::path::Path;
use std::process::Command;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, wt_dir};

/// The user's git in `dir`, which must succeed; its combined output on failure.
fn users_git(dir: &Path, args: &[&str]) {
    let output = try_git(dir, args);
    assert!(
        output.status.success(),
        "the user's git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_users_git_works_while_workers_hold_unimported_commits() {
    // An upstream, and the user's clone of it.
    let upstream = repo();
    let first = commit_file(&upstream.root, "u.txt", "one\n", "upstream one");
    let clones = tempfile::tempdir().unwrap();
    let user = clones.path().canonicalize().unwrap().join("user");
    out(
        clones.path(),
        &[
            "clone",
            "-q",
            upstream.root.to_str().unwrap(),
            user.to_str().unwrap(),
        ],
    );
    out(&user, &["config", "user.name", "Run Tester"]);
    out(&user, &["config", "user.email", "run@tester.test"]);
    let base = head(&user);
    assert_eq!(base, first);

    let (_wt, wt) = wt_dir();
    create_run_branch(
        real_git(),
        &user,
        "anthrex/ug01/integration",
        &base,
        &wt.join("runs/ug01/integration"),
        T,
    )
    .unwrap();
    let task = wt.join("runs/ug01/t1");
    prepare_worktree(real_git(), &user, "anthrex/ug01/t1", &base, &task, T).unwrap();

    // (1) The worker commits in its checkout, as its git does: the commit is its own.
    let work = commit_file(&task, "w.txt", "work\n", "worker one");
    // (2) And a worker commit whose objects are in a store of its own, wherever that is:
    // any `HEAD` the user's repository cannot read.
    let store = tempfile::tempdir().unwrap();
    let own_objects = out(
        &task,
        &[
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "objects",
        ],
    );
    let alternates = std::env::join_paths([
        std::path::PathBuf::from(own_objects),
        user.join(".git/objects"),
    ])
    .unwrap();
    let elsewhere = Command::new("git")
        .args(["-c", "user.name=w", "-c", "user.email=w@w"])
        .args(["commit", "-q", "--allow-empty", "-m", "elsewhere"])
        .current_dir(&task)
        .env("GIT_OBJECT_DIRECTORY", store.path())
        .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", &alternates)
        .status()
        .unwrap();
    assert!(elsewhere.success());

    // The user keeps working meanwhile: upstream moves, and they fetch, pull, gc,
    // prune, check and look at everything.
    let second = commit_file(&upstream.root, "u.txt", "two\n", "upstream two");
    users_git(&user, &["fetch", "-q"]);
    users_git(&user, &["pull", "-q", "--ff-only"]);
    assert_eq!(head(&user), second);
    users_git(&user, &["gc", "-q", "--prune=now"]);
    users_git(&user, &["prune", "--expire=now"]);
    users_git(&user, &["repack", "-a", "-d", "-q"]);
    users_git(&user, &["fsck", "--no-progress"]);
    users_git(&user, &["log", "--all", "--oneline"]);
    assert!(
        !try_git(&user, &["cat-file", "-e", &work]).status.success(),
        "the worker's commit reached the user's store"
    );
    let list = out(&user, &["worktree", "list", "--porcelain"]);
    assert!(
        !list.contains(task.to_str().unwrap()),
        "the user's repository lists the task's checkout: {list}"
    );

    // Nothing the run needs went missing: the worker's own commit still imports, and the
    // task's branch then holds it.
    let back = Command::new("git")
        .args(["reset", "-q", "--hard", &work])
        .current_dir(&task)
        .status()
        .unwrap();
    assert!(back.success());
    assert_eq!(sync(real_git(), &task, T).unwrap(), work);
    assert_eq!(out(&user, &["rev-parse", "anthrex/ug01/t1"]), work);
    users_git(&user, &["fsck", "--no-progress"]);
}
