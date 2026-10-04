//! Milestone 9.5 task M9.5.15 (decisions 19, 21 and 22; rulings RR-1, T1-2): a race
//! lane's own standalone checkout, the crown (one compare-and-swap creating the task
//! branch at the lane's head, no checkout step), the stale locks a stopped racer can
//! leave in its checkout's own git directory, and `keep_head`'s salvage. Each against a
//! temporary repository; the executor's re-pin is `driver/lane_ops_tests.rs`'s.

mod support;

use daemon::run::engine::OpResult;
use daemon::run::git::{clear_stale_locks, crown, prepare_task_worktree, salvage, sync, task_tmp};
use daemon::run::role_launch::{
    task_engine_dir, task_objects_dir, task_repo_dir, worker_git_roots,
};
use std::path::{Path, PathBuf};
use support::recording_git;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, write, wt_dir};

/// A repository at a base commit and lane a's checkout `<wt>/runs/r/t1.a`, prepared
/// from it with its repository `<data>/tasks/t1.a`, as `PrepareWorktree` makes it.
struct Lane {
    repo: support::TempRepo,
    _wt: tempfile::TempDir,
    data: tempfile::TempDir,
    path: PathBuf,
    base: String,
}

impl Lane {
    fn new() -> Lane {
        let repo = repo();
        let base = commit_file(&repo.root, "src/a.txt", "a\n", "base");
        let (wt, wt_path) = wt_dir();
        let data = tempfile::tempdir().unwrap();
        let path = wt_path.join("runs/r/t1.a");
        prepare_task_worktree(
            real_git(),
            &repo.root,
            "anthrex/r/t1.a",
            &base,
            &path,
            &task_repo_dir(data.path(), "t1.a"),
            T,
        )
        .unwrap();
        Lane {
            repo,
            _wt: wt,
            data,
            path,
            base,
        }
    }

    fn root(&self) -> &Path {
        &self.repo.root
    }

    /// The racer's commit of `file` on its detached `HEAD`, imported by the engine (the
    /// lane branch moves to it); its id.
    fn commit(&self, file: &str, content: &str) -> String {
        let sha = commit_file(&self.path, file, content, file);
        assert_eq!(sync(real_git(), &self.path, T).unwrap(), sha);
        sha
    }

    fn git_dir(&self) -> PathBuf {
        task_repo_dir(self.data.path(), "t1.a").join("git")
    }

    fn branch(&self, name: &str) -> Option<String> {
        let output = try_git(self.root(), &["rev-parse", "-q", "--verify", name]);
        output
            .status
            .success()
            .then(|| String::from_utf8(output.stdout).unwrap().trim().to_string())
    }
}

/// The `argv` lines of a [`recording_git`] log, each split at its tabs.
fn argv_lines(log: &Path) -> Vec<Vec<String>> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.strip_prefix("argv\t"))
        .map(|rest| rest.split('\t').map(str::to_string).collect())
        .collect()
}

fn position(lines: &[Vec<String>], wanted: &[&str]) -> Option<usize> {
    lines
        .iter()
        .position(|argv| wanted.iter().all(|w| argv.iter().any(|a| a == w)))
}

#[test]
fn a_lane_checkout_has_its_own_repository() {
    let lane = Lane::new();
    let data = lane.data.path();
    let repo = task_repo_dir(data, "t1.a");
    assert_eq!(repo, data.join("tasks/t1.a"));
    assert!(repo.join("git/objects").is_dir());
    assert!(repo.join("engine").is_dir());
    assert_eq!(task_objects_dir(data, "t1.a"), repo.join("git/objects"));
    assert_eq!(task_engine_dir(data, "t1.a"), repo.join("engine"));
    assert_eq!(
        worker_git_roots(data, "t1.a"),
        vec![repo.join("git/objects"), task_tmp(&repo)],
        "the lane's own objects and TMPDIR"
    );
    assert_ne!(task_tmp(&repo), task_tmp(&task_repo_dir(data, "t1.b")));
    assert_eq!(
        lane.branch("refs/heads/anthrex/r/t1.a"),
        Some(lane.base.clone())
    );
    let sha = lane.commit("src/b.txt", "b\n");
    assert_eq!(lane.branch("refs/heads/anthrex/r/t1.a"), Some(sha));
    assert_eq!(lane.branch("refs/heads/anthrex/r/t1"), None);
}

