//! Decision 20's end of a worktree and of a run (M8a.22's executors): salvage then
//! removal, `run accept` with its clean-up (also re-run for an accept a restart
//! replayed), and `run discard`. Every step is a write through the run's `GitQueue`.

use std::path::PathBuf;
use std::sync::Arc;

use super::ops::failed;
use super::{OpCtx, RunService};
use crate::run::engine::{OpKind, OpResult};
use crate::run::git;

/// Decision 20's discard: every worktree salvaged and removed (a worktree is never
/// removed dirty without its salvage ref), then the run's branches deleted.
pub(super) async fn discard(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    root: PathBuf,
    worktrees: Vec<(PathBuf, String)>,
    branch_prefix: String,
) -> OpResult {
    for (path, reference) in &worktrees {
        service.unwatch_root(path);
        if let Err(error) =
            remove_worktree(service, ctx, root.clone(), path.clone(), reference.clone()).await
        {
            return failed(error);
        }
    }
    match service
        .write(ctx, move |g, t| {
            git::delete_branches(g, &root, &branch_prefix, t)
        })
        .await
    {
        Ok(kept_branches) => OpResult::Finished {
            outcome: "worktrees removed and branches deleted".to_string(),
            kept_branches,
        },
        Err(error) => failed(error),
    }
}

/// Decision 20's salvage, then the removal; `Some(ref)` when the worktree was dirty. A
/// worktree already gone has nothing to salvage.
pub(super) async fn remove_worktree(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    root: PathBuf,
    path: PathBuf,
    reference: String,
) -> Result<Option<String>, String> {
    let salvaged = if path.exists() {
        let message = salvage_message(&ctx.run_id, &reference);
        let at = path.clone();
        service
            .write(ctx, move |g, t| {
                git::salvage(g, &at, &reference, &message, t)
            })
            .await?
    } else {
        None
    };
    // Final fix batch F1c: a standalone checkout goes with its repository (the
    // worker's private objects, the engine's own files, its temporary directory).
    let repo = git::checkout_repo_dir(&ctx.data_dir, &path);
    service
        .write(ctx, move |g, t| {
            git::remove_checkout(g, &root, &path, Some(&repo), t)
        })
        .await?;
    Ok(salvaged)
}

/// `anthrex salvage <run>/<task>`, the task read from the ref
/// `refs/anthrex/salvage/<run>/<task>/<seq>`.
fn salvage_message(run_id: &str, reference: &str) -> String {
    let prefix = format!("refs/anthrex/salvage/{run_id}/");
    let task = reference
        .strip_prefix(&prefix)
        .and_then(|rest| rest.rsplit_once('/'))
        .map_or("run", |(task, _seq)| task);
    format!("anthrex salvage {run_id}/{task}")
}

/// Decision 20's accept. `git::accept` runs its merge under its own 10-minute
/// `ACCEPT_MERGE_TIMEOUT` and is never wrapped in a shorter one (carry T9). Once it has
/// merged the run is accepted whatever follows: a clean-up failure is still `Finished`,
/// naming it.
pub(super) async fn accept(service: &Arc<RunService>, ctx: &OpCtx, kind: OpKind) -> OpResult {
    let OpKind::Accept {
        root,
        base_branch,
        expected_base,
        run_branch,
        expected_run_head,
        message,
        worktrees,
        branch_prefix,
    } = kind
    else {
        unreachable!("accept takes an Accept");
    };
    let r = root.clone();
    let merged = service
        .write(ctx, move |g, t| {
            git::accept(
                g,
                &r,
                &base_branch,
                &expected_base,
                &run_branch,
                &expected_run_head,
                &message,
                t,
            )
        })
        .await;
    match merged {
        Ok(git::AcceptOutcome::Merged { .. }) => {}
        Ok(git::AcceptOutcome::Conflict { files }) => return OpResult::AcceptConflict { files },
        Err(error) => return failed(error),
    }
    let (outcome, kept_branches) = clean_up(service, ctx, root, &worktrees, branch_prefix).await;
    OpResult::Finished {
        outcome,
        kept_branches,
    }
}

/// Accept's clean-up (and a replayed accept's, after a restart): salvage and remove
/// every run worktree, then delete the run's branches.
pub(super) async fn clean_up(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    root: PathBuf,
    worktrees: &[(PathBuf, String)],
    branch_prefix: String,
) -> (String, Vec<String>) {
    for (path, reference) in worktrees {
        service.unwatch_root(path);
        if let Err(error) =
            remove_worktree(service, ctx, root.clone(), path.clone(), reference.clone()).await
        {
            return (format!("accepted; clean-up failed: {error}"), Vec::new());
        }
    }
    match service
        .write(ctx, move |g, t| {
            git::delete_branches(g, &root, &branch_prefix, t)
        })
        .await
    {
        Ok(kept) => ("merged into the base branch".to_string(), kept),
        Err(error) => (format!("accepted; clean-up failed: {error}"), Vec::new()),
    }
}

#[cfg(test)]
mod tests {
    use super::salvage_message;

    #[test]
    fn the_salvage_message_names_the_task() {
        assert_eq!(
            salvage_message("r-1a2b", "refs/anthrex/salvage/r-1a2b/t1/2"),
            "anthrex salvage r-1a2b/t1"
        );
        assert_eq!(
            salvage_message("r-1a2b", "refs/anthrex/salvage/r-1a2b/integration/1"),
            "anthrex salvage r-1a2b/integration"
        );
    }
}
