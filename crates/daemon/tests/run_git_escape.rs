//! M8a final fix batch F1, fix round 1 (review findings N1 and N2): a sandboxed worker
//! must not get its own git configuration run by the daemon, and no daemon git call in a
//! run worktree may be redirected to another repository. Each test plants what a worker
//! can write from inside its worktree (a nested repository, a rewritten `.git` file) or,
//! for what its sandbox no longer lets it write (the git dir's `commondir`), what anything
//! else could. The "escape" is a filter that touches a marker file in the test's own
//! temporary directory, run by the engine's and the probe's own calls.

mod support;

use daemon::git::probe::{PROBE_TIMEOUT, probe};
use daemon::run::git::{pin_worktrees, prepare_worktree, salvage, verify_done};
use daemon::run::globs::{OwnsMatcher, ProtectedMatcher};
use daemon::run::plan::BUILTIN_PROTECTED;
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, wt_dir};

struct World {
    repo: TempRepo,
    _wt: tempfile::TempDir,
    _tools: tempfile::TempDir,
    task: PathBuf,
    base: String,
    /// The file a planted filter touches when anything runs it.
    marker: PathBuf,
}

fn world(run: &str) -> World {
    let repo = repo();
    let base = commit_file(&repo.root, "f.txt", "base\n", "base");
    let (wt, wt_path) = wt_dir();
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
    let tools = tempfile::tempdir().unwrap();
    let marker = tools.path().canonicalize().unwrap().join("escaped");
    World {
        repo,
        _wt: wt,
        _tools: tools,
        task,
        base,
        marker,
    }
}

/// A repository at `dir` (created) whose own config defines `filter.x.clean` touching
/// `marker`, with a committed `.gitattributes` selecting it for every file.
fn evil_repo(dir: &Path, marker: &Path) {
    std::fs::create_dir_all(dir).unwrap();
    out(dir, &["init", "-q", "."]);
    out(dir, &["config", "user.name", "w"]);
    out(dir, &["config", "user.email", "w@w"]);
    std::fs::write(dir.join(".gitattributes"), "* filter=x\n").unwrap();
    std::fs::write(dir.join("g"), "1\n").unwrap();
    out(dir, &["add", "g", ".gitattributes"]);
    out(dir, &["commit", "-q", "-m", "s"]);
    // Defined after the commit, so setting it up runs nothing.
    out(
        dir,
        &[
            "config",
            "filter.x.clean",
            &format!("sh -c 'touch {}; cat'", marker.display()),
        ],
    );
    assert!(!marker.exists());
}