#[test]
fn crown_creates_the_task_branch_at_the_lane_head() {
    let lane = Lane::new();
    let sha = lane.commit("src/b.txt", "b\n");
    let head_file = std::fs::read_to_string(lane.git_dir().join("HEAD")).unwrap();
    let scripts = tempfile::tempdir().unwrap();
    let git = recording_git(scripts.path());

    let result = crown(git.as_os_str(), lane.root(), "anthrex/r/t1", &sha, T).unwrap();
    assert_eq!(result, OpResult::Crowned { head: sha.clone() });
    assert_eq!(lane.branch("refs/heads/anthrex/r/t1"), Some(sha.clone()));
    assert_eq!(
        std::fs::read_to_string(lane.git_dir().join("HEAD")).unwrap(),
        head_file,
        "no checkout step: the lane checkout's HEAD is untouched"
    );
    let lines = argv_lines(&scripts.path().join("git.log"));
    let at = position(&lines, &["update-ref", "refs/heads/anthrex/r/t1"]).expect("update-ref");
    let write = &lines[at];
    assert_eq!(write[2], "--no-optional-locks", "{write:?}");
    for flag in ["core.hooksPath=/dev/null", "commit.gpgSign=false"] {
        assert!(write.iter().any(|a| a == flag), "{flag}: {write:?}");
    }
    assert_eq!(
        write.last().unwrap(),
        &"0".repeat(40),
        "the create's zero oid"
    );
    for argv in &lines {
        assert_eq!(argv[2], "--no-optional-locks", "{argv:?}");
        assert!(
            argv.iter().any(|a| a == "core.hooksPath=/dev/null"),
            "{argv:?}"
        );
    }
}

#[test]
fn crown_is_idempotent_and_refuses_a_moved_branch() {
    let lane = Lane::new();
    let sha = lane.commit("src/b.txt", "b\n");
    crown(real_git(), lane.root(), "anthrex/r/t1", &sha, T).unwrap();

    let scripts = tempfile::tempdir().unwrap();
    let git = recording_git(scripts.path());
    let again = crown(git.as_os_str(), lane.root(), "anthrex/r/t1", &sha, T).unwrap();
    assert_eq!(again, OpResult::Crowned { head: sha.clone() });
    let lines = argv_lines(&scripts.path().join("git.log"));
    let write = position(&lines, &["update-ref"]).expect("the create is tried");
    let read = position(&lines, &["rev-parse", "refs/heads/anthrex/r/t1"]).expect("then read");
    assert!(write < read, "{lines:?}");

    // The task branch elsewhere: the same failed create, read as a moved ref.
    out(
        lane.root(),
        &["update-ref", "refs/heads/anthrex/r/t1", &lane.base],
    );
    let scripts = tempfile::tempdir().unwrap();
    let git = recording_git(scripts.path());
    let moved = crown(git.as_os_str(), lane.root(), "anthrex/r/t1", &sha, T).unwrap();
    assert!(matches!(moved, OpResult::RefMoved { .. }), "{moved:?}");
    assert_eq!(
        lane.branch("refs/heads/anthrex/r/t1"),
        Some(lane.base.clone())
    );
    let lines = argv_lines(&scripts.path().join("git.log"));
    let write = position(&lines, &["update-ref"]).expect("the create is tried");
    let read = position(&lines, &["rev-parse", "refs/heads/anthrex/r/t1"]).expect("then read");
    assert!(write < read, "{lines:?}");
}

/// M9.5.1 item 8: a sha256 repository's ids, and so its zero oid, have 64 digits; the
/// crown's create-only update takes that zero oid (a sha1 one would be refused).
#[test]
fn crown_uses_the_repositorys_zero_oid() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    out(&root, &["init", "-q", "--object-format=sha256"]);
    out(&root, &["config", "user.name", "Run Tester"]);
    out(&root, &["config", "user.email", "run@tester.test"]);
    let sha = commit_file(&root, "a.txt", "a\n", "a");
    assert_eq!(sha.len(), 64);
    let crowned = crown(real_git(), &root, "anthrex/r/t1", &sha, T).unwrap();
    assert_eq!(crowned, OpResult::Crowned { head: sha.clone() });
    assert_eq!(out(&root, &["rev-parse", "refs/heads/anthrex/r/t1"]), sha);
    let again = crown(real_git(), &root, "anthrex/r/t1", &sha, T).unwrap();
    assert_eq!(again, OpResult::Crowned { head: sha });
}

