//! Controller ruling C-15 (M-1): the reconcile row of a paired `MergeCandidate` (a
//! multi-stage run's highest stage and `integration`, moved in one transaction) checks
//! `integration` too. A real repository of the test's own.

use std::ffi::OsStr;
use std::time::Duration;

use super::{Reconciled, reconcile};
use crate::run::engine::{OpKind, OpResult};
use crate::run::journal::JournalLine;
use crate::run::model::PendingOp;
use crate::run::test_support::{EXAMPLE_PLAN, run_ok};

const STAGE: &str = "anthrex/r1/stage-2";
const INTEGRATION: &str = "refs/heads/anthrex/r1/integration";

fn git(root: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_PREFIX")
        .output()
        .unwrap();
    assert!(out.status.success(), "git {args:?}: {out:?}");
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

/// The one pending paired candidate of `task` onto `base`, reconciled.
fn reconciled(root: &std::path::Path, base: &str, task: &str) -> Reconciled {
    let mut run = run_ok(EXAMPLE_PLAN);
    let kind = OpKind::MergeCandidate {
        root: root.to_path_buf(),
        integration: root.join("no-integration-worktree"),
        run_branch: STAGE.into(),
        expected_run_head: base.into(),
        base_branch: "main".into(),
        expected_base: base.into(),
        task_head: task.into(),
        message: "m".into(),
        check: None,
        timeout_secs: 1,
        env: Vec::new(),
        guarded: Vec::new(),
        also_integration: true,
        tier: None,
    };
    run.pending_ops.insert(
        1,
        PendingOp {
            op: 1,
            task_id: None,
            kind: kind.clone(),
        },
    );
    let journal = [JournalLine::Intent { op: 1, kind }];
    let git = OsStr::new("git");
    let out = reconcile(git, &run, &journal, &[], Duration::from_secs(20));
    out.ops[0].1.clone()
}

#[test]
fn a_torn_paired_merge_is_a_moved_integration() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.name", "Stage Test"]);
    git(root, &["config", "user.email", "stage@test"]);
    git(root, &["commit", "-q", "--allow-empty", "-m", "base"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    git(root, &["commit", "-q", "--allow-empty", "-m", "task"]);
    let task = git(root, &["rev-parse", "HEAD"]);
    git(root, &["branch", STAGE, &base]);
    // Only `integration` moved: the stage ref is where the op found it.
    git(root, &["update-ref", INTEGRATION, &task]);
    let moved = |from: &str, to: &str| {
        Reconciled::Replay(OpResult::RefMoved {
            reason: format!("{INTEGRATION} moved from {} to {}", &from[..7], &to[..7]),
        })
    };
    assert_eq!(reconciled(root, &base, &task), moved(&base, &task));
    // The stage ref landed the candidate, but `integration` did not follow it.
    let tree = git(root, &["rev-parse", &format!("{task}^{{tree}}")]);
    let merged = git(
        root,
        &["commit-tree", &tree, "-p", &base, "-p", &task, "-m", "m"],
    );
    git(
        root,
        &["update-ref", &format!("refs/heads/{STAGE}"), &merged],
    );
    git(root, &["update-ref", INTEGRATION, &base]);
    assert_eq!(reconciled(root, &base, &task), moved(&merged, &base));
    // Both where the transaction leaves them: merged.
    git(root, &["update-ref", INTEGRATION, &merged]);
    assert_eq!(
        reconciled(root, &base, &task),
        Reconciled::Replay(OpResult::Merged {
            commit: merged,
            tier: None
        })
    );
}

/// Milestone 9.1 task M9.1.17 (decision 53): a `Propagate` is reconciled as a merge
/// candidate whose second parent is the lower stage's head. One whose commit reached
/// both refs is `Merged`; one that reached neither is not started, and issued again.
#[test]
fn propagate_is_journaled_and_reconciled() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "user.name", "Stage Test"]);
    git(root, &["config", "user.email", "stage@test"]);
    git(root, &["commit", "-q", "--allow-empty", "-m", "base"]);
    let base = git(root, &["rev-parse", "HEAD"]);
    git(root, &["commit", "-q", "--allow-empty", "-m", "stage 1"]);
    let lower = git(root, &["rev-parse", "HEAD"]);
    git(root, &["branch", STAGE, &base]);
    git(root, &["update-ref", INTEGRATION, &base]);
    let spec = crate::run::model::PropagateSpec {
        root: root.to_path_buf(),
        integration: root.join("no-integration-worktree"),
        from: 1,
        to: 2,
        from_head: lower.clone(),
        to_branch: STAGE.into(),
        expected_to_head: base.clone(),
        also_integration: true,
        base_branch: "main".into(),
        expected_base: base.clone(),
        guarded: Vec::new(),
        message: "anthrex: propagate stage-1 into stage-2".into(),
        tier: None,
        check: None,
        timeout_secs: 1,
        env: Vec::new(),
        tasks: Default::default(),
    };
    let kind = OpKind::Propagate(Box::new(spec));
    let reconcile_one = || {
        let mut run = run_ok(EXAMPLE_PLAN);
        run.pending_ops.insert(
            1,
            PendingOp {
                op: 1,
                task_id: None,
                kind: kind.clone(),
            },
        );
        let journal = [JournalLine::Intent {
            op: 1,
            kind: kind.clone(),
        }];
        let out = reconcile(
            OsStr::new("git"),
            &run,
            &journal,
            &[],
            Duration::from_secs(20),
        );
        out.ops[0].1.clone()
    };
    // Neither ref moved: not started.
    assert_eq!(reconcile_one(), Reconciled::NotStarted);
    // Both refs hold the propagate, parents `[to head, from head]`: it landed.
    let tree = git(root, &["rev-parse", &format!("{lower}^{{tree}}")]);
    let merged = git(
        root,
        &["commit-tree", &tree, "-p", &base, "-p", &lower, "-m", "m"],
    );
    git(
        root,
        &["update-ref", &format!("refs/heads/{STAGE}"), &merged],
    );
    git(root, &["update-ref", INTEGRATION, &merged]);
    assert_eq!(
        reconcile_one(),
        Reconciled::Replay(OpResult::Merged {
            commit: merged,
            tier: None
        })
    );
}
