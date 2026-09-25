//! M8a final fix batch F1, fix round 5 (coordinator's follow-up): an engine
//! `update-ref` appends to the reflogs of the ref it moves (the task's branch, and the
//! worktree's `HEAD` when it names that branch), and git opens an existing reflog
//! without refusing a symbolic link. A worker once could write both reflogs, so it
//! could make the unsandboxed engine append a line to any file the user can write.
//! Now engine worktrees have no reflogs at all: the engine writes with
//! `core.logAllRefUpdates=false`, removes any it finds before a worker launches, the
//! worker's grant no longer names them (and its git runs with the same setting), and
//! a reflog path that is a symbolic link is refused before every engine call.

mod support;

use daemon::run::git::{create_run_branch, hand_back, prepare_worktree, worker_git_dirs};
use daemon::run::role_launch::worker_git_roots;
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, out, real_git, repo, wt_dir};

struct World {
    repo: TempRepo,
    _wt: tempfile::TempDir,
    run: String,
    integration: PathBuf,
    task: PathBuf,
    common: PathBuf,
    base: String,
}

fn world(run: &str) -> World {
    let repo = repo();
    let base = commit_file(&repo.root, "f.txt", "base\n", "base");
    let (wt, wt_path) = wt_dir();
    let integration = wt_path.join(format!("runs/{run}/integration"));
    create_run_branch(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/integration"),
        &base,
        &integration,
        T,
    )
    .unwrap();
    let task = wt_path.join(format!("runs/{run}/t1"));
    prepare_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/t1"),
        &base,
        &task,
        T,
    )
    .unwrap();
    let common = PathBuf::from(out(
        &repo.root,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    ))
    .canonicalize()
    .unwrap();
    World {
        repo,
        _wt: wt,
        run: run.to_string(),
        integration,
        task,
        common,
        base,
    }
}

impl World {
    fn admin(&self) -> PathBuf {
        PathBuf::from(out(&self.task, &["rev-parse", "--absolute-git-dir"]))
    }

    /// The two reflogs an engine write to the task's branch appends to.
    fn reflogs(&self) -> [PathBuf; 2] {
        [
            self.admin().join("logs/HEAD"),
            self.common
                .join("logs/refs/heads/anthrex")
                .join(&self.run)
                .join("t1"),
        ]
    }

    /// A task commit the test makes as the worker would (its git runs with
    /// `core.logAllRefUpdates=false`), and a run head that merges cleanly.
    fn clean_case(&self) -> String {
        std::fs::write(self.task.join("t.txt"), "task\n").unwrap();
        out(&self.task, &["add", "t.txt"]);
        out(
            &self.task,
            &[
                "-c",
                "core.logAllRefUpdates=false",
                "commit",
                "-q",
                "-m",
                "task",
            ],
        );
        commit_file(&self.integration, "r.txt", "run\n", "run work")
    }
}

fn exists(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok()
}

/// The engine's worktree creation makes no reflog for the task's branch or its
/// worktree's `HEAD`, and the worker's grant removes any that exist.
#[test]
fn engine_worktrees_have_no_reflogs() {
    let w = world("rl01");
    for log in w.reflogs() {
        assert!(!exists(&log), "{} was created", log.display());
    }
    // Reflogs from before (a daemon of an earlier version, or git run by hand).
    for log in w.reflogs() {
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        std::fs::write(&log, "old\n").unwrap();
    }
    let roots = worker_git_roots(&w.common, &w.run, "t1");
    let granted = worker_git_dirs(&w.common, &w.task, &roots).unwrap();
    for log in w.reflogs() {
        assert!(!exists(&log), "{} survived the grant", log.display());
        assert!(
            !granted.iter().any(|g| log.starts_with(g)),
            "{} is granted",
            log.display()
        );
    }
    assert!(!granted.contains(&w.admin().join("logs")), "{granted:?}");
}

/// A clean hand-back moves the task's branch and creates no reflog.
#[test]
fn a_hand_back_writes_no_reflog() {
    let w = world("rl02");
    let run_head = w.clean_case();
    let back = hand_back(real_git(), &w.task, &run_head, T).unwrap();
    assert!(back.files.is_empty());
    for log in w.reflogs() {
        assert!(!exists(&log), "{} was created", log.display());
    }
}

/// A reflog that is a symbolic link to a file outside the repository (what a worker
/// could once plant) is refused before any engine call, and nothing is appended to
/// the file it names.
#[test]
fn a_reflog_link_is_refused_and_never_written_through() {
    for (index, which) in [0usize, 1].iter().enumerate() {
        let w = world(&format!("rl1{index}"));
        let run_head = w.clean_case();
        let tools = tempfile::tempdir().unwrap();
        let victim = tools.path().join("victim");
        std::fs::write(&victim, "victim\n").unwrap();
        let log = &w.reflogs()[*which];
        std::fs::create_dir_all(log.parent().unwrap()).unwrap();
        let _ = std::fs::remove_file(log);
        std::os::unix::fs::symlink(&victim, log).unwrap();

        let result = hand_back(real_git(), &w.task, &run_head, T);
        assert_eq!(
            std::fs::read_to_string(&victim).unwrap(),
            "victim\n",
            "an engine write appended through {}: {result:?}",
            log.display()
        );
        assert_eq!(out(&w.repo.root, &["rev-parse", "main"]), w.base);
        let err = result.unwrap_err();
        assert!(err.contains("reflog") && err.contains("tampered"), "{err}");
    }
}
