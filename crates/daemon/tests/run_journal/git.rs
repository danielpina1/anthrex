//! Decision 44's reconcile, one test per git row of the Interfaces table, each against
//! a real repository left in the state a crash at that point leaves.

use crate::fixture::{intents, pend, run_at};
use crate::support::recording_git;
use crate::support::run_git::{T, commit_file, head, out, real_git, repo, try_git, write, wt_dir};
use daemon::run::engine::{OpKind, OpResult};
use daemon::run::git::{
    CandidateStep, cas_update, commit_tree, create_run_branch, materialize, merge_tree,
    prepare_worktree, remove_worktree, salvage,
};
use daemon::run::journal::JournalLine;
use daemon::run::model::Run;
use daemon::run::reconcile::{Reconciled, reconcile};
use std::path::{Path, PathBuf};

/// A repository, its engine worktree directory, a data directory and a fresh run on it
/// with its integration worktree in place.
struct World {
    repo: crate::support::TempRepo,
    _wt: tempfile::TempDir,
    _data: tempfile::TempDir,
    run: Run,
}

impl World {
    fn new() -> Self {
        let repo = repo();
        let (wt, wt_path) = wt_dir();
        let data = tempfile::tempdir().unwrap();
        let base = head(&repo.root);
        let run = run_at(data.path(), &repo.root, &wt_path, &base);
        create_run_branch(
            real_git(),
            &repo.root,
            &run.run_branch(),
            &base,
            &run.integration_path(),
            T,
        )
        .unwrap();
        World {
            repo,
            _wt: wt,
            _data: data,
            run,
        }
    }

    fn root(&self) -> &Path {
        &self.repo.root
    }

    /// Task `task`'s worktree on its branch, from the base, with one commit writing
    /// `file`; returns (path, head).
    fn task_with_commit(&self, task: &str, file: &str, content: &str) -> (PathBuf, String) {
        let path = self.run.task_path(task);
        let branch = format!("anthrex/{}/{task}", self.run.id);
        prepare_worktree(
            real_git(),
            self.root(),
            &branch,
            &self.run.base_sha,
            &path,
            T,
        )
        .unwrap();
        let sha = commit_file(&path, file, content, &format!("{task}: {file}"));
        (path, sha)
    }

    /// Reconciles every pending op against its journaled intent only.
    fn reconcile(&self) -> Vec<(u64, Reconciled)> {
        let journal = intents(&self.run);
        reconcile(real_git(), &self.run, &journal, &[], T).ops
    }

    fn merge_candidate(&self, task_head: &str) -> OpKind {
        OpKind::MergeCandidate {
            root: self.root().to_path_buf(),
            integration: self.run.integration_path(),
            run_branch: self.run.run_branch(),
            expected_run_head: self.run.run_head.clone(),
            base_branch: "main".into(),
            expected_base: self.run.base_sha.clone(),
            task_head: task_head.into(),
            message: "merge t1".into(),
            check: None,
            timeout_secs: 60,
            env: Vec::new(),
        }
    }

    /// The candidate commit `merge_tree` + `commit_tree` make for `task_head`, and the
    /// integration worktree detached at it (a check about to run there).
    fn materialized_candidate(&self, task_head: &str) -> String {
        let CandidateStep::Tree(tree) =
            merge_tree(real_git(), self.root(), &self.run.run_head, task_head, T).unwrap()
        else {
            panic!("the candidate merges cleanly")
        };
        let parents = [self.run.run_head.as_str(), task_head];
        let candidate =
            commit_tree(real_git(), self.root(), &tree, &parents, "merge t1", T).unwrap();
        materialize(real_git(), &self.run.integration_path(), &candidate, T).unwrap();
        candidate
    }

    fn integration_branch(&self) -> Option<String> {
        let output = try_git(
            &self.run.integration_path(),
            &["symbolic-ref", "-q", "HEAD"],
        );
        output
            .status
            .success()
            .then(|| String::from_utf8_lossy(&output.stdout).trim().to_string())
    }
}

