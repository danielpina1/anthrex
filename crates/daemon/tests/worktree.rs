//! M5.3 — worktree create, the dirty check, removal and cleanup, against real
//! repositories in temporary directories and the real `git` on the machine.
//!
//! Root resolution itself belongs to milestone 4.5 and is tested in `tests/project.rs`;
//! the only thing asserted here is that `create` delegates to it and reports its own
//! error when the directory is not a working tree.

mod support;

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

#[test]
fn is_dirty_sees_modified_and_untracked_but_not_ignored_files() {
    let repo = TempRepo::new();
    let (_keep, wt_root) = worktrees_root();
    let created = worktree::create(git(), &repo.root, "dirt", &wt_root, deadline()).unwrap();
    let path = &created.worktree.path;

    assert!(!worktree::is_dirty(git(), path, deadline()).unwrap());

    fs::write(path.join("ignored-build.log"), "noise\n").unwrap();
    assert!(
        !worktree::is_dirty(git(), path, deadline()).unwrap(),
        "a file the committed .gitignore covers is not a change"
    );

    let untracked = path.join("untracked.txt");
    fs::write(&untracked, "new\n").unwrap();
    assert!(worktree::is_dirty(git(), path, deadline()).unwrap());

    fs::remove_file(&untracked).unwrap();
    assert!(!worktree::is_dirty(git(), path, deadline()).unwrap());

    fs::write(path.join("README"), "two\n").unwrap();
    assert!(worktree::is_dirty(git(), path, deadline()).unwrap());
}

#[test]
fn remove_clean_worktree_keeps_the_branch() {
    let repo = TempRepo::new();
    let (_keep, wt_root) = worktrees_root();
    let created = worktree::create(git(), &repo.root, "clean", &wt_root, deadline()).unwrap();

    worktree::remove(git(), &created.worktree, false, deadline()).unwrap();

    assert!(!created.worktree.path.exists());
    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);
    assert!(
        repo.branch_exists("clean"),
        "removal never deletes the branch"
    );
}

#[test]
fn remove_dirty_worktree_is_refused_then_forced() {
    let repo = TempRepo::new();
    let (_keep, wt_root) = worktrees_root();
    let created = worktree::create(git(), &repo.root, "dirty", &wt_root, deadline()).unwrap();
    let path = created.worktree.path.clone();
    fs::write(path.join("untracked.txt"), "unsaved work\n").unwrap();

    let error = worktree::remove(git(), &created.worktree, false, deadline()).unwrap_err();

    match &error {
        WorktreeError::Dirty { path: reported } => assert_eq!(reported, &path),
        other => panic!("{other:?}"),
    }
    assert!(
        path.join("untracked.txt").exists(),
        "a refused removal must leave every byte of the work in place"
    );
    assert_eq!(repo.worktree_paths().len(), 2);

    worktree::remove(git(), &created.worktree, true, deadline()).unwrap();

    assert!(!path.exists());
    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);
    assert!(repo.branch_exists("dirty"));
}

#[test]
fn remove_of_a_missing_path_prunes() {
    let repo = TempRepo::new();
    let (_keep, wt_root) = worktrees_root();
    let created = worktree::create(git(), &repo.root, "gone", &wt_root, deadline()).unwrap();
    fs::remove_dir_all(&created.worktree.path).unwrap();
    assert_eq!(
        repo.worktree_paths().len(),
        2,
        "still registered before the prune"
    );

    worktree::remove(git(), &created.worktree, false, deadline()).unwrap();

    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);
    assert!(repo.branch_exists("gone"));
}

/// What `git status --porcelain` says, which for both states in review item 2 is
/// nothing at all — the reason the two checks beside it exist.
fn porcelain_status(path: &Path) -> String {
    let output = support::git_output(path, &[OsStr::new("status"), OsStr::new("--porcelain")]);
    assert!(output.status.success(), "git status failed");
    String::from_utf8(output.stdout).unwrap()
}

/// Review item 3: only `NotFound` means the worktree is gone. Any other error reading
/// the path must not be reported to the user as a removal that succeeded.
#[test]
fn remove_does_not_call_an_unreadable_path_gone() {
    use std::os::unix::fs::PermissionsExt;
    let repo = TempRepo::new();
    let (_keep, wt_root) = worktrees_root();
    let created = worktree::create(git(), &repo.root, "unreadable", &wt_root, deadline()).unwrap();
    let parent = created.worktree.path.parent().unwrap().to_path_buf();
    let original = fs::metadata(&parent).unwrap().permissions();
    fs::set_permissions(&parent, fs::Permissions::from_mode(0o000)).unwrap();

    // Root ignores the mode, so confirm the denial is real before asserting on it.
    let denied = matches!(
        fs::symlink_metadata(&created.worktree.path),
        Err(ref error) if error.kind() == std::io::ErrorKind::PermissionDenied
    );
    let result = worktree::remove(git(), &created.worktree, false, deadline());
    fs::set_permissions(&parent, original).unwrap();

    if !denied {
        eprintln!("skipped: the directory mode did not deny access (running as root?)");
        return;
    }
    let error = result.expect_err("an unreadable worktree is not a removed worktree");
    assert!(
        !matches!(error, WorktreeError::Dirty { .. }),
        "the failure must be git's, not a dirty verdict: {error:?}"
    );
    assert_eq!(
        repo.worktree_paths().len(),
        2,
        "the worktree is still registered, so the user can retry"
    );
    assert!(created.worktree.path.join("README").exists());
}

