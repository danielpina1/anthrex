//! Milestone 9.5 decisions 21 and 22 (I/O, task M9.5.15): a race lane's ops, executed.
//! `CrownRacer` is one compare-and-swap creating the task branch at the lane's head
//! (`git::crown`), then, on `Crowned`, the lane checkout's re-pin: its own branch
//! becomes the task branch, so every later import moves the task branch and the lane
//! branch stays where the crown found it (review ruling I2). A lane's `RemoveWorktree`
//! clears the stale locks a stopped racer can leave in its checkout's own git
//! directory, salvages (a clean checkout too, at its head, with `keep_head`), and
//! removes the checkout unless `keep_path`. No kill, signal or probe (ruling RR-2):
//! the engine emits the removal only after the racer's exit.
//!
//! Every git write goes through the run's `GitQueue`; the lock clearing runs on a
//! blocking thread. No lock is held here.

use std::sync::Arc;

use proto::RaceLane;

use super::super::{OpCtx, RunService, cleanup};
use super::{blocking, failed};
use crate::run::engine::{OpKind, OpResult};
use crate::run::git;
use crate::run::model::{Run, lane_checkout};
use crate::worktree::pinned::repin_own;

/// `OpKind::CrownRacer`'s executor contract. The re-pin runs inside the same queued
/// write, right after the compare-and-swap, on its blocking thread.
pub(super) async fn crown_racer(service: &Arc<RunService>, ctx: &OpCtx, kind: OpKind) -> OpResult {
    let OpKind::CrownRacer {
        root,
        task_branch,
        lane_head,
        checkout,
        ..
    } = kind
    else {
        unreachable!("crown_racer takes a CrownRacer");
    };
    let crowned = service
        .write(ctx, move |g, t| {
            let result = git::crown(g, &root, &task_branch, &lane_head, t)?;
            if matches!(result, OpResult::Crowned { .. }) {
                repin_own(&checkout, &format!("refs/heads/{task_branch}"));
            }
            Ok(result)
        })
        .await;
    crowned.unwrap_or_else(failed)
}

/// `RemoveWorktree`, a race lane's variants included: `Removed { salvage_ref,
/// cleared_locks }`. Without them it is decision 20's salvage and removal, unchanged.
pub(super) async fn remove_lane(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    kind: OpKind,
) -> Result<OpResult, String> {
    let OpKind::RemoveWorktree {
        root,
        path,
        salvage_ref: reference,
        keep_head,
        clear_locks,
        keep_path,
    } = kind
    else {
        unreachable!("remove_lane takes a RemoveWorktree");
    };
    let mut cleared_locks = Vec::new();
    if clear_locks && path.exists() {
        let repo = git::Repo::at(&git::checkout_repo_dir(&ctx.data_dir, &path));
        let git_dir = repo.git_dir();
        cleared_locks = blocking(move || {
            git::clear_stale_locks(&git_dir).map_err(|error| {
                format!(
                    "cannot clear the stale locks in {}: {error}",
                    git_dir.display()
                )
            })
        })
        .await?;
    }
    let salvage_ref = if keep_path {
        cleanup::salvage_in(service, ctx, path, reference, keep_head).await?
    } else {
        cleanup::remove_keeping(service, ctx, root, path, reference, keep_head).await?
    };
    Ok(OpResult::Removed {
        salvage_ref,
        cleared_locks,
    })
}

/// The checkout a session of task `task` works in (ruling RR-1): lane `lane`'s own
/// while it races, else the task's (`Task::checkout_name`, the crowned lane's once
/// there is one).
pub(super) fn checkout_of(run: &Run, task: &str, lane: Option<RaceLane>) -> String {
    match (lane, run.task(task)) {
        (Some(lane), _) => lane_checkout(task, lane),
        (None, Some(t)) => t.checkout_name(),
        (None, None) => task.to_string(),
    }
}

#[cfg(test)]
#[path = "lane_ops_tests.rs"]
mod tests;
