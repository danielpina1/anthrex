//! M8a final fix batch F1, fix round 2 (re-review findings R1 and R2): a worker may
//! write its worktree's `HEAD` (a rebase needs it) and its `.git` file (it is inside the
//! worktree), but neither may steer the engine's own, unsandboxed git calls: a `HEAD`
//! naming another branch must never make an engine merge commit onto it, and a `.git`
//! symlinked to another worktree's must never pin (or grant) that worktree's git dir.

mod support;

use daemon::run::git::{
    create_run_branch, hand_back, prepare_worktree, salvage, verify_done, worker_git_dirs,
};
use daemon::run::globs::{OwnsMatcher, ProtectedMatcher};
use daemon::run::plan::BUILTIN_PROTECTED;
use daemon::run::role_launch::worker_git_roots;
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, wt_dir};

struct World {
    repo: TempRepo,
    _wt: tempfile::TempDir,
    run: String,
    integration: PathBuf,
    task: PathBuf,
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
    World {
        repo,
        _wt: wt,
        run: run.to_string(),
        integration,
        task,
        base,
    }
}

fn git_dir(dir: &Path) -> PathBuf {
    PathBuf::from(out(dir, &["rev-parse", "--absolute-git-dir"]))
}

/// R1: the worker's `HEAD` names the base branch; the engine's hand-back merge must not
/// commit onto it, nor may the done check judge the base's tip.
#[test]
fn a_head_naming_the_base_branch_steers_no_engine_call() {
    let w = world("hp01");
    commit_file(&w.task, "t.txt", "task\n", "task work");
    let run_head = commit_file(&w.integration, "r.txt", "run\n", "run work");
    let admin = git_dir(&w.task);
    // What the worker can do with its granted `HEAD`.
    std::fs::write(admin.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    // Its index and files follow, so git sees nothing in the way of a merge.
    out(&w.task, &["reset", "-q", "--hard"]);

    let result = hand_back(real_git(), &w.task, &run_head, T);
    assert_eq!(
        out(&w.repo.root, &["rev-parse", "main"]),
        w.base,
        "the base moved"
    );
    let err = result.unwrap_err();
    assert!(err.contains("HEAD"), "{err}");

    let generated = OwnsMatcher::new(&[]).unwrap();
    let protected = ProtectedMatcher::new(
        &BUILTIN_PROTECTED
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let err = verify_done(
        real_git(),
        &w.task,
        &w.base,
        &w.base,
        &["**".to_string()],
        &generated,
        &protected,
        None,
        T,
    )
    .unwrap_err();
    assert!(err.contains("HEAD"), "{err}");

    // A detached `HEAD` (mid-rebase) is not a redirection: the done check still runs.
    std::fs::write(admin.join("HEAD"), format!("{}\n", head(&w.integration))).unwrap();
    assert!(
        verify_done(
            real_git(),
            &w.task,
            &w.base,
            &w.base,
            &["**".to_string()],
            &generated,
            &protected,
            None,
            T,
        )
        .is_ok()
    );
    assert_eq!(out(&w.repo.root, &["rev-parse", "main"]), w.base);
}

/// R2: the worker makes its `.git` a symlink to the integration worktree's `.git`.
/// Neither a re-pin nor the next launch's grant may land on the integration's git dir.
#[test]
fn a_git_symlink_to_another_worktree_never_moves_the_pin_or_the_grant() {
    let w = world("hp02");
    let own = git_dir(&w.task);
    let theirs = git_dir(&w.integration);
    let index_before = std::fs::read(theirs.join("index")).unwrap();
    std::fs::remove_file(w.task.join(".git")).unwrap();
    std::os::unix::fs::symlink(w.integration.join(".git"), w.task.join(".git")).unwrap();

    let common = w.repo.root.join(".git").canonicalize().unwrap();
    let roots = worker_git_roots(&common, &w.run, "t1");
    for _ in 0..8 {
        let granted = worker_git_dirs(&common, &w.task, &roots).unwrap();
        assert!(
            !granted.iter().any(|p| p.starts_with(&theirs)),
            "the integration's git dir was granted: {granted:?}"
        );
        assert!(granted.contains(&own.join("HEAD")), "{granted:?}");
    }

    // After a restart nothing is pinned yet: the repository's side names the task's own
    // git dir, uniquely, whatever its `.git` symlink says.
    assert_eq!(
        daemon::worktree::pinned::find_git_dir(&common, &w.task).unwrap(),
        own.canonicalize().unwrap()
    );

    // A reuse of the task worktree (a re-dispatch) keeps its own git dir.
    prepare_worktree(
        real_git(),
        &w.repo.root,
        &format!("anthrex/{}/t1", w.run),
        &w.base,
        &w.task,
        T,
    )
    .unwrap();
    std::fs::write(w.task.join("new.txt"), "worker\n").unwrap();
    salvage(
        real_git(),
        &w.task,
        "refs/anthrex/salvage/hp02/t1/1",
        "anthrex salvage hp02/t1",
        T,
    )
    .unwrap();
    assert_eq!(
        std::fs::read(theirs.join("index")).unwrap(),
        index_before,
        "salvage staged into the integration worktree's index"
    );
}
