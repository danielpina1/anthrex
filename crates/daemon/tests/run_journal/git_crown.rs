//! Milestone 9.5 decision 27 (task M9.5.15, part 5): the reconcile rows of a race's git
//! ops. `CrownRacer` checks the task branch only; a lane's `RemoveWorktree` with
//! `keep_head` is done when its path is gone (or, with `keep_path`, still there) and its
//! salvage ref exists.

use crate::fixture::pend;
use crate::git::World;
use crate::support::run_git::{T, commit_file, out, real_git};
use daemon::run::engine::{OpKind, OpResult};
use daemon::run::git::{remove_worktree, salvage};
use daemon::run::reconcile::Reconciled;

fn crown(w: &World, lane_head: &str) -> OpKind {
    OpKind::CrownRacer {
        root: w.root().to_path_buf(),
        task_branch: format!("anthrex/{}/t1", w.run.id),
        lane_head: lane_head.into(),
        adopt: false,
        checkout: w.run.task_path("t1.a"),
    }
}

#[test]
fn reconcile_crown_done_and_moved() {
    let mut w = World::new();
    let base = w.run.base_sha.clone();
    let lane_head = commit_file(w.root(), "src/lane.txt", "lane\n", "lane");
    out(w.root(), &["reset", "-q", "--hard", &base]);
    let task_branch = format!("refs/heads/anthrex/{}/t1", w.run.id);

    // Absent: the crown did not happen; the op runs again (a compare-and-swap).
    let kind = crown(&w, &lane_head);
    pend(&mut w.run, 1, Some("t1"), kind);
    assert_eq!(w.reconcile(), vec![(1, Reconciled::NotStarted)]);

    // At the lane head: the crown happened.
    out(w.root(), &["update-ref", &task_branch, &lane_head]);
    assert_eq!(
        w.reconcile(),
        vec![(
            1,
            Reconciled::Replay(OpResult::Crowned {
                head: lane_head.clone()
            })
        )]
    );

    // Anywhere else: a moved engine ref, which halts the run.
    out(w.root(), &["update-ref", &task_branch, &base]);
    let answer = w.reconcile();
    assert!(
        matches!(
            &answer[..],
            [(1, Reconciled::Replay(OpResult::RefMoved { .. }))]
        ),
        "{answer:?}"
    );
}

#[test]
fn reconcile_keep_head_removal() {
    let mut w = World::new();
    let run_id = w.run.id.clone();
    let reference = |n: u32| format!("refs/anthrex/salvage/{run_id}/t1/{n}");
    let removal = |w: &World, lane: &str, n: u32, keep_path: bool| OpKind::RemoveWorktree {
        root: w.root().to_path_buf(),
        path: w.run.task_path(lane),
        salvage_ref: reference(n),
        keep_head: true,
        clear_locks: !keep_path,
        keep_path,
    };
    let (a, _) = w.task_with_commit("t1.a", "src/a.txt", "a\n");
    let (b, _) = w.task_with_commit("t1.b", "src/b.txt", "b\n");
    let (c, _) = w.task_with_commit("t1.c", "src/c.txt", "c\n");
    // a: salvaged at its head, then removed. b: salvaged and kept (`keep_path`). c: kept
    // too, but its salvage never ran.
    let saved = salvage(real_git(), &a, &reference(1), "salvage", true, T).unwrap();
    assert_eq!(saved, Some(reference(1)));
    remove_worktree(real_git(), w.root(), &a, T).unwrap();
    let saved = salvage(real_git(), &b, &reference(2), "salvage", true, T).unwrap();
    assert_eq!(saved, Some(reference(2)));
    let kinds = [
        removal(&w, "t1.a", 1, false),
        removal(&w, "t1.b", 2, true),
        removal(&w, "t1.c", 3, true),
    ];
    for (op, kind) in (1..).zip(kinds) {
        pend(&mut w.run, op, Some("t1"), kind);
    }

    let removed = |n: u32| {
        Reconciled::Replay(OpResult::Removed {
            salvage_ref: Some(reference(n)),
            cleared_locks: Vec::new(),
        })
    };
    assert_eq!(
        w.reconcile(),
        vec![
            (1, removed(1)),
            (2, removed(2)),
            (3, Reconciled::NotStarted)
        ]
    );
    assert!(b.is_dir() && c.is_dir(), "a kept checkout is left");
}
