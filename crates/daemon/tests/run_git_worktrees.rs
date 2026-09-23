//! M8a.8: the run and task worktrees (decisions 16, 18 and 19): creation, locking,
//! reuse, re-adding and re-pointing, and engine writes that ignore hooks and signing.

mod support;

use daemon::run::git::{
    CandidateStep, cas_update, commit_tree, create_run_branch, lock_worktree, materialize,
    merge_tree, preflight, prepare_review, prepare_worktree, reattach, remove_worktree, salvage,
    verify_done,
};
use daemon::run::globs::{OwnsMatcher, ProtectedMatcher};
use support::TempRepo;
use support::run_git::{
    T, commit_file, head, out, real_git, repo, try_git, worktree_block, write, wt_dir,
};

#[test]
fn run_branch_and_integration_worktree_are_created_and_locked() {
    let repo = repo();
    let base = head(&repo.root);
    commit_file(&repo.root, "later.txt", "later\n", "after base");
    let (_keep, wt) = wt_dir();
    let path = wt.join("runs/fix-r1ab/integration");

    let created = create_run_branch(
        real_git(),
        &repo.root,
        "anthrex/fix-r1ab/integration",
        &base,
        &path,
        T,
    )
    .unwrap();

    assert_eq!(created, base);
    assert_eq!(head(&path), base);
    let block = worktree_block(&repo.root, &path).expect("git lists the worktree");
    assert!(
        block
            .lines()
            .any(|l| l == "branch refs/heads/anthrex/fix-r1ab/integration"),
        "{block}"
    );
    assert!(
        block.lines().any(|l| l == "locked anthrex run fix-r1ab"),
        "{block}"
    );
}

#[test]
fn task_branch_starts_at_the_given_run_head_and_is_reused() {
    let repo = repo();
    let first = commit_file(&repo.root, "a.txt", "a\n", "a");
    let second = commit_file(&repo.root, "b.txt", "b\n", "b");
    let (_keep, wt) = wt_dir();
    let path = wt.join("runs/r-9f0e/t1");
    let branch = "anthrex/r-9f0e/t1";

    let h = prepare_worktree(real_git(), &repo.root, branch, &first, &path, T).unwrap();
    assert_eq!(h, first);
    assert_eq!(out(&repo.root, &["rev-parse", branch]), first);
    assert_eq!(out(&path, &["symbolic-ref", "--short", "HEAD"]), branch);
    assert!(
        worktree_block(&repo.root, &path)
            .unwrap()
            .lines()
            .any(|l| l.starts_with("locked")),
        "a task worktree is locked right after creation"
    );

    // A second call with the same arguments reuses everything.
    let refs_before = out(&repo.root, &["for-each-ref"]);
    let list_before = out(&repo.root, &["worktree", "list", "--porcelain"]);
    let h = prepare_worktree(real_git(), &repo.root, branch, &first, &path, T).unwrap();
    assert_eq!(h, first);
    assert_eq!(out(&repo.root, &["for-each-ref"]), refs_before);
    assert_eq!(
        out(&repo.root, &["worktree", "list", "--porcelain"]),
        list_before
    );

    // The worktree goes (two `--force`s: it is locked); the branch is re-added.
    out(
        &repo.root,
        &[
            "worktree",
            "remove",
            "--force",
            "--force",
            path.to_str().unwrap(),
        ],
    );
    assert!(!path.exists());
    let h = prepare_worktree(real_git(), &repo.root, branch, &first, &path, T).unwrap();
    assert_eq!(h, first);
    assert_eq!(out(&path, &["symbolic-ref", "--short", "HEAD"]), branch);

    // A newer `from` while the branch has no commit of its own re-points it.
    let h = prepare_worktree(real_git(), &repo.root, branch, &second, &path, T).unwrap();
    assert_eq!(h, second);
    assert_eq!(out(&repo.root, &["rev-parse", branch]), second);
    assert_eq!(head(&path), second);
    assert_eq!(out(&path, &["symbolic-ref", "--short", "HEAD"]), branch);

    // With a commit of its own, a newer `from` leaves it alone.
    let own = commit_file(&path, "own.txt", "own\n", "task work");
    let third = commit_file(&repo.root, "c.txt", "c\n", "c");
    let h = prepare_worktree(real_git(), &repo.root, branch, &third, &path, T).unwrap();
    assert_eq!(h, own);
    assert_eq!(out(&repo.root, &["rev-parse", branch]), own);
}

#[test]
fn a_task_branch_can_live_under_the_run_branch_name() {
    let repo = repo();
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();

    create_run_branch(
        real_git(),
        &repo.root,
        "anthrex/r2cd/integration",
        &base,
        &wt.join("runs/r2cd/integration"),
        T,
    )
    .unwrap();
    prepare_worktree(
        real_git(),
        &repo.root,
        "anthrex/r2cd/t1",
        &base,
        &wt.join("runs/r2cd/t1"),
        T,
    )
    .unwrap();

    assert!(repo.branch_exists("anthrex/r2cd/integration"));
    assert!(repo.branch_exists("anthrex/r2cd/t1"));
    assert!(
        !try_git(&repo.root, &["branch", "anthrex/r2cd"])
            .status
            .success()
    );
}

