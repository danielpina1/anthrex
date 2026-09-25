//! M8a final fix batch F1c (re-review 4, concern 3a): a task checkout is its own
//! repository in anthrex's data directory. Its making is idempotent step by step
//! (decision 43): the repository's files, `HEAD`, the checkout's `.git` file, then its
//! files from `HEAD`, and the engine's `ready` marker last. A crash before the marker
//! leaves a checkout the next attempt finishes; the checkout's config names the user's
//! object store as its only alternate and turns reflogs and automatic gc off.

mod support;

use daemon::run::git::{Repo, prepare_task_worktree};
use support::run_git::{T, commit_file, head, out, real_git, repo, wrapper_git, wt_dir};

#[test]
fn a_checkout_interrupted_before_its_files_is_finished_by_the_next_attempt() {
    let repo = repo();
    let base = commit_file(&repo.root, "f.txt", "base\n", "base");
    let (_wt, wt) = wt_dir();
    let data = tempfile::tempdir().unwrap();
    let path = wt.join("runs/ck01/t1");
    let dir = data.path().join("tasks/t1");
    let tools = tempfile::tempdir().unwrap();
    // The daemon dies while the checkout's files are put in place.
    let dying = wrapper_git(
        tools.path(),
        r#"case " $* " in *" read-tree -u --reset "*) exit 99;; esac"#,
    );
    let err = prepare_task_worktree(
        dying.as_os_str(),
        &repo.root,
        "anthrex/ck01/t1",
        &base,
        &path,
        &dir,
        T,
    )
    .unwrap_err();
    assert!(!err.is_empty());
    let repo_dir = Repo::at(&dir);
    assert!(
        !repo_dir.ready(),
        "marked ready before its files were there"
    );
    assert!(!path.join("f.txt").exists());

    let h = prepare_task_worktree(
        real_git(),
        &repo.root,
        "anthrex/ck01/t1",
        &base,
        &path,
        &dir,
        T,
    )
    .unwrap();
    assert_eq!(h, base);
    assert!(repo_dir.ready());
    assert_eq!(
        std::fs::read_to_string(path.join("f.txt")).unwrap(),
        "base\n"
    );
    assert_eq!(out(&path, &["status", "--porcelain"]), "");
    assert_eq!(head(&path), base);

    // The config the engine wrote: the user's object store the only alternate, no
    // reflogs, no automatic gc, the user's repository config included.
    assert_eq!(out(&path, &["config", "core.logAllRefUpdates"]), "false");
    assert_eq!(out(&path, &["config", "gc.auto"]), "0");
    assert_eq!(
        out(&path, &["config", "--includes", "--local", "user.email"]),
        "run@tester.test"
    );
    assert_eq!(
        std::fs::read_to_string(repo_dir.objects().join("info/alternates")).unwrap(),
        format!(
            "{}\n",
            repo.root
                .join(".git/objects")
                .canonicalize()
                .unwrap()
                .display()
        )
    );
    // Its commits stay in its own store until the engine imports them.
    let work = commit_file(&path, "w.txt", "w\n", "work");
    assert!(
        !support::run_git::try_git(&repo.root, &["cat-file", "-e", &work])
            .status
            .success()
    );
}
