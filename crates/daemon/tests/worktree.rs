//! M5.3 — worktree create, the dirty check, removal and cleanup, against real
//! repositories in temporary directories and the real `git` on the machine.
//!
//! Making a worktree lives here; unmaking one lives in the `removal` submodule below.
//!
//! Root resolution itself belongs to milestone 4.5 and is tested in `tests/project.rs`;
//! the only thing asserted here is that `create` delegates to it and reports its own
//! error when the directory is not a working tree.

mod support;

#[path = "worktree/removal.rs"]
mod removal;

use daemon::worktree::{self, OPERATION_TIMEOUT, WorktreeError, repo_worktrees_dir};
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use support::{TempRepo, head_branch};
use tempfile::TempDir;

fn git() -> &'static OsStr {
    OsStr::new("git")
}

fn deadline() -> Instant {
    Instant::now() + OPERATION_TIMEOUT
}

/// A `worktrees_root` that is already canonical, so the paths these tests compute match
/// the one `create` returns — which canonicalizes the worktree it made, because that path
/// becomes the git registry's key for the window (design decision 21).
fn worktrees_root() -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().canonicalize().unwrap();
    (dir, path)
}

/// The names directly under `dir`, sorted; an empty vector when `dir` does not exist.
fn entries(dir: &Path) -> Vec<String> {
    let Ok(read) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = read
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn a_directory_outside_a_repository_is_not_a_repo() {
    let plain = tempfile::tempdir().unwrap();
    let (_keep, wt_root) = worktrees_root();

    let error = worktree::create(git(), plain.path(), "feat/x", &wt_root, deadline()).unwrap_err();

    assert!(matches!(error, WorktreeError::NotARepo { .. }), "{error:?}");
    assert!(
        error.to_string().starts_with("not a git repository: "),
        "{error}"
    );
    assert!(
        entries(&wt_root).is_empty(),
        "a refused create must leave nothing behind: {:?}",
        entries(&wt_root)
    );
}

#[test]
fn create_with_a_new_branch() {
    let repo = TempRepo::new();
    let (_keep, wt_root) = worktrees_root();

    let created = worktree::create(git(), &repo.root, "feat/new", &wt_root, deadline()).unwrap();

    let expected = repo_worktrees_dir(&wt_root, &repo.root).join("feat-new");
    assert!(created.created_branch);
    assert_eq!(created.worktree.path, expected);
    assert_eq!(created.worktree.repo_root, repo.root);
    assert_eq!(created.worktree.branch, "feat/new");
    assert!(expected.join("README").exists());
    assert!(repo.branch_exists("feat/new"));
    assert_eq!(head_branch(&expected), "feat/new");
}

#[test]
fn create_with_an_existing_branch() {
    let repo = TempRepo::new();
    repo.git(&[OsStr::new("branch"), OsStr::new("existing")]);
    let (_keep, wt_root) = worktrees_root();

    let created = worktree::create(git(), &repo.root, "existing", &wt_root, deadline()).unwrap();

    assert!(!created.created_branch);
    assert_eq!(head_branch(&created.worktree.path), "existing");
    assert!(repo.branch_exists("existing"));
}

#[test]
fn create_from_a_subdirectory_still_uses_the_project_root() {
    let repo = TempRepo::new();
    let sub = repo.root.join("sub");
    fs::create_dir(&sub).unwrap();
    let (_keep, wt_root) = worktrees_root();
    let repo_dir = repo_worktrees_dir(&wt_root, &repo.root);

    let from_sub = worktree::create(git(), &sub, "b", &wt_root, deadline()).unwrap();

    assert_eq!(from_sub.worktree.path, repo_dir.join("b"));
    assert_eq!(from_sub.worktree.repo_root, repo.root);

    // And from inside the linked worktree that create just made: a linked worktree
    // shares its project root with the main checkout, so `<wt>` must not move.
    let nested =
        worktree::create(git(), &from_sub.worktree.path, "c", &wt_root, deadline()).unwrap();

    assert_eq!(nested.worktree.path, repo_dir.join("c"));
    assert_eq!(nested.worktree.repo_root, repo.root);
}

#[test]
fn a_branch_checked_out_elsewhere_fails_cleanly() {
    let repo = TempRepo::new();
    let (_keep, wt_root) = worktrees_root();
    let first = worktree::create(git(), &repo.root, "taken", &wt_root, deadline()).unwrap();

    let again = worktree::create(git(), &repo.root, "taken", &wt_root, deadline()).unwrap_err();

    match &again {
        WorktreeError::BranchInUse { branch, path } => {
            assert_eq!(branch, "taken");
            assert_eq!(path, &first.worktree.path);
        }
        other => panic!("{other:?}"),
    }

    // The branch the main checkout is standing on is the same case.
    let main = worktree::create(git(), &repo.root, "main", &wt_root, deadline()).unwrap_err();

    match &main {
        WorktreeError::BranchInUse { branch, path } => {
            assert_eq!(branch, "main");
            assert_eq!(path, &repo.root);
        }
        other => panic!("{other:?}"),
    }

    assert_eq!(
        repo.worktree_paths(),
        vec![repo.root.clone(), first.worktree.path.clone()]
    );
    assert!(
        first.worktree.path.join("README").exists(),
        "the refused creates must not have touched the worktree that already existed"
    );
    assert_eq!(
        entries(&repo_worktrees_dir(&wt_root, &repo.root)),
        ["taken"]
    );
}

#[test]
fn an_existing_path_is_refused() {
    let repo = TempRepo::new();
    let (_keep, wt_root) = worktrees_root();
    let taken = repo_worktrees_dir(&wt_root, &repo.root).join("feat-x");
    fs::create_dir_all(&taken).unwrap();
    fs::write(taken.join("precious.txt"), "not ours\n").unwrap();

    let error = worktree::create(git(), &repo.root, "feat/x", &wt_root, deadline()).unwrap_err();

    match &error {
        WorktreeError::PathExists { path } => assert_eq!(path, &taken),
        other => panic!("{other:?}"),
    }
    assert!(!repo.branch_exists("feat/x"));
    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);
    assert!(
        taken.join("precious.txt").exists(),
        "a refused create must never touch the directory that was already there"
    );
}

