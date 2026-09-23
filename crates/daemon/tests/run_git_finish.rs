//! M8a.9: salvage and cleanup (decision 20), accept onto the base branch (decision 20,
//! including an advanced base and a conflict against one), and deleting a run's
//! branches. Split from `run_git_merge.rs` to keep both under AGENTS.md rule 8's ~600
//! lines.

mod support;

use daemon::run::git::{
    AcceptOutcome, accept, delete_branches, prepare_worktree, remove_worktree, salvage,
};
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{
    T, commit_file, head, out, real_git, repo, try_git, worktree_block, write, wt_dir,
};

fn task_worktree(repo: &TempRepo, run: &str, task: &str) -> (tempfile::TempDir, PathBuf) {
    let base = head(&repo.root);
    let (keep, wt) = wt_dir();
    let path = wt.join(format!("runs/{run}/{task}"));
    prepare_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/{task}"),
        &base,
        &path,
        T,
    )
    .unwrap();
    (keep, path)
}

fn salvage_refs(root: &Path) -> String {
    out(
        root,
        &["for-each-ref", "--format=%(refname)", "refs/anthrex"],
    )
}

fn tree_names(dir: &Path, commit: &str) -> Vec<String> {
    // `-z`: without it git quotes a non-ASCII name.
    out(dir, &["ls-tree", "-r", "-z", "--name-only", commit])
        .split('\0')
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .collect()
}

#[test]
fn salvage_of_a_clean_worktree_writes_nothing() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "sv01", "t1");
    commit_file(&task, "done.txt", "committed\n", "committed work");
    // An ignored file does not make a worktree dirty.
    write(&task, "ignored-build.log", "noise\n");

    let saved = salvage(
        real_git(),
        &task,
        "refs/anthrex/salvage/sv01/t1/1",
        "anthrex salvage sv01/t1",
        T,
    )
    .unwrap();
    assert_eq!(saved, None);
    assert_eq!(salvage_refs(&repo.root), "");

    // A lone untracked file is work to save, even when the user's config hides
    // untracked files from `git status`.
    out(&repo.root, &["config", "status.showUntrackedFiles", "no"]);
    write(&task, "notes.txt", "untracked\n");
    let reference = "refs/anthrex/salvage/sv01/t1/1";
    let saved = salvage(real_git(), &task, reference, "anthrex salvage sv01/t1", T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));
    assert!(tree_names(&repo.root, reference).contains(&"notes.txt".to_string()));
}

#[test]
fn salvage_captures_tracked_and_untracked_changes_but_not_ignored_files() {
    let repo = repo();
    commit_file(
        &repo.root,
        ".gitignore",
        "ignored-*\ntarget/\n",
        "ignore target",
    );
    commit_file(&repo.root, "staged.txt", "base\n", "staged file");
    let (_keep, task) = task_worktree(&repo, "sv02", "t1");
    let task_head = head(&task);
    write(&task, "README", "modified\n");
    write(&task, "staged.txt", "staged change\n");
    out(&task, &["add", "staged.txt"]);
    write(&task, "new dir/ü new.txt", "untracked\n");
    write(&task, "target/x", "ignored\n");

    let reference = "refs/anthrex/salvage/sv02/t1/1";
    let message = "anthrex salvage sv02/t1";
    let saved = salvage(real_git(), &task, reference, message, T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));

    let names = tree_names(&repo.root, reference);
    assert!(names.contains(&"README".to_string()), "{names:?}");
    assert!(
        names.contains(&"new dir/ü new.txt".to_string()),
        "{names:?}"
    );
    assert!(!names.iter().any(|n| n.starts_with("target/")), "{names:?}");
    assert_eq!(
        out(&repo.root, &["show", &format!("{reference}:README")]),
        "modified"
    );
    assert_eq!(
        out(&repo.root, &["show", &format!("{reference}:staged.txt")]),
        "staged change"
    );
    assert_eq!(
        out(&repo.root, &["rev-list", "--parents", "-n", "1", reference]),
        format!("{} {task_head}", out(&repo.root, &["rev-parse", reference]))
    );
    assert_eq!(
        out(&repo.root, &["log", "-1", "--format=%B", reference]),
        message
    );
    // The task branch did not move.
    assert_eq!(head(&task), task_head);

    // Salvaging again to the same ref with the same content is the same salvage (a
    // replayed op); a second, different salvage takes the next sequence number, and
    // never overwrites the first.
    assert_eq!(
        salvage(real_git(), &task, reference, message, T)
            .unwrap()
            .as_deref(),
        Some(reference)
    );
    write(&task, "more.txt", "more\n");
    let first = out(&repo.root, &["rev-parse", reference]);
    assert!(salvage(real_git(), &task, reference, message, T).is_err());
    assert_eq!(out(&repo.root, &["rev-parse", reference]), first);
    let second = "refs/anthrex/salvage/sv02/t1/2";
    assert_eq!(
        salvage(real_git(), &task, second, message, T)
            .unwrap()
            .as_deref(),
        Some(second)
    );
    assert!(tree_names(&repo.root, second).contains(&"more.txt".to_string()));
}

