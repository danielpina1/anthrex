//! Unmaking a worktree: `is_dirty`, `remove` and `discard_new`, including the two
//! states that hold real commits while `git status` stays silent about them.
//!
//! A submodule of `tests/worktree.rs` rather than a test binary of its own, so it keeps
//! that file's `TempRepo`, `worktrees_root`, `git()` and `deadline()` helpers instead of
//! pushing them into `support` for one more reader. The split is AGENTS.md rule 8: one
//! file for making a worktree, one for unmaking it.

use super::*;

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
