//! M8a final fix batch F1, fix round 3: the hand-back now merges with `--no-commit`,
//! commits with `commit-tree` and moves the task's branch with a compare-and-swap
//! `update-ref`, so a crash can leave two new states. Reconcile finishes the first and
//! undoes the second, writing no ref in either.

use crate::fixture::pend;
use crate::git::World;
use crate::support::run_git::{commit_file, head, out, try_git};
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