/// Review focus 4: a racer stopped mid-commit can leave `index.lock` or `HEAD.lock` in
/// its checkout's own git directory. Only those two are removed, only there.
#[test]
fn stale_locks_are_cleared_only_in_the_lanes_git_dir() {
    let lane = Lane::new();
    let sha = lane.commit("src/b.txt", "b\n");
    write(&lane.path, "src/b.txt", "dirty\n");
    for name in ["index.lock", "HEAD.lock"] {
        std::fs::write(lane.git_dir().join(name), "").unwrap();
    }
    let users = lane.root().join(".git");
    for name in ["index.lock", "HEAD.lock"] {
        std::fs::write(users.join(name), "").unwrap();
    }
    // M9.5.1 item 8: a partial commit's leftover, which is never waited on.
    let elsewhere = lane.git_dir().join("next-index-123.lock");
    std::fs::write(&elsewhere, "").unwrap();

    let cleared = clear_stale_locks(&lane.git_dir()).unwrap();
    assert_eq!(cleared, vec!["index.lock", "HEAD.lock"]);
    for name in ["index.lock", "HEAD.lock"] {
        assert!(!lane.git_dir().join(name).exists(), "{name} is cleared");
        assert!(users.join(name).is_file(), "the user's {name} is untouched");
    }
    assert!(elsewhere.is_file(), "no other lock is touched");
    assert_eq!(
        clear_stale_locks(&lane.git_dir()).unwrap(),
        Vec::<String>::new()
    );

    let reference = "refs/anthrex/salvage/r/t1/1";
    let saved = salvage(real_git(), &lane.path, reference, "salvage", true, T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));
    assert_eq!(
        out(lane.root(), &["show", &format!("{reference}:src/b.txt")]),
        "dirty"
    );
    assert_eq!(
        out(lane.root(), &["rev-parse", &format!("{reference}^")]),
        sha
    );
    for name in ["index.lock", "HEAD.lock"] {
        std::fs::remove_file(users.join(name)).unwrap();
    }
}

#[test]
fn a_linked_lock_is_removed_never_followed() {
    let lane = Lane::new();
    let target = lane.root().join("kept.txt");
    std::fs::write(&target, "keep\n").unwrap();
    std::os::unix::fs::symlink(&target, lane.git_dir().join("index.lock")).unwrap();
    assert_eq!(
        clear_stale_locks(&lane.git_dir()).unwrap(),
        vec!["index.lock"]
    );
    assert!(target.is_file(), "the link's target is untouched");
}

#[test]
fn keep_head_salvages_a_clean_checkout_at_head() {
    let lane = Lane::new();
    let sha = lane.commit("src/b.txt", "b\n");
    let reference = "refs/anthrex/salvage/r/t1/1";
    let saved = salvage(real_git(), &lane.path, reference, "salvage", true, T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));
    assert_eq!(out(lane.root(), &["rev-parse", reference]), sha);
    // A replay (an op re-run after a crash) gives the same ref again.
    let again = salvage(real_git(), &lane.path, reference, "salvage", true, T).unwrap();
    assert_eq!(again.as_deref(), Some(reference));
    assert_eq!(out(lane.root(), &["rev-parse", reference]), sha);
}

/// Pinning: decision 20's salvage of a clean checkout writes nothing, as before.
#[test]
fn without_keep_head_a_clean_checkout_gets_no_ref() {
    let lane = Lane::new();
    lane.commit("src/b.txt", "b\n");
    let reference = "refs/anthrex/salvage/r/t1/1";
    let saved = salvage(real_git(), &lane.path, reference, "salvage", false, T).unwrap();
    assert_eq!(saved, None);
    assert_eq!(lane.branch(reference), None);
    assert_eq!(
        head(&lane.path),
        lane.branch("refs/heads/anthrex/r/t1.a").unwrap()
    );
}