#[test]
fn engine_writes_ignore_hooks_and_signing() {
    let repo = repo();
    out(&repo.root, &["config", "commit.gpgsign", "true"]);
    out(&repo.root, &["config", "gpg.program", "/bin/false"]);
    repo.failing_post_checkout();
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();

    create_run_branch(
        real_git(),
        &repo.root,
        "anthrex/hk12/integration",
        &base,
        &wt.join("runs/hk12/integration"),
        T,
    )
    .unwrap();
    let task = wt.join("runs/hk12/t1");
    prepare_worktree(real_git(), &repo.root, "anthrex/hk12/t1", &base, &task, T).unwrap();
    // The test's own commit passes `-c commit.gpgsign=false` (support::git_output).
    let newer = commit_file(&repo.root, "n.txt", "n\n", "newer");
    let repointed =
        prepare_worktree(real_git(), &repo.root, "anthrex/hk12/t1", &newer, &task, T).unwrap();
    assert_eq!(repointed, newer);
    prepare_review(
        real_git(),
        &repo.root,
        "anthrex/hk12/t1",
        &base,
        &wt.join("runs/hk12/t1.review"),
        T,
    )
    .unwrap();

    assert!(!repo.hook_marker().exists(), "no hook ran");
}

#[test]
fn engine_paths_with_spaces_and_unicode_work() {
    let repo = support::run_git::identified(TempRepo::with_prefix("ax run ü "));
    let pre = preflight(real_git(), &repo.root, T).unwrap();
    assert_eq!(pre.root, repo.root);
    let wt_keep = tempfile::Builder::new()
        .prefix("ax wt ü ")
        .tempdir_in("/tmp")
        .unwrap();
    let wt = wt_keep.path().canonicalize().unwrap();

    create_run_branch(
        real_git(),
        &repo.root,
        "anthrex/sp01/integration",
        &pre.base_sha,
        &wt.join("runs/sp01/integration"),
        T,
    )
    .unwrap();
    let task = wt.join("runs/sp01/t1");
    prepare_worktree(
        real_git(),
        &repo.root,
        "anthrex/sp01/t1",
        &pre.base_sha,
        &task,
        T,
    )
    .unwrap();
    let h = commit_file(&task, "src/ü file.rs", "fn ü() {}\n", "unicode");
    write(&task, "lib/ü new.rs", "untracked\n");
    let none = OwnsMatcher::new(&[]).unwrap();
    let no_protected = ProtectedMatcher::new(&[]).unwrap();
    let done = verify_done(
        real_git(),
        &task,
        &pre.base_sha,
        &pre.base_sha,
        &["lib/**".to_string()],
        &none,
        &no_protected,
        None,
        T,
    )
    .unwrap();
    assert_eq!(done.outside_owns, vec!["src/ü file.rs".to_string()]);
    assert_eq!(done.untracked_in_owns, vec!["lib/ü new.rs".to_string()]);
    let (_, head_sha, patch) = prepare_review(
        real_git(),
        &repo.root,
        "anthrex/sp01/t1",
        &pre.base_sha,
        &wt.join("runs/sp01/t1.review"),
        T,
    )
    .unwrap();
    assert_eq!(head_sha, h);
    assert!(patch.contains("fn ü() {}"), "{patch}");

    // M8a.9: the merge candidate, materialized in the integration worktree and
    // compare-and-swapped onto the run branch, on the same paths.
    let integration = wt.join("runs/sp01/integration");
    let CandidateStep::Tree(tree) =
        merge_tree(real_git(), &repo.root, &pre.base_sha, &h, T).unwrap()
    else {
        panic!("a clean candidate");
    };
    let candidate = commit_tree(
        real_git(),
        &repo.root,
        &tree,
        &[&pre.base_sha, &h],
        "anthrex: merge t1: ü",
        T,
    )
    .unwrap();
    materialize(real_git(), &integration, &candidate, T).unwrap();
    assert!(integration.join("src/ü file.rs").exists());
    assert!(
        cas_update(
            real_git(),
            &repo.root,
            "anthrex/sp01/integration",
            &candidate,
            &pre.base_sha,
            T
        )
        .unwrap()
    );
    reattach(real_git(), &integration, "anthrex/sp01/integration", T).unwrap();
    assert_eq!(head(&integration), candidate);

    // Salvage of the untracked non-ASCII file, then removal of the locked worktree.
    let reference = "refs/anthrex/salvage/sp01/t1/1";
    let saved = salvage(real_git(), &task, reference, "anthrex salvage sp01/t1", T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));
    assert_eq!(
        out(&repo.root, &["show", &format!("{reference}:lib/ü new.rs")]),
        "untracked"
    );
    remove_worktree(real_git(), &repo.root, &task, T).unwrap();
    assert!(!task.exists());
    assert_eq!(worktree_block(&repo.root, &task), None);
}