#[test]
fn salvage_of_a_conflicted_hand_back_keeps_the_markers() {
    let repo = repo();
    commit_file(&repo.root, "shared.txt", "base\n", "shared");
    let (_keep, task) = task_worktree(&repo, "sv03", "t1");
    commit_file(&task, "shared.txt", "task\n", "task");
    let run_head = commit_file(&repo.root, "shared.txt", "run\n", "run");
    assert!(
        !try_git(&task, &["merge", "--no-edit", &run_head])
            .status
            .success()
    );

    let reference = "refs/anthrex/salvage/sv03/t1/1";
    let saved = salvage(real_git(), &task, reference, "anthrex salvage sv03/t1", T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));
    let text = out(&repo.root, &["show", &format!("{reference}:shared.txt")]);
    assert!(text.contains("<<<<<<<"), "{text}");
}

#[test]
fn remove_refuses_nothing_after_salvage() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "rm01", "t1");
    assert!(
        worktree_block(&repo.root, &task)
            .unwrap()
            .lines()
            .any(|l| l.starts_with("locked")),
        "the task worktree is locked"
    );
    write(&task, "wip.txt", "uncommitted\n");
    write(&task, "README", "edited\n");

    let reference = "refs/anthrex/salvage/rm01/t1/1";
    let saved = salvage(real_git(), &task, reference, "anthrex salvage rm01/t1", T).unwrap();
    assert_eq!(saved.as_deref(), Some(reference));
    remove_worktree(real_git(), &repo.root, &task, T).unwrap();

    assert!(!task.exists());
    assert_eq!(worktree_block(&repo.root, &task), None);
    assert!(tree_names(&repo.root, reference).contains(&"wip.txt".to_string()));
    // The task branch is kept (decision 20: until accept or discard).
    assert!(repo.branch_exists("anthrex/rm01/t1"));

    // Removing it again, or a worktree deleted by hand, is not an error.
    remove_worktree(real_git(), &repo.root, &task, T).unwrap();
    let (_keep2, gone) = task_worktree(&repo, "rm01", "t2");
    std::fs::remove_dir_all(&gone).unwrap();
    remove_worktree(real_git(), &repo.root, &gone, T).unwrap();
    assert_eq!(worktree_block(&repo.root, &gone), None);
}

/// A run branch `anthrex/<run>/integration` one commit ahead of `main`'s current head,
/// changing `file`. Returns `(base_sha, run_head)`.
fn run_branch(repo: &TempRepo, run: &str, file: &str, content: &str) -> (String, String) {
    let base = head(&repo.root);
    let branch = format!("anthrex/{run}/integration");
    out(&repo.root, &["checkout", "-q", "-b", &branch]);
    let run_head = commit_file(&repo.root, file, content, "run work");
    out(&repo.root, &["checkout", "-q", "main"]);
    (base, run_head)
}

fn parents(dir: &Path, commit: &str) -> Vec<String> {
    out(dir, &["rev-list", "--parents", "-n", "1", commit])
        .split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect()
}

fn accept_run(repo: &TempRepo, run: &str, expected_base: &str) -> Result<AcceptOutcome, String> {
    accept(
        real_git(),
        &repo.root,
        "main",
        expected_base,
        &format!("anthrex/{run}/integration"),
        &format!("anthrex: accept run {run}: do the thing"),
        T,
    )
}

#[test]
fn accept_requires_the_base_branch_and_a_clean_tree() {
    let repo = repo();
    let (base, _run_head) = run_branch(&repo, "ac01", "a.txt", "run\n");

    out(&repo.root, &["checkout", "-q", "-b", "feature"]);
    assert_eq!(
        accept_run(&repo, "ac01", &base),
        Err(format!(
            "check out main in {} first (currently feature)",
            repo.root.display()
        ))
    );
    out(&repo.root, &["checkout", "-q", "--detach"]);
    assert_eq!(
        accept_run(&repo, "ac01", &base),
        Err(format!(
            "check out main in {} first (currently a detached HEAD)",
            repo.root.display()
        ))
    );
    out(&repo.root, &["checkout", "-q", "main"]);

    write(&repo.root, "README", "uncommitted\n");
    assert_eq!(
        accept_run(&repo, "ac01", &base),
        Err(format!(
            "the working tree at {} has uncommitted changes; commit or stash them first",
            repo.root.display()
        ))
    );
    assert_eq!(out(&repo.root, &["rev-parse", "main"]), base);
    assert!(!repo.root.join("a.txt").exists());

    // Untracked files alone do not block (decision 17's check is of tracked files).
    out(&repo.root, &["checkout", "--", "README"]);
    write(&repo.root, "notes.txt", "mine\n");
    assert!(matches!(
        accept_run(&repo, "ac01", &base),
        Ok(AcceptOutcome::Merged { .. })
    ));
}

