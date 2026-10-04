//! Task M9.5.15, part 2 (ruling T1-3; M9.5.1 item 9): a daemon restart pins every lane
//! checkout of a race again, as `PrepareWorktree` pinned it: a live lane under its own
//! branch `anthrex/<run>/<task>.<lane>`, the crowned (or adopted) lane under the task
//! branch, each lane's proof and review checkouts read-only. Without it a lane checkout
//! is unpinned after a restart and the import refuses it.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use proto::LaneState;

use super::run_worktree_paths;
use crate::run::git;
use crate::run::model::Run;
use crate::run::test_support::{PROFILE, plan_with, race_of, run_ok, task_toml};
use crate::worktree::pinned;

const T: Duration = Duration::from_secs(30);

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.name=w", "-c", "user.email=w@w"])
        .args([
            "-c",
            "commit.gpgsign=false",
            "-c",
            "core.hooksPath=/dev/null",
        ])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_PREFIX")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn program() -> &'static OsStr {
    OsStr::new("git")
}

/// A run `r1` on a temporary repository whose task `t1` races, both lane checkouts and
/// lane a's proof checkout prepared from the base.
fn racing() -> (tempfile::TempDir, Run) {
    let tmp = tempfile::tempdir().unwrap();
    let top = tmp.path().canonicalize().unwrap();
    let root = top.join("repo");
    std::fs::create_dir_all(&root).unwrap();
    git(&root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("a.txt"), "a\n").unwrap();
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-q", "-m", "base"]);
    let base = git(&root, &["rev-parse", "HEAD"]);
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.id = "r1".into();
    run.root = root.clone();
    run.git_common_dir = root.join(".git").canonicalize().unwrap();
    run.wt_dir = top.join("wt");
    run.data_dir = top.join("data/runs/r1");
    run.tasks[0].branch = "anthrex/r1/t1".into();
    run.tasks[0].worktree = run.task_path("t1");
    for lane in ["t1.a", "t1.b"] {
        let path = run.task_path(lane);
        let repo = git::checkout_repo_dir(&run.data_dir, &path);
        let branch = format!("anthrex/r1/{lane}");
        git::prepare_task_worktree(program(), &root, &branch, &base, &path, &repo, T).unwrap();
    }
    let proof = run.proof_path("t1.a");
    let repo = git::checkout_repo_dir(&run.data_dir, &proof);
    git::prepare_scratch_in(program(), &root, &proof, &base, &repo, T).unwrap();
    let task = &run.tasks[0];
    run.tasks[0].race = Some(race_of(task, [LaneState::Working, LaneState::Review]));
    (tmp, run)
}

/// A daemon restart: the registry is empty, then `restore` pins the run's checkouts.
fn restart(run: &Run) {
    let paths: Vec<PathBuf> = ["t1", "t1.a", "t1.b", "t1.a.proof", "t1.a.review"]
        .iter()
        .map(|name| run.task_path(name))
        .collect();
    for path in &paths {
        pinned::unpin(path);
    }
    git::pin_worktrees(&run.git_common_dir, &run_worktree_paths(run));
}

/// The racer commits `file` in `checkout`; the engine imports it. Its head.
fn racer_commits(checkout: &Path, file: &str) -> Result<String, String> {
    std::fs::write(checkout.join(file), format!("{file}\n")).unwrap();
    git(checkout, &["add", "-A"]);
    git(checkout, &["commit", "-q", "-m", file]);
    git::sync(program(), checkout, T)
}

fn branch(run: &Run, name: &str) -> String {
    git(&run.root, &["rev-parse", &format!("refs/heads/{name}")])
}

#[test]
fn a_restart_repins_every_lane_checkout() {
    let (_tmp, mut run) = racing();
    restart(&run);
    for lane in ["t1.a", "t1.b"] {
        let path = run.task_path(lane);
        let own = format!("refs/heads/anthrex/r1/{lane}");
        assert_eq!(pinned::own_ref(&path), Some(own), "{lane} is pinned");
        let pin = pinned::pinned(&path).unwrap();
        let repo = git::Repo::at(&git::checkout_repo_dir(&run.data_dir, &path));
        assert_eq!(pin.objects, Some(repo.objects()));
        assert_eq!(pin.engine, Some(repo.engine()));
        assert!(pin.standalone);
    }
    let proof = pinned::pinned(&run.proof_path("t1.a")).expect("lane a's proof is pinned");
    assert_eq!(proof.own, None, "a proof checkout is read-only");
    let head = racer_commits(&run.task_path("t1.a"), "one.txt").expect("the import accepts it");
    assert_eq!(branch(&run, "anthrex/r1/t1.a"), head);

    // Lane a is crowned (the crown made the task branch at its head) and the task moved
    // into its checkout; after a restart its imports move the task branch.
    let task = &run.tasks[0];
    run.tasks[0].race = Some(race_of(task, [LaneState::Won, LaneState::Lost]));
    run.tasks[0].worktree = run.task_path("t1.a");
    git(
        &run.root,
        &["update-ref", "refs/heads/anthrex/r1/t1", &head],
    );
    restart(&run);
    let crowned = run.task_path("t1.a");
    assert_eq!(
        pinned::own_ref(&crowned),
        Some("refs/heads/anthrex/r1/t1".into())
    );
    assert_eq!(
        pinned::own_ref(&run.task_path("t1.b")),
        Some("refs/heads/anthrex/r1/t1.b".into()),
        "the losing lane keeps its own branch until it is removed"
    );
    let next = racer_commits(&crowned, "two.txt").expect("the import accepts it");
    assert_eq!(branch(&run, "anthrex/r1/t1"), next, "the task branch moved");
    assert_eq!(
        branch(&run, "anthrex/r1/t1.a"),
        head,
        "the lane branch did not"
    );
}