/// Review item 2, first state: a detached `HEAD` whose commit is on no branch. `git
/// status` is silent about it and a plain `git worktree remove` deletes it, at which
/// point the commit is unreachable.
#[test]
fn a_detached_head_holding_unreachable_commits_is_dirty() {
    let repo = TempRepo::new();
    let (_keep, wt_root) = worktrees_root();
    let created = worktree::create(git(), &repo.root, "detachable", &wt_root, deadline()).unwrap();
    let path = created.worktree.path.clone();

    assert!(!worktree::is_dirty(git(), &path, deadline()).unwrap());

    support::git(&path, &[OsStr::new("checkout"), OsStr::new("--detach")]);
    assert!(
        !worktree::is_dirty(git(), &path, deadline()).unwrap(),
        "detached at a commit a branch still reaches is not work at risk"
    );

    fs::write(path.join("scratch.txt"), "an approach worth keeping\n").unwrap();
    support::git(&path, &[OsStr::new("add"), OsStr::new("scratch.txt")]);
    support::git(
        &path,
        &[
            OsStr::new("commit"),
            OsStr::new("-m"),
            OsStr::new("scratch"),
        ],
    );

    assert_eq!(
        porcelain_status(&path),
        "",
        "the committed state is invisible to `git status`, which is the whole hazard"
    );
    assert!(
        worktree::is_dirty(git(), &path, deadline()).unwrap(),
        "a commit no ref reaches is work a removal would destroy"
    );

    let error = worktree::remove(git(), &created.worktree, false, deadline()).unwrap_err();

    match &error {
        WorktreeError::Dirty { path: reported } => assert_eq!(reported, &path),
        other => panic!("{other:?}"),
    }
    assert!(path.join("scratch.txt").exists());

    worktree::remove(git(), &created.worktree, true, deadline()).unwrap();
    assert!(!path.exists());
}

/// Review item 2, second state: a rebase paused at `edit` with a clean tree. The
/// `rebase-merge` state and every commit already replayed go without a murmur.
#[test]
fn a_paused_rebase_with_a_clean_tree_is_dirty() {
    let repo = TempRepo::new();
    support::commit_more(&repo.root, 3);
    let (_keep, wt_root) = worktrees_root();
    let created = worktree::create(git(), &repo.root, "rebasing", &wt_root, deadline()).unwrap();
    let path = created.worktree.path.clone();

    assert!(!worktree::is_dirty(git(), &path, deadline()).unwrap());

    pause_rebase_at_edit(&path);

    assert_eq!(
        porcelain_status(&path),
        "",
        "the paused rebase leaves a clean tree, which is the whole hazard"
    );
    assert!(
        worktree::is_dirty(git(), &path, deadline()).unwrap(),
        "a paused rebase is work a removal would destroy"
    );

    let error = worktree::remove(git(), &created.worktree, false, deadline()).unwrap_err();

    match &error {
        WorktreeError::Dirty { path: reported } => assert_eq!(reported, &path),
        other => panic!("{other:?}"),
    }
    assert!(path.exists());

    worktree::remove(git(), &created.worktree, true, deadline()).unwrap();
    assert!(!path.exists());
}

/// Stops an interactive rebase of the last two commits at the first one, leaving a
/// `rebase-merge` state and a clean working tree.
fn pause_rebase_at_edit(worktree: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let editor = worktree.join(".pause-rebase.sh");
    fs::write(
        &editor,
        "#!/bin/sh\nsed -e '1s/^pick/edit/' \"$1\" > \"$1.anthrex\" && mv \"$1.anthrex\" \"$1\"\n",
    )
    .unwrap();
    fs::set_permissions(&editor, fs::Permissions::from_mode(0o755)).unwrap();

    let output = support::git_output_env(
        worktree,
        &[("GIT_SEQUENCE_EDITOR", editor.as_os_str())],
        &[OsStr::new("rebase"), OsStr::new("-i"), OsStr::new("HEAD~2")],
    );
    assert!(
        output.status.success(),
        "rebase -i: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    fs::remove_file(&editor).unwrap();
    assert!(
        worktree.join(".git").exists(),
        "the worktree is still a worktree"
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
        WorktreeError::Git { action, stderr } => {
            assert_eq!(action, "worktree add");
            assert!(stderr.contains("already exists"), "{stderr:?}");
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
fn discard_new_deletes_only_a_branch_it_created() {
    let repo = TempRepo::new();
    let (_keep, wt_root) = worktrees_root();

    let fresh = worktree::create(git(), &repo.root, "fresh", &wt_root, deadline()).unwrap();
    assert!(fresh.created_branch);

    worktree::discard_new(git(), &fresh).unwrap();

    assert!(!fresh.worktree.path.exists());
    assert!(
        !repo.branch_exists("fresh"),
        "a branch this create made moments ago at HEAD holds nothing to lose"
    );
    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);

    repo.git(&[OsStr::new("branch"), OsStr::new("preexisting")]);
    let reused = worktree::create(git(), &repo.root, "preexisting", &wt_root, deadline()).unwrap();
    assert!(!reused.created_branch);

    worktree::discard_new(git(), &reused).unwrap();

    assert!(!reused.worktree.path.exists());
    assert!(
        repo.branch_exists("preexisting"),
        "a branch that was already there is not this create's to delete"
    );
    assert_eq!(repo.worktree_paths(), vec![repo.root.clone()]);
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

    assert!(matches!(error, WorktreeError::TimedOut { .. }), "{error:?}");
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
        WorktreeError::Git { action, .. } => assert_eq!(action, "worktree add"),
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

    assert!(matches!(error, WorktreeError::Git { .. }), "{error:?}");
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