#[test]
fn accept_merges_no_ff() {
    let repo = repo();
    let (base, run_head) = run_branch(&repo, "ac02", "a.txt", "run\n");

    let outcome = accept_run(&repo, "ac02", &base).unwrap();
    let AcceptOutcome::Merged { commit } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(out(&repo.root, &["rev-parse", "main"]), commit);
    assert_eq!(parents(&repo.root, &commit), vec![base, run_head]);
    assert_eq!(
        out(&repo.root, &["log", "-1", "--format=%B", &commit]),
        "anthrex: accept run ac02: do the thing"
    );
    assert_eq!(
        out(&repo.root, &["symbolic-ref", "--short", "HEAD"]),
        "main"
    );
    assert_eq!(out(&repo.root, &["status", "--porcelain"]), "");
    assert_eq!(out(&repo.root, &["show", "HEAD:a.txt"]), "run");
}

#[test]
fn accept_onto_an_advanced_base_merges_no_ff() {
    let repo = repo();
    let (_base, run_head) = run_branch(&repo, "ac03", "a.txt", "run\n");
    let advanced = commit_file(&repo.root, "other.txt", "user\n", "user work");

    let outcome = accept_run(&repo, "ac03", &advanced).unwrap();
    let AcceptOutcome::Merged { commit } = outcome else {
        panic!("{outcome:?}");
    };
    assert_eq!(out(&repo.root, &["rev-parse", "main"]), commit);
    assert_eq!(parents(&repo.root, &commit), vec![advanced, run_head]);
    assert!(repo.root.join("a.txt").exists());
    assert!(repo.root.join("other.txt").exists());
}

#[test]
fn accept_conflict_with_an_advanced_base_aborts_and_leaves_base_untouched() {
    let repo = repo();
    let (_base, _run_head) = run_branch(&repo, "ac04", "README", "run line\n");
    let advanced = commit_file(&repo.root, "README", "user line\n", "user edit");
    write(&repo.root, "notes.txt", "untracked, mine\n");
    let status_before = out(&repo.root, &["status", "--porcelain"]);

    let outcome = accept_run(&repo, "ac04", &advanced).unwrap();
    assert_eq!(
        outcome,
        AcceptOutcome::Conflict {
            files: vec!["README".to_string()]
        }
    );
    assert_eq!(out(&repo.root, &["rev-parse", "refs/heads/main"]), advanced);
    assert_eq!(out(&repo.root, &["status", "--porcelain"]), status_before);
    assert!(
        !try_git(&repo.root, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .status
            .success()
    );
    assert_eq!(
        std::fs::read_to_string(repo.root.join("README")).unwrap(),
        "user line\n"
    );
}

#[test]
fn accept_refuses_when_the_base_moved_again() {
    let repo = repo();
    let (base, _run_head) = run_branch(&repo, "ac05", "a.txt", "run\n");
    let moved = commit_file(&repo.root, "other.txt", "user\n", "user work");

    assert_eq!(
        accept_run(&repo, "ac05", &base),
        Err("the base branch moved again; run accept again".to_string())
    );
    assert_eq!(out(&repo.root, &["rev-parse", "main"]), moved);
    assert!(!repo.root.join("a.txt").exists());
    assert_eq!(out(&repo.root, &["status", "--porcelain"]), "");
}

#[test]
fn delete_branches_removes_every_run_branch_but_keeps_salvage_refs() {
    let repo = repo();
    let base = head(&repo.root);
    for branch in [
        "anthrex/db01/integration",
        "anthrex/db01/t1",
        "anthrex/db01/t2",
        "anthrex/db010/integration",
        "feature",
    ] {
        out(&repo.root, &["branch", branch, &base]);
    }
    out(
        &repo.root,
        &["update-ref", "refs/anthrex/salvage/db01/t1/1", &base],
    );
    let branches = || out(&repo.root, &["for-each-ref", "--format=%(refname)"]);

    delete_branches(real_git(), &repo.root, "anthrex/db01/", T).unwrap();
    assert_eq!(
        branches(),
        [
            "refs/anthrex/salvage/db01/t1/1",
            "refs/heads/anthrex/db010/integration",
            "refs/heads/feature",
            "refs/heads/main",
        ]
        .join("\n")
    );

    // Without the trailing slash it means the same prefix, and nothing is left to do.
    delete_branches(real_git(), &repo.root, "anthrex/db01", T).unwrap();
    out(&repo.root, &["branch", "anthrex/db01/t3", &base]);
    delete_branches(real_git(), &repo.root, "anthrex/db01", T).unwrap();
    assert!(!repo.branch_exists("anthrex/db01/t3"));
    assert!(repo.branch_exists("anthrex/db010/integration"));

    // An empty prefix would name every branch.
    assert!(delete_branches(real_git(), &repo.root, "", T).is_err());
    assert!(delete_branches(real_git(), &repo.root, "/", T).is_err());
    assert!(repo.branch_exists("main"));
}