fn prepare(run: &Run, task: &str, branch: &str, setup: Option<&str>) -> OpKind {
    OpKind::PrepareWorktree {
        root: run.root.clone(),
        branch: branch.into(),
        from: run.base_sha.clone(),
        path: run.task_path(task),
        setup: setup.map(str::to_string),
        env: Vec::new(),
    }
}

#[test]
fn reconcile_prepare_worktree_done_and_not_started() {
    let mut w = World::new();
    let base = w.run.base_sha.clone();
    let run_branch = w.run.run_branch();
    let t1_branch = format!("anthrex/{}/t1", w.run.id);
    let t2_branch = format!("anthrex/{}/t2", w.run.id);
    prepare_worktree(
        real_git(),
        w.root(),
        &t1_branch,
        &base,
        &w.run.task_path("t1"),
        T,
    )
    .unwrap();
    prepare_worktree(
        real_git(),
        w.root(),
        &t2_branch,
        &base,
        &w.run.task_path("t2"),
        T,
    )
    .unwrap();
    // A `worktree add` that died part way: a directory git never registered.
    let partial = w.run.task_path("t3");
    write(&partial, "half-written.txt", "x");

    let integration = OpKind::CreateRunBranch {
        root: w.root().to_path_buf(),
        branch: run_branch.clone(),
        base_sha: base.clone(),
        path: w.run.integration_path(),
        setup: None,
        env: Vec::new(),
    };
    let t1 = prepare(&w.run, "t1", &t1_branch, None);
    let t2 = prepare(&w.run, "t2", &t2_branch, Some("make setup"));
    let t3 = prepare(&w.run, "t3", &format!("anthrex/{}/t3", w.run.id), None);
    let other_branch = prepare(&w.run, "t1", &format!("anthrex/{}/other", w.run.id), None);
    pend(&mut w.run, 1, None, integration);
    pend(&mut w.run, 2, Some("t1"), t1);
    pend(&mut w.run, 3, Some("t2"), t2);
    pend(&mut w.run, 4, Some("t3"), t3);
    pend(&mut w.run, 5, Some("t1"), other_branch);

    let ops = w.reconcile();
    let worktree = |head: &str| Reconciled::Replay(OpResult::Worktree { head: head.into() });
    assert_eq!(
        ops,
        vec![
            (1, worktree(&base)),
            (2, worktree(&base)),
            (3, Reconciled::NotStarted),
            (4, Reconciled::NotStarted),
            (5, Reconciled::NotStarted),
        ],
        "listed on its branch: replayed; with a setup: re-run; missing or on another \
         branch: not started"
    );
    assert!(!partial.exists(), "the partial directory is removed");
    assert!(
        w.run.task_path("t1").is_dir(),
        "a listed worktree is never removed"
    );
    assert!(w.run.task_path("t2").is_dir());
}

#[test]
fn reconcile_merge_candidate_already_advanced() {
    let mut w = World::new();
    let (_, task_head) = w.task_with_commit("t1", "src/a.txt", "a\n");
    let candidate = w.materialized_candidate(&task_head);
    // The compare-and-swap landed; the daemon died before `reattach`.
    assert!(
        cas_update(
            real_git(),
            w.root(),
            &w.run.run_branch(),
            &candidate,
            &w.run.run_head,
            T
        )
        .unwrap()
    );
    let kind = w.merge_candidate(&task_head);
    pend(&mut w.run, 7, Some("t1"), kind);

    assert_eq!(
        w.reconcile(),
        vec![(
            7,
            Reconciled::Replay(OpResult::Merged {
                commit: candidate.clone()
            })
        )]
    );
    assert_eq!(
        w.integration_branch().as_deref(),
        Some(format!("refs/heads/{}", w.run.run_branch()).as_str()),
        "the integration worktree is back on the run branch"
    );
    assert_eq!(head(&w.run.integration_path()), candidate);
}

