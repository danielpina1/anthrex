//! M8a.9: the merge candidate (decision 36): `merge-tree`, `commit-tree`, the
//! compare-and-swap on the run branch, materializing a candidate in the integration
//! worktree and re-attaching it, and the conflict hand-back; plus the ref reads of
//! decisions 20 and 21 (`read_ref`, `guard_refs`, `commits_since`). Split from
//! `run_git.rs` to keep both under AGENTS.md rule 8's ~600 lines.

mod support;

use daemon::run::git::{
    ACCEPT_LIST_MAX, CandidateStep, RefCheck, cas_update, commit_tree, commits_since,
    create_run_branch, guard_refs, hand_back, materialize, merge_tree, prepare_worktree, read_ref,
    reattach,
};
use std::path::{Path, PathBuf};
use support::TempRepo;
use support::run_git::{T, commit_file, head, out, real_git, repo, try_git, write, wt_dir};

/// Commits `path` on `branch` (created from the current `HEAD` if missing) and returns
/// to the branch `root` was on. Returns the new commit.
fn commit_on(root: &Path, branch: &str, path: &str, content: &str) -> String {
    let back = out(root, &["symbolic-ref", "--short", "HEAD"]);
    if try_git(root, &["rev-parse", "-q", "--verify", branch])
        .status
        .success()
    {
        out(root, &["checkout", "-q", branch]);
    } else {
        out(root, &["checkout", "-q", "-b", branch]);
    }
    let sha = commit_file(root, path, content, &format!("{branch}: {path}"));
    out(root, &["checkout", "-q", &back]);
    sha
}

fn parents(dir: &Path, commit: &str) -> Vec<String> {
    out(dir, &["rev-list", "--parents", "-n", "1", commit])
        .split_whitespace()
        .skip(1)
        .map(str::to_string)
        .collect()
}

fn short(sha: &str) -> &str {
    &sha[..7]
}

#[test]
fn merge_tree_returns_a_tree_or_the_conflicted_files() {
    let repo = repo();
    commit_file(&repo.root, "shared.txt", "base\n", "shared");
    commit_file(&repo.root, "ü both.txt", "base\n", "unicode shared");
    let base = head(&repo.root);

    // Clean: the run and the task change different files.
    let run_head = commit_on(&repo.root, "run", "a.txt", "run\n");
    let task_head = commit_on(&repo.root, "task", "b.txt", "task\n");
    let step = merge_tree(real_git(), &repo.root, &run_head, &task_head, T).unwrap();
    let CandidateStep::Tree(tree) = step else {
        panic!("a clean merge gives a tree, got {step:?}");
    };
    assert_eq!(out(&repo.root, &["cat-file", "-t", &tree]), "tree");
    assert_eq!(out(&repo.root, &["show", &format!("{tree}:a.txt")]), "run");
    assert_eq!(out(&repo.root, &["show", &format!("{tree}:b.txt")]), "task");
    assert_eq!(
        out(&repo.root, &["show", &format!("{tree}:shared.txt")]),
        "base"
    );

    // Conflicted: both change `shared.txt` and the non-ASCII file; `c.txt` merges
    // cleanly and is not listed. The names come back unquoted.
    out(&repo.root, &["checkout", "-q", "-b", "run2", &base]);
    out(&repo.root, &["checkout", "-q", "main"]);
    commit_on(&repo.root, "run2", "shared.txt", "run\n");
    let run2 = commit_on(&repo.root, "run2", "ü both.txt", "run\n");
    out(&repo.root, &["checkout", "-q", "-b", "task2", &base]);
    out(&repo.root, &["checkout", "-q", "main"]);
    commit_on(&repo.root, "task2", "shared.txt", "task\n");
    commit_on(&repo.root, "task2", "c.txt", "clean\n");
    let task2 = commit_on(&repo.root, "task2", "ü both.txt", "task\n");
    let step = merge_tree(real_git(), &repo.root, &run2, &task2, T).unwrap();
    assert_eq!(
        step,
        CandidateStep::Conflict(vec!["shared.txt".to_string(), "ü both.txt".to_string()])
    );

    // Not a commit: an error, not a conflict.
    assert!(merge_tree(real_git(), &repo.root, &run2, "no-such-ref", T).is_err());
}

