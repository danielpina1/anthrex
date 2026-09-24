//! M8a final fix batch F1, fix round 4 (re-review 2, S1): a sandboxed worker may write
//! its own task branch's ref file (a commit moves it). It can make that file `ref:
//! refs/heads/main`, or a symbolic link. No engine call may then read the base's tip as
//! the task's work, and no engine `update-ref` may follow it onto the base: every
//! pinned call refuses a task branch that is not a plain ref, every root-side read of it
//! refuses a symbolic one, and every engine `update-ref` passes `--no-deref`, which also
//! covers a flip made after those checks, while git runs.

mod support;

use daemon::run::git::{
    create_run_branch, hand_back, prepare_review, prepare_worktree, salvage, verify_done,
};
use daemon::run::globs::{OwnsMatcher, ProtectedMatcher};
use daemon::run::plan::BUILTIN_PROTECTED;
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, out, real_git, repo, wrapper_git, wt_dir};

struct World {
    repo: TempRepo,
    _wt: tempfile::TempDir,
    run: String,
    integration: PathBuf,
    task: PathBuf,
    review: PathBuf,
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
        review: wt_path.join(format!("runs/{run}/review-t1")),
        base,
    }
}

impl World {
    fn branch(&self) -> String {
        format!("anthrex/{}/t1", self.run)
    }

    /// The task branch's loose ref file in the common dir.
    fn own_file(&self) -> PathBuf {
        let common = PathBuf::from(out(
            &self.repo.root,
            &["rev-parse", "--path-format=absolute", "--git-common-dir"],
        ));
        let file = common.join("refs/heads").join(self.branch());
        assert!(file.is_file(), "no loose ref at {}", file.display());
        file
    }

    fn main(&self) -> String {
        out(&self.repo.root, &["rev-parse", "main"])
    }
}

fn matchers() -> (OwnsMatcher, ProtectedMatcher) {
    let protected: Vec<String> = BUILTIN_PROTECTED.iter().map(|s| s.to_string()).collect();
    (
        OwnsMatcher::new(&[]).unwrap(),
        ProtectedMatcher::new(&protected).unwrap(),
    )
}

/// Every engine call the reviewer's S1 names, against a task branch the worker has
/// redirected at the base. Each must refuse; the base must never move.
fn assert_every_engine_call_refuses(w: &World, run_head: &str) {
    let result = hand_back(real_git(), &w.task, run_head, T);
    assert_eq!(w.main(), w.base, "the hand-back moved the base: {result:?}");
    let err = result.unwrap_err();
    assert!(err.contains("tampered"), "{err}");

    let (generated, protected) = matchers();
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
    assert!(err.contains("tampered"), "{err}");

    std::fs::write(w.task.join("new.txt"), "worker\n").unwrap();
    let result = salvage(
        real_git(),
        &w.task,
        &format!("refs/anthrex/salvage/{}/t1/1", w.run),
        "salvage",
        T,
    );
    assert!(result.is_err(), "salvage judged the base's tip: {result:?}");
    std::fs::remove_file(w.task.join("new.txt")).unwrap();

    // A re-dispatch would re-point a branch "with no commit of its own" (the base's tip
    // is an ancestor of the run head) onto the run head.
    let result = prepare_worktree(real_git(), &w.repo.root, &w.branch(), run_head, &w.task, T);
    assert_eq!(w.main(), w.base, "the re-point moved the base: {result:?}");
    assert!(
        result.is_err(),
        "the re-point read the base's tip: {result:?}"
    );

    // A review of the branch by name must not review the base's tree.
    let result = prepare_review(real_git(), &w.repo.root, &w.branch(), &w.base, &w.review, T);
    assert!(
        result.is_err(),
        "the review read the base's tip: {result:?}"
    );
    assert_eq!(w.main(), w.base);
}

/// S1: the worker writes `ref: refs/heads/main` into its own branch file, and makes its
/// index and files match the base.
#[test]
fn a_task_branch_made_a_symref_to_the_base_is_refused() {
    let w = world("sr01");
    let run_head = commit_file(&w.integration, "r.txt", "run\n", "run work");
    commit_file(&w.task, "t.txt", "task\n", "task work");
    std::fs::write(w.own_file(), "ref: refs/heads/main\n").unwrap();
    out(&w.task, &["read-tree", "-u", "--reset", "main"]);
    assert_every_engine_call_refuses(&w, &run_head);
}

/// S1: the same through a symbolic link, relative (git reads it as a symbolic ref) and
/// absolute (git reads through it).
#[test]
fn a_task_branch_made_a_symlink_to_the_base_is_refused() {
    for (run, absolute) in [("sr02", false), ("sr03", true)] {
        let w = world(run);
        let run_head = commit_file(&w.integration, "r.txt", "run\n", "run work");
        let file = w.own_file();
        let main_file = w.repo.root.join(".git/refs/heads/main");
        assert!(main_file.is_file());
        std::fs::remove_file(&file).unwrap();
        let target: PathBuf = if absolute {
            main_file.canonicalize().unwrap()
        } else {
            PathBuf::from("refs/heads/main")
        };
        std::os::unix::fs::symlink(&target, &file).unwrap();
        assert_every_engine_call_refuses(&w, &run_head);
    }
}

/// A git that, the first time its arguments include `update-ref`, rewrites `file` to
/// `ref: refs/heads/main` (what a worker's leftover process could do in that instant,
/// after every engine check), then runs the engine's command.
fn flipping_git(tools: &Path, file: &Path) -> PathBuf {
    let marker = tools.join("flipped");
    wrapper_git(
        tools,
        &format!(
            r#"if [ ! -e '{marker}' ]; then
  for a in "$@"; do
    if [ "$a" = update-ref ]; then
      : > '{marker}'
      printf 'ref: refs/heads/main\n' > '{file}'
      break
    fi
  done
fi"#,
            marker = marker.display(),
            file = file.display(),
        ),
    )
}

/// S1(a): the branch becomes a symbolic ref to the base while the hand-back's
/// compare-and-swap runs. The task has no commit of its own, so its tip is the base's:
/// a dereferencing `update-ref` would move the base.
#[test]
fn a_task_branch_flipped_during_the_hand_back_moves_no_base() {
    let w = world("sr04");
    let run_head = commit_file(&w.integration, "r.txt", "run\n", "run work");
    let tools = tempfile::tempdir().unwrap();
    let git = flipping_git(tools.path(), &w.own_file());

    let result = hand_back(git.as_os_str(), &w.task, &run_head, T);
    assert!(tools.path().join("flipped").exists(), "the race never ran");
    assert_eq!(w.main(), w.base, "the base moved: {result:?}");
    let own = std::fs::read_to_string(w.own_file()).unwrap();
    assert!(!own.starts_with("ref:"), "the symbolic ref survived: {own}");
}

/// S1(a): the same during the re-point of a branch with no commit of its own.
#[test]
fn a_task_branch_flipped_during_a_repoint_moves_no_base() {
    let w = world("sr05");
    let from = commit_file(&w.integration, "r.txt", "run\n", "run work");
    let tools = tempfile::tempdir().unwrap();
    let git = flipping_git(tools.path(), &w.own_file());

    let result = prepare_worktree(
        git.as_os_str(),
        &w.repo.root,
        &w.branch(),
        &from,
        &w.task,
        T,
    );
    assert!(tools.path().join("flipped").exists(), "the race never ran");
    assert_eq!(w.main(), w.base, "the base moved: {result:?}");
    assert_eq!(
        out(&w.repo.root, &["rev-parse", &w.branch()]),
        from,
        "the task's own branch did not take the re-point"
    );
}