#[test]
fn invalid_and_reserved_branches_are_refused_before_worktree_add() {
    let repo = TempRepo::new();
    // `@{-1}` below is git's "the branch I was on before" shorthand, which only resolves
    // once there is a previous checkout — so give it one.
    repo.git(&[
        OsStr::new("checkout"),
        OsStr::new("-b"),
        OsStr::new("other"),
    ]);
    repo.git(&[OsStr::new("checkout"), OsStr::new("main")]);
    let (_keep, wt_root) = worktrees_root();
    let cases = [
        ("bad name", "branch name cannot contain spaces"),
        ("-x", "branch name cannot start with '-'"),
        (
            "anthrex/r/t1",
            "branches under anthrex/ are reserved for orchestration runs",
        ),
        ("runs", "branch name 'runs' is reserved"),
        ("a..b", "invalid branch name 'a..b'"),
        // Rejected only by the echo-unchanged half of the `check-ref-format` rule:
        // verified against git 2.50.1, `check-ref-format --branch '@{-1}'` *exits zero*
        // here and prints `other`. Exit status alone would let it through, and the
        // worktree would silently land on whatever branch the shorthand resolved to.
        ("@{-1}", "invalid branch name '@{-1}'"),
    ];

    for (branch, expected) in cases {
        let error = worktree::create(git(), &repo.root, branch, &wt_root, deadline()).unwrap_err();
        assert_eq!(error.to_string(), expected, "branch {branch:?}");
        assert!(
            matches!(error, WorktreeError::InvalidBranch(_)),
            "branch {branch:?}: {error:?}"
        );
    }

    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);
    assert!(
        entries(&wt_root).is_empty(),
        "a create refused before `worktree add` must not have made any directory"
    );
}