#[test]
fn commit_tree_and_cas_advance_the_run_branch() {
    let repo = repo();
    let base = head(&repo.root);
    let branch = "anthrex/cas1/integration";
    out(&repo.root, &["branch", branch, &base]);
    let task_head = commit_on(&repo.root, "anthrex/cas1/t1", "t.txt", "task\n");
    let CandidateStep::Tree(tree) =
        merge_tree(real_git(), &repo.root, &base, &task_head, T).unwrap()
    else {
        panic!("clean");
    };

    let message = "anthrex: merge t1: add t";
    let candidate = commit_tree(
        real_git(),
        &repo.root,
        &tree,
        &[&base, &task_head],
        message,
        T,
    )
    .unwrap();
    assert_eq!(
        parents(&repo.root, &candidate),
        vec![base.clone(), task_head.clone()]
    );
    assert_eq!(
        out(&repo.root, &["log", "-1", "--format=%B", &candidate]),
        message
    );
    assert_eq!(
        out(&repo.root, &["rev-parse", &format!("{candidate}^{{tree}}")]),
        tree
    );
    // The task's ancestry is kept, so "was it merged" is an ancestry question.
    assert!(
        try_git(
            &repo.root,
            &["merge-base", "--is-ancestor", &task_head, &candidate]
        )
        .status
        .success()
    );

    // The right `old`: the branch moves.
    assert!(cas_update(real_git(), &repo.root, branch, &candidate, &base, T).unwrap());
    assert_eq!(out(&repo.root, &["rev-parse", branch]), candidate);

    // A stale `old`: false, and nothing moves.
    let other = commit_tree(real_git(), &repo.root, &tree, &[&base], "other", T).unwrap();
    assert!(!cas_update(real_git(), &repo.root, branch, &other, &base, T).unwrap());
    assert_eq!(out(&repo.root, &["rev-parse", branch]), candidate);

    // A branch that does not exist is not at any `old`.
    assert!(
        !cas_update(
            real_git(),
            &repo.root,
            "anthrex/cas1/none",
            &other,
            &base,
            T
        )
        .unwrap()
    );
    assert!(
        !try_git(
            &repo.root,
            &["rev-parse", "-q", "--verify", "anthrex/cas1/none"]
        )
        .status
        .success()
    );
}

struct Integration {
    _keep: tempfile::TempDir,
    path: PathBuf,
    branch: String,
    base: String,
}

fn integration(repo: &TempRepo, run: &str) -> Integration {
    let base = head(&repo.root);
    let (keep, wt) = wt_dir();
    let path = wt.join(format!("runs/{run}/integration"));
    let branch = format!("anthrex/{run}/integration");
    create_run_branch(real_git(), &repo.root, &branch, &base, &path, T).unwrap();
    Integration {
        _keep: keep,
        path,
        branch,
        base,
    }
}

#[test]
fn materialize_and_reattach_leave_the_integration_worktree_on_its_branch() {
    let repo = repo();
    let int = integration(&repo, "mat1");
    let task_head = commit_on(&repo.root, "anthrex/mat1/t1", "t.txt", "task\n");
    let CandidateStep::Tree(tree) =
        merge_tree(real_git(), &repo.root, &int.base, &task_head, T).unwrap()
    else {
        panic!("clean");
    };
    let candidate = commit_tree(
        real_git(),
        &repo.root,
        &tree,
        &[&int.base, &task_head],
        "anthrex: merge t1: t",
        T,
    )
    .unwrap();

    // Left-overs of an earlier check: an untracked file and an edit to a tracked one.
    write(&int.path, "build-output/junk.o", "junk\n");
    write(&int.path, "README", "edited by a check\n");

    materialize(real_git(), &int.path, &candidate, T).unwrap();
    assert_eq!(head(&int.path), candidate);
    assert!(
        !try_git(&int.path, &["symbolic-ref", "-q", "HEAD"])
            .status
            .success(),
        "HEAD is detached at the candidate"
    );
    assert!(
        int.path.join("t.txt").exists(),
        "the task's file is checked out"
    );
    assert!(!int.path.join("build-output").exists(), "clean -fd ran");
    assert_eq!(out(&int.path, &["status", "--porcelain"]), "");
    // The run branch did not move.
    assert_eq!(out(&repo.root, &["rev-parse", &int.branch]), int.base);

    // A check that dirties the tree, then back to the run head.
    write(&int.path, "t.txt", "edited by a red check\n");
    reattach(real_git(), &int.path, &int.branch, T).unwrap();
    assert_eq!(
        out(&int.path, &["symbolic-ref", "HEAD"]),
        format!("refs/heads/{}", int.branch)
    );
    assert_eq!(head(&int.path), int.base);
    assert!(!int.path.join("t.txt").exists());
    assert_eq!(out(&int.path, &["status", "--porcelain"]), "");

    // Green: the CAS moves the branch while HEAD is detached, and reattach lands on it.
    materialize(real_git(), &int.path, &candidate, T).unwrap();
    assert!(
        cas_update(
            real_git(),
            &repo.root,
            &int.branch,
            &candidate,
            &int.base,
            T
        )
        .unwrap()
    );
    reattach(real_git(), &int.path, &int.branch, T).unwrap();
    assert_eq!(
        out(&int.path, &["symbolic-ref", "HEAD"]),
        format!("refs/heads/{}", int.branch)
    );
    assert_eq!(head(&int.path), candidate);
}

