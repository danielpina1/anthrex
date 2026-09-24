//! M8a final fix batch F1, fix round 4 (re-review 2, S2 and S4): a hand-back refused
//! after its merge ran must not leave that merge in progress, or every later hand-back
//! of the task is refused ("a merge is already in progress") and a held task retried
//! loops. The refused merge is undone (keeping the worker's own uncommitted edits), a
//! detached `HEAD` is refused before anything is merged, and the engine's own untouched
//! leftover is cleared on the next hand-back's entry. An abort resets to `HEAD`, not to
//! the branch tip.

mod support;

use daemon::run::git::{abort_merge, create_run_branch, hand_back, prepare_worktree};
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, wrapper_git, wt_dir};

struct World {
    _repo: TempRepo,
    _wt: tempfile::TempDir,
    run: String,
    integration: PathBuf,
    task: PathBuf,
}

fn world(run: &str) -> World {
    let repo = repo();
    commit_file(&repo.root, "f.txt", "base\n", "base");
    let base = commit_file(&repo.root, "g.txt", "base\n", "base g");
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
        _repo: repo,
        _wt: wt,
        run: run.to_string(),
        integration,
        task,
    }
}

fn merge_head_exists(dir: &Path) -> bool {
    try_git(dir, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
        .status
        .success()
}

fn own(w: &World) -> String {
    format!("refs/heads/anthrex/{}/t1", w.run)
}

/// The second parent of `commit`.
fn second_parent(dir: &Path, commit: &str) -> String {
    out(dir, &["rev-parse", &format!("{commit}^2")])
}

/// S2: the compare-and-swap loses to a concurrent commit on the task's branch. The
/// refusal undoes the merge, keeps the worker's uncommitted edit, and the next
/// hand-back succeeds.
#[test]
fn a_hand_back_refused_after_its_merge_undoes_it_and_the_next_one_succeeds() {
    let w = world("hr01");
    let tip = commit_file(&w.task, "t.txt", "task\n", "task work");
    let run_head = commit_file(&w.integration, "r.txt", "run\n", "run work");
    // A commit the worker's leftover process lands on the branch mid-hand-back.
    let tree = out(&w.task, &["rev-parse", "HEAD^{tree}"]);
    let side = out(&w.task, &["commit-tree", &tree, "-p", &tip, "-m", "side"]);
    let tools = tempfile::tempdir().unwrap();
    let marker = tools.path().join("raced");
    let git = wrapper_git(
        tools.path(),
        &format!(
            r#"if [ ! -e '{marker}' ]; then
  for a in "$@"; do
    if [ "$a" = update-ref ]; then
      : > '{marker}'
      "$REAL" -C "$2" update-ref {own} {side} {tip} || exit 98
      break
    fi
  done
fi"#,
            marker = marker.display(),
            own = own(&w),
        ),
    );
    // An uncommitted edit of a file the merge does not touch.
    std::fs::write(w.task.join("g.txt"), "worker edit\n").unwrap();

    let err = hand_back(git.as_os_str(), &w.task, &run_head, T).unwrap_err();
    assert!(marker.exists(), "the race never ran");
    assert!(err.contains("moved"), "{err}");
    assert!(!merge_head_exists(&w.task), "the merge was left: {err}");
    assert!(!w.task.join("r.txt").exists(), "the merge's file was left");
    assert_eq!(
        std::fs::read_to_string(w.task.join("g.txt")).unwrap(),
        "worker edit\n",
        "the worker's edit was lost"
    );
    assert_eq!(out(&w.task, &["status", "--porcelain"]), "M g.txt");

    let done = hand_back(real_git(), &w.task, &run_head, T).unwrap();
    assert_eq!(done.onto, side);
    assert!(done.files.is_empty());
    assert_eq!(second_parent(&w.task, &done.head), run_head);
    assert_eq!(head(&w.task), done.head);
    assert!(!merge_head_exists(&w.task));
}

/// S2: a detached `HEAD` is refused before anything is merged; once the worktree is
/// back on its branch, the hand-back succeeds.
#[test]
fn a_detached_head_is_refused_before_the_merge() {
    let w = world("hr02");
    commit_file(&w.task, "t.txt", "task\n", "task work");
    let run_head = commit_file(&w.integration, "r.txt", "run\n", "run work");
    out(&w.task, &["checkout", "-q", "--detach", "HEAD~1"]);

    let err = hand_back(real_git(), &w.task, &run_head, T).unwrap_err();
    assert!(err.contains("not on"), "{err}");
    assert!(!merge_head_exists(&w.task), "a merge was left: {err}");

    out(
        &w.task,
        &["checkout", "-q", &format!("anthrex/{}/t1", w.run)],
    );
    let done = hand_back(real_git(), &w.task, &run_head, T).unwrap();
    assert_eq!(second_parent(&w.task, &done.head), run_head);
}

/// S2: the engine's own clean merge, left uncommitted and untouched (a crash, or an
/// undo that failed), is cleared by the next hand-back, which then succeeds; one whose
/// index the worker changed since is not.
#[test]
fn the_engines_untouched_leftover_merge_is_cleared_on_entry() {
    let w = world("hr03");
    let tip = commit_file(&w.task, "t.txt", "task\n", "task work");
    let run_head = commit_file(&w.integration, "r.txt", "run\n", "run work");
    out(
        &w.task,
        &["merge", "-q", "--no-ff", "--no-commit", &run_head],
    );

    let done = hand_back(real_git(), &w.task, &run_head, T).unwrap();
    assert_eq!(done.onto, tip);
    assert_eq!(second_parent(&w.task, &done.head), run_head);
    assert!(!merge_head_exists(&w.task));

    // A touched leftover: the worker staged a change on top of the clean merge.
    let w = world("hr04");
    commit_file(&w.task, "t.txt", "task\n", "task work");
    let run_head = commit_file(&w.integration, "r.txt", "run\n", "run work");
    out(
        &w.task,
        &["merge", "-q", "--no-ff", "--no-commit", &run_head],
    );
    std::fs::write(w.task.join("r.txt"), "worker's version\n").unwrap();
    out(&w.task, &["add", "r.txt"]);

    let err = hand_back(real_git(), &w.task, &run_head, T).unwrap_err();
    assert!(err.contains("already in progress"), "{err}");
    assert!(merge_head_exists(&w.task));
    assert_eq!(
        std::fs::read_to_string(w.task.join("r.txt")).unwrap(),
        "worker's version\n"
    );
}

/// S4: after a conflicted hand-back the worker detaches `HEAD` (allowed); the abort
/// resets the index and files to `HEAD`'s commit, as `git merge --abort` does, leaving
/// no phantom staged change against it.
#[test]
fn an_abort_with_a_detached_head_resets_to_head() {
    let w = world("hr05");
    commit_file(&w.task, "f.txt", "task\n", "task edit");
    let run_head = commit_file(&w.integration, "f.txt", "run\n", "run edit");
    let conflicted = hand_back(real_git(), &w.task, &run_head, T).unwrap();
    assert_eq!(conflicted.files, vec!["f.txt".to_string()]);
    let admin = PathBuf::from(out(&w.task, &["rev-parse", "--absolute-git-dir"]));
    let parent = out(&w.task, &["rev-parse", "HEAD~1"]);
    std::fs::write(admin.join("HEAD"), format!("{parent}\n")).unwrap();

    abort_merge(real_git(), &w.task, T).unwrap();
    assert!(!merge_head_exists(&w.task));
    assert_eq!(head(&w.task), parent);
    assert_eq!(out(&w.task, &["status", "--porcelain"]), "");
}