/// Review item 1: `created_branch` is decided by a `show-ref` that runs before `git
/// worktree add`. A branch that appears in between makes the add fail *without creating
/// the path*, and that branch is someone else's. Cleanup must not delete it.
#[test]
fn cleanup_keeps_a_branch_that_appeared_after_the_show_ref() {
    let repo = TempRepo::new();
    let (_keep, wt_root) = worktrees_root();
    let scripts = tempfile::tempdir().unwrap();
    let racing = racing_git(scripts.path(), &repo.root, "rival");

    let error = worktree::create(
        racing.as_os_str(),
        &repo.root,
        "rival",
        &wt_root,
        deadline(),
    )
    .unwrap_err();

    match &error {
        WorktreeError::FailedAfterAdd(message) => {
            assert!(message.starts_with("git worktree add failed:"), "{message}");
            assert!(message.contains("already exists"), "{message}");
            assert!(
                message.ends_with("; the new worktree was removed"),
                "nothing was left on disk, so the user can retry: {message}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(
        repo.branch_exists("rival"),
        "the branch that won the race is not this create's to delete"
    );
    assert!(
        !repo_worktrees_dir(&wt_root, &repo.root)
            .join("rival")
            .exists()
    );
    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);
}

/// Review item 5: the reachable half of design decision 16's two suffixes. A create that
/// fails after `worktree add` *and* whose cleanup also fails must say so, because the
/// worktree is still on disk and retrying the same branch will now hit `worktree path
/// already exists` with nothing to explain where that path came from.
#[test]
fn a_create_whose_cleanup_also_fails_says_the_worktree_is_still_there() {
    let repo = TempRepo::new();
    repo.failing_post_checkout();
    let (_keep, wt_root) = worktrees_root();
    let repo_dir = repo_worktrees_dir(&wt_root, &repo.root);
    let scripts = tempfile::tempdir().unwrap();
    let obstinate = git_that_refuses_to_remove(scripts.path());

    let error = worktree::create(
        obstinate.as_os_str(),
        &repo.root,
        "stranded",
        &wt_root,
        deadline(),
    )
    .unwrap_err();

    match &error {
        WorktreeError::FailedAfterAdd(message) => {
            assert!(
                message.starts_with("git worktree add failed:"),
                "the failure that actually stopped the create comes first: {message}"
            );
            assert!(
                message.contains("; cleanup failed: "),
                "the user must be told the worktree is still there: {message}"
            );
            assert!(
                message.contains("cleanup refused by the test"),
                "git's own reason survives into the message: {message}"
            );
            assert!(
                !message.contains("the new worktree was removed"),
                "a failed cleanup must never be reported in the words of one that worked: {message}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(
        repo_dir.join("stranded").join("README").exists(),
        "the message is only true if the worktree really is still on disk"
    );
}

/// A `git` that refuses every `worktree remove` and is the real git otherwise, so a
/// create's cleanup fails for a reason the test controls.
fn git_that_refuses_to_remove(dir: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let real = which_git();
    let script = dir.join("obstinate-git");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$*\" in\n  *'worktree remove'*)\n    \
             echo 'fatal: cleanup refused by the test' >&2\n    exit 128 ;;\n  \
             *) exec '{real}' \"$@\" ;;\nesac\n",
            real = real.display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

/// A `git` that creates `branch` in `repo` immediately after answering the `show-ref`
/// that asks whether it exists, and is the real git for everything else — the race in
/// review item 1, made deterministic.
fn racing_git(dir: &Path, repo: &Path, branch: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let real = which_git();
    let script = dir.join("racing-git");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\ncase \"$*\" in\n  *show-ref*)\n    '{real}' \"$@\"\n    rc=$?\n    \
             '{real}' -C '{repo}' branch '{branch}' HEAD >/dev/null 2>&1\n    exit $rc ;;\n  \
             *) exec '{real}' \"$@\" ;;\nesac\n",
            real = real.display(),
            repo = repo.display(),
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

#[test]
fn every_invocation_of_a_real_create_passes_no_optional_locks() {
    let repo = TempRepo::new();
    let (_keep, wt_root) = worktrees_root();
    let scripts = tempfile::tempdir().unwrap();
    let log = scripts.path().join("argv.log");
    let recording = recording_git(scripts.path(), &log);

    let created = worktree::create(
        recording.as_os_str(),
        &repo.root,
        "logged",
        &wt_root,
        deadline(),
    )
    .unwrap();

    assert!(created.worktree.path.join("README").exists());
    let recorded = fs::read_to_string(&log).unwrap();
    let lines: Vec<&str> = recorded.lines().collect();
    assert!(
        lines.len() >= 4,
        "a create runs check-ref-format, worktree list, show-ref and worktree add: {lines:?}"
    );
    for line in &lines {
        assert!(
            line.contains("--no-optional-locks"),
            "recorded invocation without the flag: {line:?}"
        );
    }
}

#[test]
fn a_timeout_during_create_cleans_up() {
    let repo = TempRepo::new();
    repo.slow_post_checkout(10);
    let (_keep, wt_root) = worktrees_root();

    let started = Instant::now();
    let error = worktree::create(
        git(),
        &repo.root,
        "slow",
        &wt_root,
        Instant::now() + Duration::from_secs(2),
    )
    .unwrap_err();
    let elapsed = started.elapsed();

    // The timeout is now wrapped with decision 16's suffix, which is the whole point of
    // surfacing the cleanup result: a user whose 30 s checkout times out on a large
    // repository has to be told whether anthrex left a worktree behind.
    match &error {
        WorktreeError::FailedAfterAdd(message) => {
            assert!(message.contains("timed out after"), "{message}");
            assert!(
                message.ends_with("; the new worktree was removed"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(
        elapsed < Duration::from_secs(6),
        "the hook's own 10 s must not be waited out: {elapsed:?}"
    );
    let repo_dir = repo_worktrees_dir(&wt_root, &repo.root);
    // `create` makes `<wt>` immediately before `git worktree add` and nowhere else, so
    // this is the proof that the deadline struck inside the add and not in one of the
    // cheap checks before it — without which the assertions below would hold for a
    // create that never reached the step whose cleanup is under test.
    assert!(
        repo_dir.exists(),
        "the timeout must have happened inside `worktree add`, not before it"
    );
    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);
    assert!(!repo_dir.join("slow").exists());
    assert!(
        !repo.branch_exists("slow"),
        "the branch this create made at HEAD is cleaned up with it"
    );
}

/// The same cleanup, reached without a timeout: a `post-checkout` hook that exits
/// non-zero makes `git worktree add` fail once the worktree and branch already exist.
#[test]
fn a_create_that_fails_after_worktree_add_cleans_up_but_keeps_an_older_branch() {
    let repo = TempRepo::new();
    repo.failing_post_checkout();
    let (_keep, wt_root) = worktrees_root();
    let repo_dir = repo_worktrees_dir(&wt_root, &repo.root);

    let error = worktree::create(git(), &repo.root, "half", &wt_root, deadline()).unwrap_err();

    match &error {
        WorktreeError::FailedAfterAdd(message) => {
            assert!(message.starts_with("git worktree add failed:"), "{message}");
            assert!(
                message.ends_with("; the new worktree was removed"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(!repo_dir.join("half").exists());
    assert!(
        !repo.branch_exists("half"),
        "the branch this create made moments ago at HEAD goes with it"
    );
    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);

    // The asymmetry that matters: the same failure on a branch that was already there
    // must clean up the worktree and leave the branch, which may hold work.
    repo.git(&[OsStr::new("branch"), OsStr::new("older")]);

    let error = worktree::create(git(), &repo.root, "older", &wt_root, deadline()).unwrap_err();

    assert!(
        matches!(error, WorktreeError::FailedAfterAdd(_)),
        "{error:?}"
    );
    assert!(!repo_dir.join("older").exists());
    assert!(
        repo.branch_exists("older"),
        "cleanup never deletes a branch this create did not make"
    );
    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);
}

/// A `git` that appends its whole argv to `log` and then becomes the real git, so a
/// create can be run for real while every invocation it makes is recorded.
fn recording_git(dir: &Path, log: &Path) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let real = which_git();
    let script = dir.join("recording-git");
    fs::write(
        &script,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexec '{}' \"$@\"\n",
            log.display(),
            real.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    script
}

fn which_git() -> PathBuf {
    let output = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg("command -v git")
        .output()
        .unwrap();
    assert!(output.status.success(), "git is not on PATH");
    PathBuf::from(String::from_utf8(output.stdout).unwrap().trim())
}
