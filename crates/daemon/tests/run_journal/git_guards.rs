//! Fix round 1 of M8a.21: the guards on reconcile's git writes and the table rows the
//! review's surviving mutants showed untested (I3, m1, m4, m5).

use crate::fixture::intents;
use crate::fixture::pend;
use crate::git::{World, prepare};
use crate::git_accept::accept_op;
use crate::support::recording_git;
use crate::support::run_git::{T, commit_file, head, out, real_git, try_git, write};
use daemon::run::engine::{OpKind, OpResult};
use daemon::run::git::{CandidateStep, cas_update, commit_tree, merge_tree};
use daemon::run::reconcile::{Reconciled, reconcile};

fn merge_head(dir: &std::path::Path) -> Option<String> {
    let output = try_git(dir, &["rev-parse", "-q", "--verify", "MERGE_HEAD"]);
    output
        .status
        .success()
        .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// I3 (mutant M4): a merge the user has in progress in their checkout that is not the
/// run's is never aborted, even while an accept is pending.
#[test]
fn reconcile_accept_leaves_the_users_own_merge_alone() {
    let mut w = World::new();
    commit_file(
        &w.run.integration_path(),
        "src/a.txt",
        "a\n",
        "the run's work",
    );
    out(w.root(), &["checkout", "-q", "-b", "topic"]);
    let topic = commit_file(w.root(), "README", "topic\n", "the user's topic");
    out(w.root(), &["checkout", "-q", "main"]);
    commit_file(w.root(), "README", "main\n", "the user's main");
    assert!(
        !try_git(w.root(), &["merge", "-q", "--no-edit", "topic"])
            .status
            .success()
    );
    let kind = accept_op(&w);
    pend(&mut w.run, 9, None, kind);

    assert_eq!(w.reconcile(), vec![(9, Reconciled::NotStarted)]);
    assert_eq!(
        merge_head(w.root()),
        Some(topic),
        "the user's merge is untouched"
    );
}

/// I3 (mutant M5): a merge in a task worktree that is not the run head's hand-back
/// (the worker's own) is not reported as the hand-back.
#[test]
fn reconcile_hand_back_ignores_a_merge_of_something_else() {
    let mut w = World::new();
    let (t1, t1_head) = w.task_with_commit("t1", "src/shared.txt", "task\n");
    let run_head = commit_file(&w.run.integration_path(), "README", "run\n", "run");
    w.run.run_head = run_head.clone();
    out(&t1, &["checkout", "-q", "--detach", "HEAD~1"]);
    let side = commit_file(&t1, "src/shared.txt", "side\n", "side");
    out(&t1, &["checkout", "-q", "--detach", &t1_head]);
    assert!(
        !try_git(&t1, &["merge", "-q", "--no-edit", &side])
            .status
            .success()
    );
    let kind = OpKind::HandBack {
        worktree: t1.clone(),
        run_head,
        task_head: None,
    };
    pend(&mut w.run, 1, Some("t1"), kind);

    assert_eq!(w.reconcile(), vec![(1, Reconciled::NotStarted)]);
    assert_eq!(merge_head(&t1), Some(side));
}

/// m1: reconcile removes nothing a corrupted `run.json` names outside the run's own
/// worktree root. Final fix batch F1c (3a): reconcile removes no partial task checkout
/// at all (the re-issued op finishes it), so nothing is removed anywhere.
#[test]
fn reconcile_never_removes_a_directory_outside_the_runs_worktrees() {
    let mut w = World::new();
    let outside = tempfile::tempdir().unwrap();
    let victim = outside.path().join("precious");
    write(&victim, "work.txt", "keep me");
    let mut kind = prepare(&w.run, "t1", &format!("anthrex/{}/t1", w.run.id), None);
    if let OpKind::PrepareWorktree { path, .. } = &mut kind {
        *path = victim.clone();
    }
    pend(&mut w.run, 1, Some("t1"), kind);
    // `..` out of the run's root is outside too.
    let escape = w.run.task_path("..").join("..").join("elsewhere");
    write(&escape, "work.txt", "keep me too");
    let mut kind = prepare(&w.run, "t2", &format!("anthrex/{}/t2", w.run.id), None);
    if let OpKind::PrepareWorktree { path, .. } = &mut kind {
        *path = escape.clone();
    }
    pend(&mut w.run, 2, Some("t2"), kind);

    let journal = intents(&w.run);
    let result = reconcile(real_git(), &w.run, &journal, &[], T);
    assert_eq!(
        result.ops,
        vec![(1, Reconciled::NotStarted), (2, Reconciled::NotStarted)]
    );
    assert!(victim.join("work.txt").is_file(), "never removed");
    assert!(escape.join("work.txt").is_file(), "never removed");
    assert!(
        !result.notes.iter().any(|n| n.contains("removed")),
        "{:?}",
        result.notes
    );
}

/// m4 (mutant M2): a worktree git lists on its branch whose directory is gone is not a
/// finished `PrepareWorktree`.
#[test]
fn reconcile_prepare_worktree_whose_directory_is_gone_is_not_started() {
    let mut w = World::new();
    let branch = format!("anthrex/{}/t1", w.run.id);
    let path = w.run.task_path("t1");
    w.prepare_task("t1", &branch, &w.run.base_sha.clone());
    std::fs::remove_dir_all(&path).unwrap();
    let kind = prepare(&w.run, "t1", &branch, None);
    pend(&mut w.run, 1, Some("t1"), kind);

    assert_eq!(w.reconcile(), vec![(1, Reconciled::NotStarted)]);
}

/// m4 (mutant M3): a merge commit of the expected pair in the other order is not the
/// engine's candidate.
#[test]
fn reconcile_merge_candidate_with_swapped_parents_is_ref_moved() {
    let mut w = World::new();
    let (_, task_head) = w.task_with_commit("t1", "src/a.txt", "a\n");
    let CandidateStep::Tree(tree) =
        merge_tree(real_git(), w.root(), &w.run.run_head, &task_head, T).unwrap()
    else {
        panic!("clean")
    };
    let parents = [task_head.as_str(), w.run.run_head.as_str()];
    let swapped = commit_tree(real_git(), w.root(), &tree, &parents, "swapped", T).unwrap();
    assert!(
        cas_update(
            real_git(),
            w.root(),
            &w.run.run_branch(),
            &swapped,
            &w.run.run_head,
            T
        )
        .unwrap()
    );
    let kind = w.merge_candidate(&task_head);
    pend(&mut w.run, 7, Some("t1"), kind);

    let ops = w.reconcile();
    assert!(
        matches!(
            &ops[..],
            [(7, Reconciled::Replay(OpResult::RefMoved { .. }))]
        ),
        "{ops:?}"
    );
}

/// m5: the abort in the user's checkout runs as `git::accept` runs there: the scrubbed
/// environment and `--no-optional-locks`, without decision 18's signing override; and,
/// since final fix batch F1 (C-C1), with no hook and no fsmonitor.
#[test]
fn the_accept_abort_in_the_users_checkout_has_no_engine_write_flags() {
    let mut w = World::new();
    commit_file(
        &w.run.integration_path(),
        "README",
        "the run\n",
        "the run's work",
    );
    commit_file(w.root(), "README", "the user\n", "the user's work");
    let run_ref = format!("refs/heads/{}", w.run.run_branch());
    assert!(
        !try_git(w.root(), &["merge", "-q", "--no-ff", "--no-edit", &run_ref])
            .status
            .success()
    );
    let user_head = head(w.root());
    let kind = accept_op(&w);
    pend(&mut w.run, 9, None, kind);
    let logs = tempfile::tempdir().unwrap();
    let git = recording_git(logs.path());

    let journal = intents(&w.run);
    let ops = reconcile(git.as_os_str(), &w.run, &journal, &[], T).ops;
    assert_eq!(ops, vec![(9, Reconciled::NotStarted)]);
    assert_eq!(merge_head(w.root()), None);
    assert_eq!(head(w.root()), user_head);
    let log = std::fs::read_to_string(logs.path().join("git.log")).unwrap();
    let abort = log
        .lines()
        .find(|l| l.starts_with("argv") && l.ends_with("\tmerge\t--abort"))
        .unwrap_or_else(|| panic!("no abort in {log}"));
    assert!(abort.contains("\t--no-optional-locks\t"), "{abort}");
    assert!(abort.contains("\tcore.hooksPath=/dev/null\t"), "{abort}");
    assert!(abort.contains("\tcore.fsmonitor=false\t"), "{abort}");
    assert!(!abort.contains("commit.gpgSign"), "{abort}");
    assert!(
        !log.lines().any(|l| l.starts_with("env\tGIT_DIR=")),
        "scrubbed environment: {log}"
    );
}

/// Final fix batch F1, fix round 1 (review N6, D-12's third site): reconciling a
/// `PrepareWorktree` that never registered its worktree forgets none of the user's own
/// worktrees whose directories are merely missing.
#[test]
fn reconcile_prepare_worktree_never_prunes_the_users_worktrees() {
    let mut w = World::new();
    let theirs = tempfile::tempdir().unwrap();
    let users = theirs.path().canonicalize().unwrap().join("mine");
    out(
        w.root(),
        &["worktree", "add", "-q", "--detach", users.to_str().unwrap()],
    );
    std::fs::rename(&users, users.with_file_name("elsewhere")).unwrap();
    let branch = format!("anthrex/{}/t1", w.run.id);
    let kind = prepare(&w.run, "t1", &branch, None);
    pend(&mut w.run, 1, Some("t1"), kind);

    assert_eq!(w.reconcile(), vec![(1, Reconciled::NotStarted)]);
    let list = out(w.root(), &["worktree", "list", "--porcelain"]);
    assert!(
        list.contains(&format!("worktree {}", users.display())),
        "the user's worktree was pruned: {list}"
    );
}

/// Final fix batch F1, fix round 1 (review N4): an `Accept` intent journaled before the
/// op carried its run head still loads, and reconcile then reads the run branch.
#[test]
fn an_accept_intent_without_its_run_head_still_loads() {
    let w = World::new();
    let mut value = serde_json::to_value(accept_op(&w)).unwrap();
    value["Accept"]
        .as_object_mut()
        .unwrap()
        .remove("expected_run_head")
        .unwrap();
    let kind: OpKind = serde_json::from_value(value).unwrap();
    let OpKind::Accept {
        expected_run_head, ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(expected_run_head, "");
}