/// Every engine and probe call that looks at a task worktree: the probe, the done check
/// and salvage (its `status`, `add -A` and the rest). Errors are fine; running the
/// planted filter is not.
fn engine_calls(w: &World) {
    let _ = probe(real_git(), &w.task, PROBE_TIMEOUT);
    let generated = OwnsMatcher::new(&[]).unwrap();
    let protected = ProtectedMatcher::new(
        &BUILTIN_PROTECTED
            .iter()
            .map(|s| s.to_string())
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let _ = verify_done(
        real_git(),
        &w.task,
        &w.base,
        &w.base,
        &["**".to_string()],
        &generated,
        &protected,
        None,
        T,
    );
    let _ = salvage(
        real_git(),
        &w.task,
        "refs/anthrex/salvage/x/t1/1",
        "anthrex salvage x/t1",
        T,
    );
}

/// N1: a nested repository committed as a gitlink in the task worktree, its own config
/// defining the filter, and a change inside it.
#[test]
fn a_nested_repository_in_a_task_worktree_runs_nothing() {
    let w = world("es01");
    let sub = w.task.join("sub");
    evil_repo(&sub, &w.marker);
    out(&w.task, &["add", "sub"]);
    out(&w.task, &["commit", "-q", "-m", "add sub"]);
    std::fs::write(sub.join("g"), "2\n").unwrap();
    // And a second, untracked one with a change.
    let sub2 = w.task.join("sub2");
    evil_repo(&sub2, &w.marker);
    std::fs::write(sub2.join("g"), "3\n").unwrap();
    // Proof the plant is live: a plain `git status` in the worktree runs it.
    out(&w.task, &["status", "--porcelain"]);
    assert!(w.marker.exists(), "the plant is not live");
    std::fs::remove_file(&w.marker).unwrap();

    engine_calls(&w);
    assert!(!w.marker.exists(), "a nested repository's filter ran");
    // Twice: salvage's second run finds its ref and compares instead.
    std::fs::write(sub.join("g"), "4\n").unwrap();
    engine_calls(&w);
    assert!(!w.marker.exists(), "a nested repository's filter ran");
}

/// N2 (a): the worktree's `.git` file pointed at a repository the worker made.
#[test]
fn a_rewritten_git_file_is_never_followed() {
    let w = world("es02");
    let evil = w.task.join(".evil");
    evil_repo(&evil, &w.marker);
    std::fs::write(
        w.task.join(".git"),
        format!("gitdir: {}\n", evil.join(".git").display()),
    )
    .unwrap();
    std::fs::write(w.task.join(".gitattributes"), "* filter=x\n").unwrap();
    std::fs::write(w.task.join("f.txt"), "changed\n").unwrap();

    engine_calls(&w);
    assert!(
        !w.marker.exists(),
        "the worker's own repository's filter ran"
    );
}

/// N2 (c): the worktree's `.git` file pointed at the user's own git directory. Salvage
/// must not stage the worker's files into the user's index on the base branch.
#[test]
fn a_git_file_pointed_at_the_users_repository_leaves_it_alone() {
    let w = world("es03");
    std::fs::write(
        w.task.join(".git"),
        format!("gitdir: {}\n", w.repo.root.join(".git").display()),
    )
    .unwrap();
    std::fs::write(w.task.join("f.txt"), "worker content\n").unwrap();
    std::fs::write(w.task.join("n.txt"), "new\n").unwrap();
    let before = out(&w.repo.root, &["status", "--porcelain"]);
    let main = head(&w.repo.root);

    engine_calls(&w);
    assert_eq!(out(&w.repo.root, &["diff", "--cached", "--name-only"]), "");
    assert_eq!(out(&w.repo.root, &["status", "--porcelain"]), before);
    assert_eq!(head(&w.repo.root), main);
}

/// N2 (b): the git dir's `commondir` pointed at a repository with the filter. A worker's
/// sandbox can no longer write it (`run_git_sandbox.rs`); if anything else does, every
/// call in the worktree is refused.
#[test]
fn a_rewritten_commondir_refuses_every_call() {
    let w = world("es04");
    let evil = w.task.join(".evc");
    evil_repo(&evil, &w.marker);
    let git_dir = PathBuf::from(out(&w.task, &["rev-parse", "--absolute-git-dir"]));
    std::fs::write(
        git_dir.join("commondir"),
        format!("{}\n", evil.join(".git").display()),
    )
    .unwrap();
    std::fs::write(w.task.join(".gitattributes"), "* filter=x\n").unwrap();
    std::fs::write(w.task.join("f.txt"), "changed\n").unwrap();

    engine_calls(&w);
    assert!(
        !w.marker.exists(),
        "a filter from the redirected common dir ran"
    );
    let err = salvage(
        real_git(),
        &w.task,
        "refs/anthrex/salvage/x/t1/2",
        "anthrex salvage x/t1",
        T,
    )
    .unwrap_err();
    assert!(err.contains("tampered"), "{err}");
    assert!(probe(real_git(), &w.task, PROBE_TIMEOUT).is_none());
}

/// After a daemon restart the worktrees exist but nothing in this process pinned them:
/// the restore pins each one from the repository's side before any call, and one the
/// repository does not list is refused outright.
#[test]
fn a_restart_pins_existing_worktrees_before_any_call() {
    let w = world("es05");
    // A worktree the engine did not create in this process, as after a restart.
    let restarted = w.task.with_file_name("t2");
    out(
        &w.repo.root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "anthrex/es05/t2",
            restarted.to_str().unwrap(),
            &w.base,
        ],
    );
    std::fs::write(
        restarted.join(".git"),
        format!("gitdir: {}\n", w.repo.root.join(".git").display()),
    )
    .unwrap();
    std::fs::write(restarted.join("f.txt"), "worker content\n").unwrap();
    let common = w.repo.root.join(".git");
    let stray = w.task.with_file_name("not-a-worktree");
    std::fs::create_dir_all(&stray).unwrap();
    pin_worktrees(
        &common,
        &[
            (restarted.clone(), Some("anthrex/es05/t2".to_string())),
            (stray.clone(), None),
        ],
    );

    let _ = salvage(
        real_git(),
        &restarted,
        "refs/anthrex/salvage/x/t2/1",
        "s",
        T,
    );
    assert_eq!(out(&w.repo.root, &["diff", "--cached", "--name-only"]), "");
    let err = salvage(real_git(), &stray, "refs/anthrex/salvage/x/s/1", "s", T).unwrap_err();
    assert!(err.contains("is not a linked worktree of"), "{err}");
}
