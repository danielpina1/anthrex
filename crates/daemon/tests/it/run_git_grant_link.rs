//! Milestone 9.11 decision 16 (D8): the Linux grant's two windows. (1) A link a worker
//! left at a path the grant denies used to be removed and replaced by a placeholder,
//! leaving a moment where the path was free to plant again; the grant now refuses
//! instead, and leaves the link for the user to see. (2) The pin check ran before each
//! daemon git call in a task checkout, never after: a `commondir` or `shallow` planted
//! while the call ran steered it unseen. The check now runs after the call too, and a
//! failure discards the call's result.
//!
//! The Linux grant shape is built on any OS (`GrantShape::WholeDir`), so both run
//! everywhere.

use crate::support;

use daemon::run::git::{GrantShape, prepare_task_worktree, worker_git_grant};
use daemon::run::role_launch::{task_repo_dir, worker_git_roots};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use support::run_git::{T, commit_file, out, real_git, repo, wrapper_git, wt_dir};

struct Checkout {
    _repo: support::TempRepo,
    _wt: tempfile::TempDir,
    data: tempfile::TempDir,
    common: PathBuf,
    task: PathBuf,
    admin: PathBuf,
}

/// A task checkout (its own repository in the run's data directory), as the engine
/// makes one.
fn checkout(run: &str) -> Checkout {
    let repo = repo();
    let base = commit_file(&repo.root, "f.txt", "base\n", "base");
    let (wt, wt_path) = wt_dir();
    let data = tempfile::tempdir().unwrap();
    let task = wt_path.join(format!("runs/{run}/t1"));
    prepare_task_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/t1"),
        &base,
        &task,
        &task_repo_dir(data.path(), "t1"),
        T,
    )
    .unwrap();
    let common = PathBuf::from(out(
        &repo.root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ))
    .canonicalize()
    .unwrap();
    let admin = PathBuf::from(out(&task, &["rev-parse", "--absolute-git-dir"]))
        .canonicalize()
        .unwrap();
    Checkout {
        _repo: repo,
        _wt: wt,
        data,
        common,
        task,
        admin,
    }
}

#[test]
fn a_link_at_a_denied_path_refuses_the_grant() {
    let elsewhere = tempfile::tempdir().unwrap();
    let target = elsewhere.path().join("file");
    std::fs::write(&target, "theirs\n").unwrap();
    // A file the grant denies, and a directory it denies.
    for (name, to) in [
        ("packed-refs", target.as_path()),
        ("hooks", elsewhere.path()),
    ] {
        let c = checkout(&format!("gl-{name}"));
        let path = c.admin.join(name);
        if path.is_dir() {
            std::fs::remove_dir_all(&path).unwrap();
        } else {
            let _ = std::fs::remove_file(&path);
        }
        std::os::unix::fs::symlink(to, &path).unwrap();
        let roots = worker_git_roots(c.data.path(), "t1");
        let refused = worker_git_grant(&c.common, &c.task, &roots, GrantShape::WholeDir)
            .expect_err("a link at a denied path was granted");
        assert_eq!(
            refused,
            format!("the checkout's git directory has a link at {name}; anthrex will not grant it")
        );
        // The link is left in place, and nothing at its target was touched.
        let meta = std::fs::symlink_metadata(&path).unwrap();
        assert!(meta.file_type().is_symlink(), "{name}'s link was removed");
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "theirs\n");
        // The macOS shape denies nothing, so it is unchanged.
        worker_git_grant(&c.common, &c.task, &roots, GrantShape::Files).unwrap();
    }
}

/// The deadline of a call through a `git` stand-in: its first exec can stall on macOS
/// (a fresh script is scanned before it runs), which the call must not time out on.
fn deadline() -> Instant {
    Instant::now() + Duration::from_secs(120)
}

/// A `git` stand-in that, for `rev-parse` only, plants `name` in the checkout's git
/// directory `admin` and then runs the real git: the plant lands during the call,
/// after the check before it.
fn planting(dir: &Path, admin: &Path, name: &str) -> PathBuf {
    wrapper_git(
        dir,
        &format!(
            "case \" $* \" in *\" rev-parse \"*) printf 'x\\n' > '{}' ;; esac",
            admin.join(name).display()
        ),
    )
}

#[test]
fn a_commondir_planted_during_a_call_discards_its_result() {
    for name in ["commondir", "shallow"] {
        let c = checkout(&format!("gp-{name}"));
        let bin = tempfile::tempdir().unwrap();
        let git = planting(bin.path(), &c.admin, name);
        // The check before the call passes: nothing is planted yet.
        let refused = daemon::worktree::run_git(
            git.as_os_str(),
            &c.task,
            &[OsStr::new("rev-parse"), OsStr::new("HEAD")],
            deadline(),
        )
        .expect_err("a call whose checkout was tampered with during it returned its result");
        let text = refused.to_string();
        assert!(
            text.contains(&format!(
                "it has a {}",
                match name {
                    "commondir" => "commondir",
                    _ => "shallow file",
                }
            )),
            "{text}"
        );
        assert!(c.admin.join(name).exists(), "the stand-in planted nothing");
    }
}