fn locked(root: &std::path::Path, path: &std::path::Path) -> bool {
    worktree_block(root, path)
        .unwrap()
        .lines()
        .any(|l| l.starts_with("locked"))
}

#[test]
fn a_task_worktree_deleted_by_hand_or_unlocked_is_restored_and_relocked() {
    let repo = repo();
    let base = head(&repo.root);
    let (_keep, wt) = wt_dir();
    let path = wt.join("runs/rl01/t1");
    let branch = "anthrex/rl01/t1";
    prepare_worktree(real_git(), &repo.root, branch, &base, &path, T).unwrap();

    // Deleted by hand: git still lists it, locked and prunable.
    std::fs::remove_dir_all(&path).unwrap();
    let h = prepare_worktree(real_git(), &repo.root, branch, &base, &path, T).unwrap();
    assert_eq!(h, base);
    assert_eq!(out(&path, &["symbolic-ref", "--short", "HEAD"]), branch);
    assert!(locked(&repo.root, &path));

    // Unlocked by someone: a reuse locks it again.
    out(&repo.root, &["worktree", "unlock", path.to_str().unwrap()]);
    assert!(!locked(&repo.root, &path));
    prepare_worktree(real_git(), &repo.root, branch, &base, &path, T).unwrap();
    assert!(locked(&repo.root, &path));

    // Locking an already locked worktree is not an error.
    lock_worktree(real_git(), &repo.root, &path, "anthrex run rl01", T).unwrap();
    lock_worktree(real_git(), &repo.root, &path, "anthrex run rl01", T).unwrap();
    assert!(locked(&repo.root, &path));
}

/// A pre-warmed worktree at `first` in which setup edited `edited` (tracked) and wrote
/// untracked build output, and a run head `second` that changed `moved`.
fn pre_warmed(
    edited: &str,
    moved: &str,
) -> (TempRepo, tempfile::TempDir, std::path::PathBuf, String) {
    let repo = repo();
    commit_file(&repo.root, "Cargo.lock", "lock v1\n", "lock");
    let first = commit_file(&repo.root, "setup.cfg", "cfg v1\n", "cfg");
    let (keep, wt) = wt_dir();
    let path = wt.join("runs/pw01/t1");
    prepare_worktree(real_git(), &repo.root, "anthrex/pw01/t1", &first, &path, T).unwrap();
    write(&path, edited, "edited by setup\n");
    write(&path, "target/out.bin", "build output\n");
    let second = commit_file(&repo.root, moved, "moved on\n", "run head moves");
    (repo, keep, path, second)
}

fn assert_re_pointed(repo: &TempRepo, path: &std::path::Path, second: &str) {
    let h = prepare_worktree(real_git(), &repo.root, "anthrex/pw01/t1", second, path, T).unwrap();
    assert_eq!(h, second);
    assert_eq!(out(&repo.root, &["rev-parse", "anthrex/pw01/t1"]), second);
    assert_eq!(
        out(path, &["symbolic-ref", "--short", "HEAD"]),
        "anthrex/pw01/t1"
    );
    assert_eq!(
        out(path, &["status", "--porcelain", "--untracked-files=no"]),
        ""
    );
    assert_eq!(
        std::fs::read_to_string(path.join("target/out.bin")).unwrap(),
        "build output\n",
        "untracked build output is kept"
    );
}

#[test]
fn re_pointing_over_a_setup_edit_the_run_head_also_changed() {
    let (repo, _keep, path, second) = pre_warmed("Cargo.lock", "Cargo.lock");
    assert_re_pointed(&repo, &path, &second);
    assert_eq!(
        std::fs::read_to_string(path.join("Cargo.lock")).unwrap(),
        "moved on\n"
    );
}

#[test]
fn re_pointing_drops_a_setup_edit_the_run_head_did_not_change() {
    let (repo, _keep, path, second) = pre_warmed("setup.cfg", "Cargo.lock");
    assert_re_pointed(&repo, &path, &second);
    assert_eq!(
        std::fs::read_to_string(path.join("setup.cfg")).unwrap(),
        "cfg v1\n",
        "a setup edit must not leak into the task's start state"
    );
}

#[test]
fn a_user_branch_named_like_the_run_is_reported_plainly() {
    let repo = repo();
    let base = head(&repo.root);
    out(&repo.root, &["branch", "anthrex/col1"]);
    let (_keep, wt) = wt_dir();

    let err = create_run_branch(
        real_git(),
        &repo.root,
        "anthrex/col1/integration",
        &base,
        &wt.join("runs/col1/integration"),
        T,
    )
    .unwrap_err();

    assert_eq!(
        err,
        "branch anthrex/col1 exists, so git cannot create anthrex/col1/integration; rename or delete anthrex/col1"
    );
}
