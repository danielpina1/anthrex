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

/// F1c round 3 (N2): `objects/` is writable by the worker and by confined commands. A
/// link planted at `objects/info` must not make the engine write `alternates` into
/// the directory it names; the next prepare refuses and the other directory is
/// untouched.
#[test]
fn a_link_at_objects_info_is_refused_and_nothing_is_written_through_it() {
    let repo = repo();
    let base = commit_file(&repo.root, "f.txt", "base\n", "base");
    let (_wt, wt) = wt_dir();
    let data = tempfile::tempdir().unwrap();
    let path = wt.join("runs/ck02/t1");
    let dir = data.path().join("tasks/t1");
    let prepare = || {
        prepare_task_worktree(
            real_git(),
            &repo.root,
            "anthrex/ck02/t1",
            &base,
            &path,
            &dir,
            T,
        )
    };
    prepare().unwrap();
    let victim = tempfile::tempdir().unwrap();
    std::fs::write(victim.path().join("alternates"), "VICTIM\n").unwrap();
    let info = Repo::at(&dir).objects().join("info");
    std::fs::remove_dir_all(&info).unwrap();
    std::os::unix::fs::symlink(victim.path(), &info).unwrap();

    let err = prepare().unwrap_err();
    assert!(err.contains("tampered"), "{err}");
    assert_eq!(
        std::fs::read_to_string(victim.path().join("alternates")).unwrap(),
        "VICTIM\n",
        "the engine wrote through the link"
    );
    let names: Vec<_> = std::fs::read_dir(victim.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names.len(), 1, "{names:?}");
}

/// F1c round 3 (N6): an import that fails for a passing reason (here `index-pack`
/// fails) while a checkout is re-made keeps the worker's `HEAD`; the next attempt
/// imports its commit instead of starting again from the branch tip.
#[test]
fn a_failed_import_while_re_making_a_checkout_keeps_the_workers_head() {
    let repo = repo();
    let base = commit_file(&repo.root, "f.txt", "base\n", "base");
    let (_wt, wt) = wt_dir();
    let data = tempfile::tempdir().unwrap();
    let path = wt.join("runs/ck03/t1");
    let dir = data.path().join("tasks/t1");
    let prepare = |git: &std::ffi::OsStr| {
        prepare_task_worktree(git, &repo.root, "anthrex/ck03/t1", &base, &path, &dir, T)
    };
    prepare(real_git()).unwrap();
    let work = commit_file(&path, "w.txt", "w\n", "work");
    std::fs::remove_dir_all(&path).unwrap();

    let tools = tempfile::tempdir().unwrap();
    let failing = wrapper_git(
        tools.path(),
        r#"case " $* " in *" index-pack "*) exit 99;; esac"#,
    );
    assert!(prepare(failing.as_os_str()).is_err());
    let head = std::fs::read_to_string(Repo::at(&dir).git_dir().join("HEAD")).unwrap();
    assert_eq!(head.trim(), work, "the worker's HEAD was reset");

    assert_eq!(prepare(real_git()).unwrap(), work);
    assert_eq!(out(&repo.root, &["rev-parse", "anthrex/ck03/t1"]), work);
}
