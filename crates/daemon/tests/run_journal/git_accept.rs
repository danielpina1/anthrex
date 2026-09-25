//! Decision 44's reconcile of `run accept`'s row (split from `git.rs` in final fix
//! batch F1c).

use crate::fixture::{intents, pend, run_at};
use crate::git::{World, prepare};
use crate::support::recording_git;
use crate::support::run_git::{T, commit_file, head, out, real_git, try_git};
use daemon::run::engine::{OpKind, OpResult};
use daemon::run::journal::JournalLine;
use daemon::run::reconcile::{Reconciled, reconcile};
use std::path::Path;

pub fn accept_op(w: &World) -> OpKind {
    OpKind::Accept {
        root: w.root().to_path_buf(),
        base_branch: "main".into(),
        expected_base: w.run.base_sha.clone(),
        run_branch: w.run.run_branch(),
        expected_run_head: out(
            w.root(),
            &["rev-parse", &format!("refs/heads/{}", w.run.run_branch())],
        ),
        message: "accept the run".into(),
        worktrees: Vec::new(),
        branch_prefix: format!("anthrex/{}/", w.run.id),
    }
}

#[test]
fn reconcile_accept_already_merged() {
    let mut w = World::new();
    commit_file(
        &w.run.integration_path(),
        "src/a.txt",
        "a\n",
        "the run's work",
    );
    // Accept's merge landed on the base; the daemon died before its `done`.
    let run_ref = format!("refs/heads/{}", w.run.run_branch());
    out(w.root(), &["merge", "-q", "--no-ff", "--no-edit", &run_ref]);
    let base_head = head(w.root());
    // Its clean-up never ran: the task's worktree and branch are still there.
    w.task_with_commit("t1", "src/a.txt", "a\n");
    let kind = accept_op(&w);
    pend(&mut w.run, 9, None, kind);

    // Fix round 1, m2: `OpKind::Accept`'s contract for a clean-up that did not complete:
    // `Finished`, the branches kept, and an outcome that says so.
    assert_eq!(
        w.reconcile(),
        vec![(
            9,
            Reconciled::Replay(OpResult::Finished {
                outcome: format!(
                    "accepted as {}; clean-up did not run before the restart",
                    &base_head[..7]
                ),
                kept_branches: vec![
                    format!("anthrex/{}/integration", w.run.id),
                    format!("anthrex/{}/t1", w.run.id),
                ],
            })
        )]
    );
}

/// Final fix batch F1, finding B-I1: the daemon died during accept's clean-up, after
/// the run branch itself was deleted. The run head the op carried still shows the base
/// has it: the accept is `Finished`, not "request it again" forever.
#[test]
fn reconcile_accept_whose_clean_up_deleted_the_run_branch_is_finished() {
    let mut w = World::new();
    commit_file(
        &w.run.integration_path(),
        "src/a.txt",
        "a\n",
        "the run's work",
    );
    let kind = accept_op(&w);
    let run_ref = format!("refs/heads/{}", w.run.run_branch());
    out(w.root(), &["merge", "-q", "--no-ff", "--no-edit", &run_ref]);
    let base_head = head(w.root());
    let integration = w.run.integration_path();
    out(
        w.root(),
        &[
            "worktree",
            "remove",
            "-f",
            "-f",
            integration.to_str().unwrap(),
        ],
    );
    out(w.root(), &["update-ref", "-d", &run_ref]);
    pend(&mut w.run, 9, None, kind);

    assert_eq!(
        w.reconcile(),
        vec![(
            9,
            Reconciled::Replay(OpResult::Finished {
                outcome: format!(
                    "accepted as {}; clean-up did not run before the restart",
                    &base_head[..7]
                ),
                kept_branches: Vec::new(),
            })
        )]
    );
}

#[test]
fn reconcile_accept_inside_a_conflicted_merge_aborts_it() {
    let mut w = World::new();
    commit_file(
        &w.run.integration_path(),
        "README",
        "the run\n",
        "the run's work",
    );
    commit_file(w.root(), "README", "the user\n", "the user's work");
    let run_ref = format!("refs/heads/{}", w.run.run_branch());
    let user_head = head(w.root());
    assert!(
        !try_git(w.root(), &["merge", "-q", "--no-ff", "--no-edit", &run_ref])
            .status
            .success()
    );
    let kind = accept_op(&w);
    pend(&mut w.run, 9, None, kind);
    // A second run's accept that never started: no merge in progress, nothing merged.
    let mut other = w.run.clone();
    other.pending_ops.clear();

    assert_eq!(w.reconcile(), vec![(9, Reconciled::NotStarted)]);
    assert!(
        !try_git(w.root(), &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .status
            .success(),
        "the half-done accept merge is aborted"
    );
    assert_eq!(head(w.root()), user_head);
    assert_eq!(
        std::fs::read_to_string(w.root().join("README")).unwrap(),
        "the user\n"
    );

    // Not merged, nothing in progress: not started.
    let kind = accept_op(&w);
    pend(&mut other, 10, None, kind);
    let journal = intents(&other);
    assert_eq!(
        reconcile(real_git(), &other, &journal, &[], T).ops,
        vec![(10, Reconciled::NotStarted)]
    );
}

#[test]
fn reconcile_done_line_replays_without_touching_git() {
    let data = tempfile::tempdir().unwrap();
    let logs = tempfile::tempdir().unwrap();
    let git = recording_git(logs.path());
    let root = Path::new("/tmp/no-such-repository");
    let mut run = run_at(
        data.path(),
        root,
        Path::new("/tmp/no-such-wt"),
        &"a".repeat(40),
    );
    let merged = OpResult::Merged {
        commit: "c".repeat(40),
    };
    let prepared = OpResult::Worktree {
        head: "d".repeat(40),
    };
    let candidate = OpKind::MergeCandidate {
        root: root.to_path_buf(),
        integration: run.integration_path(),
        run_branch: run.run_branch(),
        expected_run_head: "a".repeat(40),
        base_branch: "main".into(),
        expected_base: "a".repeat(40),
        task_head: "f".repeat(40),
        message: "m".into(),
        check: None,
        timeout_secs: 1,
        env: Vec::new(),
    };
    let t1 = prepare(&run, "t1", "anthrex/x/t1", None);
    let t2 = prepare(&run, "t2", "anthrex/x/t2", None);
    pend(&mut run, 1, Some("t1"), candidate);
    pend(&mut run, 2, Some("t1"), t1);
    // Op 3 was persisted, but the daemon died before its intent line.
    pend(&mut run, 3, Some("t2"), t2);
    let mut journal = intents(&run);
    journal.retain(|line| line.op() != 3);
    journal.push(JournalLine::Done {
        op: 1,
        result: merged.clone(),
    });
    journal.push(JournalLine::Done {
        op: 2,
        result: prepared.clone(),
    });
    // A journal line for an op the run no longer has pending is ignored.
    journal.push(JournalLine::Done {
        op: 99,
        result: OpResult::RefsOk,
    });

    let got = reconcile(&git.into_os_string(), &run, &journal, &[], T).ops;
    assert_eq!(
        got,
        vec![
            (1, Reconciled::Replay(merged)),
            (2, Reconciled::Replay(prepared)),
            (3, Reconciled::NotStarted),
        ]
    );
    let log = logs.path().join("git.log");
    assert!(
        !log.exists(),
        "no git ran: {}",
        std::fs::read_to_string(&log).unwrap_or_default()
    );
}
