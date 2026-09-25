//! M8a final fix batch F1, fix round 3: the hand-back now merges with `--no-commit`,
//! commits with `commit-tree` and moves the task's branch with a compare-and-swap
//! `update-ref`, so a crash can leave two new states. Reconcile finishes the first and
//! undoes the second, writing no ref in either. Final fix batch F1b adds the move of the
//! worktree's detached `HEAD` after the branch's, and a crash point between them.

use crate::fixture::pend;
use crate::git::World;
use crate::support::run_git::{T, commit_file, head, out, real_git, try_git};
use daemon::run::engine::{OpKind, OpResult};
use daemon::run::reconcile::Reconciled;

fn merge_head_exists(dir: &std::path::Path) -> bool {
    try_git(dir, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
        .status
        .success()
}

/// The daemon died after the task branch took the merge commit, before `merge --quit`.
#[test]
fn reconcile_a_committed_hand_back_left_mid_merge_finishes_it() {
    let mut w = World::new();
    let (t1, t1_head) = w.task_with_commit("t1", "src/a.txt", "task\n");
    let run_head = commit_file(&w.run.integration_path(), "src/r.txt", "run\n", "run");
    w.run.run_head = run_head.clone();
    out(&t1, &["merge", "-q", "--no-ff", "--no-commit", &run_head]);
    let tree = out(&t1, &["write-tree"]);
    let commit = out(
        &t1,
        &[
            "commit-tree",
            &tree,
            "-p",
            &t1_head,
            "-p",
            &run_head,
            "-m",
            "merge",
        ],
    );
    let own = format!("refs/heads/anthrex/{}/t1", w.run.id);
    out(&t1, &["update-ref", &own, &commit, &t1_head]);
    // Final fix batch F1b: the worktree is detached, so its `HEAD` moved too.
    out(
        &t1,
        &["update-ref", "--no-deref", "HEAD", &commit, &t1_head],
    );
    pend(
        &mut w.run,
        1,
        Some("t1"),
        OpKind::HandBack {
            worktree: t1.clone(),
            run_head,
            task_head: None,
        },
    );

    assert_eq!(
        w.reconcile(),
        vec![(
            1,
            Reconciled::Replay(OpResult::HandedBack {
                files: Vec::new(),
                head: Some(commit.clone()),
                onto: Some(t1_head),
            })
        )]
    );
    assert!(!merge_head_exists(&t1), "the merge state was left");
    assert_eq!(head(&t1), commit);
    assert_eq!(out(&t1, &["status", "--porcelain"]), "");
}

/// The daemon died after the clean `--no-commit` merge, before its commit: undone, and
/// the hand-back runs again.
#[test]
fn reconcile_an_uncommitted_hand_back_merge_undoes_it() {
    let mut w = World::new();
    let base = head(w.root());
    let (t1, t1_head) = w.task_with_commit("t1", "src/a.txt", "task\n");
    let run_head = commit_file(&w.run.integration_path(), "src/r.txt", "run\n", "run");
    w.run.run_head = run_head.clone();
    out(&t1, &["merge", "-q", "--no-ff", "--no-commit", &run_head]);
    pend(
        &mut w.run,
        1,
        Some("t1"),
        OpKind::HandBack {
            worktree: t1.clone(),
            run_head,
            task_head: None,
        },
    );

    assert_eq!(w.reconcile(), vec![(1, Reconciled::NotStarted)]);
    assert!(!merge_head_exists(&t1), "the merge was left in progress");
    assert_eq!(head(&t1), t1_head);
    assert_eq!(out(&t1, &["status", "--porcelain"]), "");
    assert!(!t1.join("src/r.txt").exists());
    assert_eq!(out(w.root(), &["rev-parse", "main"]), base);
}

/// Fix round 4, S3: a conflicted hand-back of the run head that the worker resolved and
/// staged, uncommitted, looks like the engine's clean leftover (`MERGE_HEAD` is the run
/// head, nothing is unmerged, `HEAD^2` is not the run head). It is the worker's work:
/// reconcile must leave it, never reset it away.
#[test]
fn reconcile_leaves_a_resolved_conflicted_hand_back_alone() {
    let mut w = World::new();
    let (t1, t1_head) = w.task_with_commit("t1", "src/a.txt", "task\n");
    let run_head = commit_file(&w.run.integration_path(), "src/a.txt", "run\n", "run");
    w.run.run_head = run_head.clone();
    let merge = try_git(&t1, &["merge", "-q", "--no-ff", "--no-commit", &run_head]);
    assert!(!merge.status.success(), "the merge did not conflict");
    std::fs::write(t1.join("src/a.txt"), "resolved\n").unwrap();
    out(&t1, &["add", "src/a.txt"]);
    pend(
        &mut w.run,
        1,
        Some("t1"),
        OpKind::HandBack {
            worktree: t1.clone(),
            run_head,
            task_head: None,
        },
    );

    assert_eq!(w.reconcile(), vec![(1, Reconciled::NotStarted)]);
    assert!(merge_head_exists(&t1), "the worker's merge was undone");
    assert_eq!(head(&t1), t1_head);
    assert_eq!(
        std::fs::read_to_string(t1.join("src/a.txt")).unwrap(),
        "resolved\n",
        "the worker's resolution was wiped"
    );
    assert_eq!(out(&t1, &["diff", "--cached", "--name-only"]), "src/a.txt");
}

// Final fix batch F1, fix round 5: the hand-back runs no `git merge`. A clean one is
// `merge-tree`, `commit-tree`, the branch's compare-and-swap, then `read-tree`; a
// conflicted one is `read-tree` (the marker tree), the merge state files (`MERGE_MSG`,
// `MERGE_MODE`, then `MERGE_HEAD`), then `update-index` (the unmerged stages). One test
// per point a crash can stop it at.

fn pend_hand_back(w: &mut World, t1: &std::path::Path, run_head: &str) {
    pend(
        &mut w.run,
        1,
        Some("t1"),
        OpKind::HandBack {
            worktree: t1.to_path_buf(),
            run_head: run_head.to_string(),
            task_head: None,
        },
    );
}

/// `merge-tree --write-tree`'s tree for `ours` and `theirs`, conflict or not.
fn merge_tree(dir: &std::path::Path, ours: &str, theirs: &str) -> String {
    let output = try_git(dir, &["merge-tree", "--write-tree", ours, theirs]);
    let text = String::from_utf8(output.stdout).unwrap();
    text.lines().next().unwrap().to_string()
}

fn admin(dir: &std::path::Path) -> std::path::PathBuf {
    std::path::PathBuf::from(out(dir, &["rev-parse", "--absolute-git-dir"]))
}

/// Died after `commit-tree`, before the branch moved: nothing to see, nothing done.
#[test]
fn reconcile_a_clean_hand_back_stopped_before_its_branch_moved_is_not_started() {
    let mut w = World::new();
    let (t1, t1_head) = w.task_with_commit("t1", "src/a.txt", "task\n");
    let run_head = commit_file(&w.run.integration_path(), "src/r.txt", "run\n", "run");
    w.run.run_head = run_head.clone();
    let tree = merge_tree(&t1, &t1_head, &run_head);
    out(
        &t1,
        &[
            "commit-tree",
            &tree,
            "-p",
            &t1_head,
            "-p",
            &run_head,
            "-m",
            "m",
        ],
    );
    pend_hand_back(&mut w, &t1, &run_head);

    assert_eq!(w.reconcile(), vec![(1, Reconciled::NotStarted)]);
    assert_eq!(head(&t1), t1_head);
    assert_eq!(out(&t1, &["status", "--porcelain"]), "");
}

/// Final fix batch F1b: died after the branch's compare-and-swap, before the
/// worktree's `HEAD` moved. The branch follows the worker's `HEAD` (reconcile records
/// it back), and the hand-back runs again.
#[test]
fn reconcile_a_clean_hand_back_stopped_before_its_head_moved_is_not_started() {
    let mut w = World::new();
    let (t1, t1_head) = w.task_with_commit("t1", "src/a.txt", "task\n");
    let run_head = commit_file(&w.run.integration_path(), "src/r.txt", "run\n", "run");
    w.run.run_head = run_head.clone();
    let tree = merge_tree(&t1, &t1_head, &run_head);
    let commit = out(
        &t1,
        &[
            "commit-tree",
            &tree,
            "-p",
            &t1_head,
            "-p",
            &run_head,
            "-m",
            "m",
        ],
    );
    let own = format!("refs/heads/anthrex/{}/t1", w.run.id);
    out(&t1, &["update-ref", "--no-deref", &own, &commit, &t1_head]);
    pend_hand_back(&mut w, &t1, &run_head);

    assert_eq!(w.reconcile(), vec![(1, Reconciled::NotStarted)]);
    assert_eq!(head(&t1), t1_head);
    assert_eq!(out(&t1, &["rev-parse", &own]), t1_head);
    assert_eq!(out(&t1, &["status", "--porcelain"]), "");
}

/// Died after the branch's compare-and-swap and the move of `HEAD` (F1b), before
/// `read-tree`: `HEAD` and the branch hold the merge commit and the index and files are
/// still the task's. Reconcile moves them on and replays the clean hand-back.
#[test]
fn reconcile_a_clean_hand_back_whose_files_never_followed_finishes_it() {
    let mut w = World::new();
    let (t1, t1_head) = w.task_with_commit("t1", "src/a.txt", "task\n");
    let run_head = commit_file(&w.run.integration_path(), "src/r.txt", "run\n", "run");
    w.run.run_head = run_head.clone();
    let tree = merge_tree(&t1, &t1_head, &run_head);
    let commit = out(
        &t1,
        &[
            "commit-tree",
            &tree,
            "-p",
            &t1_head,
            "-p",
            &run_head,
            "-m",
            "m",
        ],
    );
    let own = format!("refs/heads/anthrex/{}/t1", w.run.id);
    out(&t1, &["update-ref", "--no-deref", &own, &commit, &t1_head]);
    out(
        &t1,
        &["update-ref", "--no-deref", "HEAD", &commit, &t1_head],
    );
    pend_hand_back(&mut w, &t1, &run_head);

    assert_eq!(
        w.reconcile(),
        vec![(
            1,
            Reconciled::Replay(OpResult::HandedBack {
                files: Vec::new(),
                head: Some(commit.clone()),
                onto: Some(t1_head),
            })
        )]
    );
    assert_eq!(head(&t1), commit);
    assert_eq!(
        out(&t1, &["status", "--porcelain"]),
        "",
        "files left behind"
    );
    assert!(t1.join("src/r.txt").exists());
}

/// Died after the marker tree went in, with only `MERGE_MSG` written: undone (index,
/// files and the stray message), and the hand-back runs again.
#[test]
fn reconcile_a_conflicted_hand_back_stopped_before_merge_head_undoes_it() {
    let mut w = World::new();
    let (t1, t1_head) = w.task_with_commit("t1", "src/a.txt", "task\n");
    let run_head = commit_file(&w.run.integration_path(), "src/a.txt", "run\n", "run");
    w.run.run_head = run_head.clone();
    let tree = merge_tree(&t1, &t1_head, &run_head);
    out(&t1, &["read-tree", "-m", "-u", &t1_head, &tree]);
    std::fs::write(admin(&t1).join("MERGE_MSG"), "Merge commit\n").unwrap();
    pend_hand_back(&mut w, &t1, &run_head);

    assert_eq!(w.reconcile(), vec![(1, Reconciled::NotStarted)]);
    assert_eq!(head(&t1), t1_head);
    assert_eq!(
        out(&t1, &["status", "--porcelain"]),
        "",
        "markers left behind"
    );
    assert_eq!(
        std::fs::read_to_string(t1.join("src/a.txt")).unwrap(),
        "task\n"
    );
    assert!(
        !admin(&t1).join("MERGE_MSG").exists(),
        "the message was left"
    );
}

/// Died after `MERGE_HEAD` was written, before the conflicts were staged (nothing
/// unmerged; the index is the marker tree): undone, and the hand-back runs again.
#[test]
fn reconcile_a_conflicted_hand_back_stopped_before_its_stages_undoes_it() {
    let mut w = World::new();
    let (t1, t1_head) = w.task_with_commit("t1", "src/a.txt", "task\n");
    let run_head = commit_file(&w.run.integration_path(), "src/a.txt", "run\n", "run");
    w.run.run_head = run_head.clone();
    let tree = merge_tree(&t1, &t1_head, &run_head);
    out(&t1, &["read-tree", "-m", "-u", &t1_head, &tree]);
    let admin = admin(&t1);
    std::fs::write(admin.join("MERGE_MSG"), "Merge commit\n").unwrap();
    std::fs::write(admin.join("MERGE_MODE"), "no-ff").unwrap();
    std::fs::write(admin.join("MERGE_HEAD"), format!("{run_head}\n")).unwrap();
    pend_hand_back(&mut w, &t1, &run_head);

    assert_eq!(w.reconcile(), vec![(1, Reconciled::NotStarted)]);
    assert!(!merge_head_exists(&t1), "the merge was left in progress");
    assert_eq!(head(&t1), t1_head);
    assert_eq!(
        out(&t1, &["status", "--porcelain"]),
        "",
        "markers left behind"
    );
}

/// Died after the last step (or just before its `done` line): the conflicted hand-back
/// the engine itself made is replayed with its files.
#[test]
fn reconcile_a_finished_conflicted_hand_back_replays_it() {
    let mut w = World::new();
    let (t1, t1_head) = w.task_with_commit("t1", "src/a.txt", "task\n");
    let run_head = commit_file(&w.run.integration_path(), "src/a.txt", "run\n", "run");
    w.run.run_head = run_head.clone();
    let back = daemon::run::git::hand_back(real_git(), &t1, &run_head, T).unwrap();
    assert_eq!(back.files, vec!["src/a.txt".to_string()]);
    pend_hand_back(&mut w, &t1, &run_head);

    assert_eq!(
        w.reconcile(),
        vec![(
            1,
            Reconciled::Replay(OpResult::HandedBack {
                files: vec!["src/a.txt".to_string()],
                head: Some(t1_head.clone()),
                onto: Some(t1_head),
            })
        )]
    );
    assert!(merge_head_exists(&t1));
}