#[test]
fn reconcile_merge_candidate_not_advanced_reattaches_the_integration_worktree() {
    let mut w = World::new();
    let (_, task_head) = w.task_with_commit("t1", "src/a.txt", "a\n");
    let candidate = w.materialized_candidate(&task_head);
    assert_eq!(w.integration_branch(), None, "detached at the candidate");
    assert_eq!(head(&w.run.integration_path()), candidate);
    let kind = w.merge_candidate(&task_head);
    pend(&mut w.run, 7, Some("t1"), kind);

    assert_eq!(w.reconcile(), vec![(7, Reconciled::NotStarted)]);
    assert_eq!(
        w.integration_branch().as_deref(),
        Some(format!("refs/heads/{}", w.run.run_branch()).as_str()),
        "reattached to the run branch"
    );
    assert_eq!(head(&w.run.integration_path()), w.run.run_head);
    assert!(
        !w.run.integration_path().join("src/a.txt").exists(),
        "the unmerged candidate's files are gone from the integration worktree"
    );
}

#[test]
fn reconcile_merge_candidate_ref_moved() {
    let mut w = World::new();
    let (_, task_head) = w.task_with_commit("t1", "src/a.txt", "a\n");
    // Someone committed on the run branch: its head is neither the expected run head
    // nor a candidate of the expected pair.
    let integration = w.run.integration_path();
    let moved = commit_file(&integration, "intruder.txt", "x\n", "not the engine");
    let kind = w.merge_candidate(&task_head);
    pend(&mut w.run, 7, Some("t1"), kind);

    let ops = w.reconcile();
    assert_eq!(ops.len(), 1);
    let (7, Reconciled::Replay(OpResult::RefMoved { reason })) = &ops[0] else {
        panic!("expected RefMoved, got {ops:?}")
    };
    assert_eq!(
        reason,
        &format!(
            "refs/heads/{} moved from {} to {}",
            w.run.run_branch(),
            &w.run.run_head[..7],
            &moved[..7]
        )
    );
}