/// A task worktree at `base` with `shared.txt`, and a task commit on it.
fn task_worktree(
    repo: &TempRepo,
    run: &str,
    file: &str,
    content: &str,
) -> (tempfile::TempDir, PathBuf) {
    let base = head(&repo.root);
    let (keep, wt) = wt_dir();
    let path = wt.join(format!("runs/{run}/t1"));
    prepare_worktree(
        real_git(),
        &repo.root,
        &format!("anthrex/{run}/t1"),
        &base,
        &path,
        T,
    )
    .unwrap();
    commit_file(&path, file, content, "task work");
    (keep, path)
}

#[test]
fn hand_back_leaves_markers_and_merge_head() {
    let repo = repo();
    commit_file(&repo.root, "shared.txt", "base\n", "shared");
    let (_keep, task) = task_worktree(&repo, "hb1", "shared.txt", "task\n");
    let run_head = commit_on(&repo.root, "anthrex/hb1/integration", "shared.txt", "run\n");

    let files = hand_back(real_git(), &task, &run_head, T).unwrap();
    assert_eq!(files, vec!["shared.txt".to_string()]);
    assert_eq!(out(&task, &["rev-parse", "MERGE_HEAD"]), run_head);
    let text = std::fs::read_to_string(task.join("shared.txt")).unwrap();
    assert!(text.contains("<<<<<<<"), "{text}");
    assert!(text.contains(">>>>>>>"), "{text}");
}

#[test]
fn hand_back_that_is_clean_commits_the_merge() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "hb2", "b.txt", "task\n");
    let task_head = head(&task);
    let run_head = commit_on(&repo.root, "anthrex/hb2/integration", "a.txt", "run\n");

    let files = hand_back(real_git(), &task, &run_head, T).unwrap();
    assert!(files.is_empty(), "{files:?}");
    assert_eq!(parents(&task, "HEAD"), vec![task_head, run_head]);
    assert!(
        !try_git(&task, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .status
            .success()
    );
    assert_eq!(out(&task, &["status", "--porcelain"]), "");
    assert_eq!(
        out(&task, &["symbolic-ref", "--short", "HEAD"]),
        "anthrex/hb2/t1"
    );
}

#[test]
fn hand_back_blocked_by_an_untracked_file_is_an_error() {
    let repo = repo();
    let (_keep, task) = task_worktree(&repo, "hb3", "b.txt", "task\n");
    let task_head = head(&task);
    write(&task, "a.txt", "the worker's own, untracked\n");
    let run_head = commit_on(&repo.root, "anthrex/hb3/integration", "a.txt", "run\n");

    let result = hand_back(real_git(), &task, &run_head, T);
    assert!(result.is_err(), "not a clean hand-back: {result:?}");
    assert_eq!(head(&task), task_head);
    assert_eq!(
        std::fs::read_to_string(task.join("a.txt")).unwrap(),
        "the worker's own, untracked\n"
    );
}

#[test]
fn read_ref_reports_a_moved_branch() {
    let repo = repo();
    let first = head(&repo.root);
    assert_eq!(
        read_ref(real_git(), &repo.root, "refs/heads/main", T).unwrap(),
        Some(first.clone())
    );
    let second = commit_file(&repo.root, "x.txt", "x\n", "x");
    let now = read_ref(real_git(), &repo.root, "refs/heads/main", T).unwrap();
    assert_eq!(now, Some(second));
    assert_ne!(now, Some(first.clone()));
    out(
        &repo.root,
        &["update-ref", "refs/anthrex/salvage/r/t1/1", &first],
    );
    assert_eq!(
        read_ref(real_git(), &repo.root, "refs/anthrex/salvage/r/t1/1", T).unwrap(),
        Some(first)
    );
    assert_eq!(
        read_ref(real_git(), &repo.root, "refs/heads/nope", T).unwrap(),
        None
    );
}

/// A run whose branch has advanced by one merge, so `run_head != base_sha` (a guard
/// that read one ref where it meant the other would otherwise pass every case).
struct Guarded {
    repo: TempRepo,
    base_sha: String,
    run_head: String,
}

const RUN: &str = "anthrex/gd01/integration";

