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

use proto::{AgentRole, RaceLane};

use super::super::{OpCtx, RunService, cleanup};
use super::failed;
use crate::run::engine::{OpKind, OpResult};
use crate::run::git;
use crate::run::model::Run;
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
    // Ruling RR-2: with `keep_path` the racer did not exit, so a lock there may be a
    // live process's; it is never cleared. Otherwise the clearing is the first write
    // in the repository's queue (review m2).
    let mut cleared_locks = Vec::new();
    if clear_locks && !keep_path && path.exists() {
        let git_dir = git::Repo::at(&git::checkout_repo_dir(&ctx.data_dir, &path)).git_dir();
        cleared_locks = service
            .write(ctx, move |_, _| {
                git::clear_stale_locks(&git_dir).map_err(|error| {
                    format!(
                        "cannot clear the stale locks in {}: {error}",
                        git_dir.display()
                    )
                })
            })
            .await?;
    }
    let salvaged = if keep_path {
        cleanup::salvage_in(service, ctx, path, reference, keep_head).await
    } else {
        cleanup::remove_keeping(service, ctx, root, path, reference, keep_head).await
    };
    // Review m3: the locks are gone whatever follows; a failure names them.
    let salvage_ref = salvaged.map_err(|error| match cleared_locks.is_empty() {
        true => error,
        false => format!("{error} (after it {})", cleared_text(&cleared_locks)),
    })?;
    Ok(OpResult::Removed {
        salvage_ref,
        cleared_locks,
    })
}

/// Decision 22's record of cleared locks: `removed a stale index.lock left by the
/// stopped racer, removed a stale HEAD.lock …`.
fn cleared_text(locks: &[String]) -> String {
    let each: Vec<String> = locks
        .iter()
        .map(|lock| format!("removed a stale {lock} left by the stopped racer"))
        .collect();
    each.join(", ")
}

/// The checkout a session of task `task` works in (ruling RR-1): lane `lane`'s stored
/// checkout (`Lane.checkout`, review m4) while it races, else the task's
/// (`Task::checkout_name`, the crowned lane's once there is one). Task M9.5.18 (task 15
/// re-review N3): a lane the run stores no `Lane` for has no checkout here, never the
/// task's own, so its session is given no other checkout's sandbox roots.
pub(super) fn checkout_of(run: &Run, task: &str, lane: Option<RaceLane>) -> Result<String, String> {
    let t = run.task(task);
    let Some(lane) = lane else {
        return Ok(t.map_or_else(|| task.to_string(), |t| t.checkout_name()));
    };
    let mut lanes = t
        .into_iter()
        .flat_map(|t| &t.race)
        .flat_map(|race| &race.lanes);
    match lanes.find(|l| l.lane == lane) {
        Some(stored) => Ok(stored.checkout.clone()),
        None => Err(format!(
            "task {task} has no lane {}; its racer gets no sandbox roots",
            lane.label()
        )),
    }
}

/// [`checkout_of`] for a session in `role`: a racer must name a lane the run stores.
pub(super) fn session_checkout(
    run: &Run,
    task: &str,
    role: AgentRole,
    lane: Option<RaceLane>,
) -> Result<String, String> {
    if role == AgentRole::Racer && lane.is_none() {
        return Err(format!(
            "a racer of task {task} names no lane; it gets no sandbox roots"
        ));
    }
    checkout_of(run, task, lane)
}

#[cfg(test)]
#[path = "lane_ops_tests.rs"]
mod tests;