#[test]
fn reconcile_hand_back_with_merge_head() {
    let mut w = World::new();
    let (t1, t1_head) = w.task_with_commit("t1", "src/shared.txt", "task one\n");
    let (t2, t2_head) = w.task_with_commit("t2", "src/b.txt", "b\n");
    let (t3, _) = w.task_with_commit("t3", "src/c.txt", "c\n");
    // The run head the tasks are handed: a commit on the run branch touching the file
    // t1 also changed.
    let run_head = commit_file(&w.run.integration_path(), "src/shared.txt", "run\n", "run");
    w.run.run_head = run_head.clone();
    // The daemon died just after each hand-back's merge: t1's conflicted, t2's clean.
    // t3's never started.
    assert!(
        !try_git(&t1, &["merge", "-q", "--no-ff", "--no-edit", &run_head])
            .status
            .success()
    );
    out(&t2, &["merge", "-q", "--no-ff", "--no-edit", &run_head]);
    let t2_merge = head(&t2);
    for (op, task, path) in [(1, "t1", &t1), (2, "t2", &t2), (3, "t3", &t3)] {
        let kind = OpKind::HandBack {
            worktree: path.clone(),
            run_head: run_head.clone(),
            task_head: None,
        };
        pend(&mut w.run, op, Some(task), kind);
    }

    assert_eq!(
        w.reconcile(),
        vec![
            (
                1,
                Reconciled::Replay(OpResult::HandedBack {
                    files: vec!["src/shared.txt".into()],
                    head: Some(t1_head.clone()),
                    onto: Some(t1_head),
                })
            ),
            (
                2,
                Reconciled::Replay(OpResult::HandedBack {
                    files: Vec::new(),
                    head: Some(t2_merge),
                    onto: Some(t2_head),
                })
            ),
            (3, Reconciled::NotStarted),
        ]
    );
    assert!(
        try_git(&t1, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .status
            .success(),
        "the conflict is left for the worker"
    );
}

/// Carry to M8a.21 (ruling on task 15): an `AbortMerge` whose worktree has no
/// `MERGE_HEAD` already happened.
#[test]
fn reconcile_abort_merge_without_merge_head_is_aborted() {
    let mut w = World::new();
    let (t1, _) = w.task_with_commit("t1", "src/shared.txt", "task one\n");
    let (t2, _) = w.task_with_commit("t2", "src/shared.txt", "task two\n");
    let run_head = commit_file(&w.run.integration_path(), "src/shared.txt", "run\n", "run");
    // t2's conflicted hand-back is still there: its abort never ran.
    assert!(
        !try_git(&t2, &["merge", "-q", "--no-ff", "--no-edit", &run_head])
            .status
            .success()
    );
    pend(
        &mut w.run,
        1,
        Some("t1"),
        OpKind::AbortMerge { worktree: t1 },
    );
    pend(
        &mut w.run,
        2,
        Some("t2"),
        OpKind::AbortMerge {
            worktree: t2.clone(),
        },
    );

    assert_eq!(
        w.reconcile(),
        vec![
            (1, Reconciled::Replay(OpResult::MergeAborted)),
            (2, Reconciled::NotStarted),
        ]
    );
    assert!(
        try_git(&t2, &["rev-parse", "-q", "--verify", "MERGE_HEAD"])
            .status
            .success(),
        "reconcile leaves the abort to the re-emitted op"
    );
}

#[test]
fn reconcile_remove_worktree_gone_with_salvage_ref() {
    let mut w = World::new();
    let (t1, _) = w.task_with_commit("t1", "src/a.txt", "a\n");
    let (t2, _) = w.task_with_commit("t2", "src/b.txt", "b\n");
    let (t3, _) = w.task_with_commit("t3", "src/c.txt", "c\n");
    let run_id = w.run.id.clone();
    let salvage_ref = |task: &str| format!("refs/anthrex/{run_id}/salvage/{task}-1");
    // t1 was dirty: salvaged, then removed. t2's directory went but git still lists it
    // (the daemon died between the removal and the prune). t3 was never touched.
    write(&t1, "src/a.txt", "uncommitted\n");
    assert_eq!(
        salvage(real_git(), &t1, &salvage_ref("t1"), "salvage t1", T).unwrap(),
        Some(salvage_ref("t1"))
    );
    remove_worktree(real_git(), w.root(), &t1, T).unwrap();
    std::fs::remove_dir_all(&t2).unwrap();
    for (op, task, path) in [(1, "t1", &t1), (2, "t2", &t2), (3, "t3", &t3)] {
        let kind = OpKind::RemoveWorktree {
            root: w.root().to_path_buf(),
            path: path.clone(),
            salvage_ref: salvage_ref(task),
        };
        pend(&mut w.run, op, Some(task), kind);
    }

    assert_eq!(
        w.reconcile(),
        vec![
            (
                1,
                Reconciled::Replay(OpResult::Removed {
                    salvage_ref: Some(salvage_ref("t1"))
                })
            ),
            (
                2,
                Reconciled::Replay(OpResult::Removed { salvage_ref: None })
            ),
            (3, Reconciled::NotStarted),
        ]
    );
    let listed = w.repo.worktree_paths();
    assert!(
        !listed.contains(&t2),
        "the gone worktree is pruned: {listed:?}"
    );
    assert!(t3.is_dir());
}

fn accept_op(w: &World) -> OpKind {
    OpKind::Accept {
        root: w.root().to_path_buf(),
        base_branch: "main".into(),
        expected_base: w.run.base_sha.clone(),
        run_branch: w.run.run_branch(),
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
    let kind = accept_op(&w);
    pend(&mut w.run, 9, None, kind);

    assert_eq!(
        w.reconcile(),
        vec![(
            9,
            Reconciled::Replay(OpResult::Finished {
                outcome: format!("accepted as {}", &base_head[..7]),
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