fn guarded() -> Guarded {
    let repo = repo();
    // `base_sha` has a parent, so `main` can be reset to a sibling of it.
    let base_sha = commit_file(&repo.root, "pre.txt", "pre\n", "pre");
    out(&repo.root, &["branch", RUN, &base_sha]);
    let task_head = commit_on(&repo.root, "anthrex/gd01/t1", "t.txt", "task\n");
    let CandidateStep::Tree(tree) =
        merge_tree(real_git(), &repo.root, &base_sha, &task_head, T).unwrap()
    else {
        panic!("clean");
    };
    let run_head = commit_tree(
        real_git(),
        &repo.root,
        &tree,
        &[&base_sha, &task_head],
        "anthrex: merge t1: t",
        T,
    )
    .unwrap();
    assert!(cas_update(real_git(), &repo.root, RUN, &run_head, &base_sha, T).unwrap());
    assert_ne!(run_head, base_sha);
    Guarded {
        repo,
        base_sha,
        run_head,
    }
}

fn guard(g: &Guarded) -> RefCheck {
    guard_refs(
        real_git(),
        &g.repo.root,
        "main",
        &g.base_sha,
        RUN,
        &g.run_head,
        T,
    )
    .unwrap()
}

#[test]
fn guard_refs_classifies_each_case() {
    // Both refs at their recorded values.
    let g = guarded();
    assert_eq!(guard(&g), RefCheck::Ok);

    // A commit on `main`: advanced, not a halt.
    let g = guarded();
    let to = commit_file(&g.repo.root, "other.txt", "o\n", "user work");
    assert_eq!(guard(&g), RefCheck::BaseAdvanced { to, commits: 1 });

    // `main` reset to a sibling of `base_sha`: rewritten.
    let g = guarded();
    out(
        &g.repo.root,
        &["reset", "-q", "--hard", &format!("{}~", g.base_sha)],
    );
    let new = commit_file(&g.repo.root, "sibling.txt", "s\n", "sibling");
    assert_eq!(
        guard(&g),
        RefCheck::Halt {
            reason: format!(
                "refs/heads/main was rewritten: {} is not an ancestor of {}",
                short(&g.base_sha),
                short(&new)
            )
        }
    );

    // `main` deleted.
    let g = guarded();
    out(&g.repo.root, &["checkout", "-q", "--detach"]);
    out(&g.repo.root, &["branch", "-D", "main"]);
    assert_eq!(
        guard(&g),
        RefCheck::Halt {
            reason: "refs/heads/main was deleted".to_string()
        }
    );

    // The run branch moved: named, and checked before the base, which also advanced.
    let g = guarded();
    let moved = commit_on(&g.repo.root, RUN, "intruder.txt", "i\n");
    commit_file(&g.repo.root, "other.txt", "o\n", "user work");
    assert_eq!(
        guard(&g),
        RefCheck::Halt {
            reason: format!(
                "refs/heads/{RUN} moved from {} to {}",
                short(&g.run_head),
                short(&moved)
            )
        }
    );

    // The run branch deleted.
    let g = guarded();
    out(
        &g.repo.root,
        &["update-ref", "-d", &format!("refs/heads/{RUN}")],
    );
    assert_eq!(
        guard(&g),
        RefCheck::Halt {
            reason: format!("refs/heads/{RUN} was deleted")
        }
    );
}

#[test]
fn commits_since_caps_and_counts() {
    let repo = repo();
    let from = head(&repo.root);
    // A colour-forcing config must not reach the lines.
    out(&repo.root, &["config", "color.ui", "always"]);
    let status = std::process::Command::new("sh")
        .arg("-c")
        .arg("for i in $(seq 1 55); do git -c commit.gpgsign=false commit -q --allow-empty -m \"change $i\" || exit 1; done")
        .current_dir(&repo.root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .unwrap();
    assert!(status.success());
    let to = head(&repo.root);

    let (lines, total) =
        commits_since(real_git(), &repo.root, &from, &to, ACCEPT_LIST_MAX, T).unwrap();
    assert_eq!(ACCEPT_LIST_MAX, 50);
    assert_eq!(total, 55);
    assert_eq!(lines.len(), 50);
    let newest = out(&repo.root, &["rev-parse", "HEAD"]);
    assert_eq!(
        lines[0],
        format!("{} Run Tester: change 55", short(&newest))
    );
    let sixth = out(&repo.root, &["rev-parse", "HEAD~49"]);
    assert_eq!(lines[49], format!("{} Run Tester: change 6", short(&sixth)));
    for line in &lines {
        assert!(!line.contains('\u{1b}'), "{line:?}");
    }

    assert_eq!(
        commits_since(real_git(), &repo.root, &to, &to, ACCEPT_LIST_MAX, T).unwrap(),
        (Vec::new(), 0)
    );
}
