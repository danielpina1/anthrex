//! Decision 36's merge candidate, one attempt of the merge queue (M8a.22's executor for
//! `OpKind::MergeCandidate`): the ref guard, `merge-tree`, the candidate commit, its
//! check in the integration worktree, the guard again, the compare-and-swap and the
//! reattach. Writes go through the run's `GitQueue`; reads run on `spawn_blocking`.

use std::sync::Arc;
use std::time::Duration;

use super::ops::blocking;
use super::{OpCtx, RunService};
use crate::run::engine::{EventKind, OpKind, OpResult};
use crate::run::git::{self, CandidateStep, RefCheck};

/// Decision 36, one attempt of the merge queue (see `OpKind::MergeCandidate`).
pub(super) async fn candidate(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    kind: OpKind,
) -> Result<OpResult, String> {
    let OpKind::MergeCandidate {
        root,
        integration,
        run_branch,
        expected_run_head,
        base_branch,
        expected_base,
        task_head,
        message,
        check,
        timeout_secs,
        env,
    } = kind
    else {
        unreachable!("candidate takes a MergeCandidate");
    };
    let guard = |advanced: Option<String>| {
        let (git, t) = (service.git(), ctx.git_timeout);
        let (root, base, eb, rb, erh) = (
            root.clone(),
            base_branch.clone(),
            expected_base.clone(),
            run_branch.clone(),
            expected_run_head.clone(),
        );
        async move {
            let checked =
                blocking(move || git::guard_refs(&git, &root, &base, &eb, &rb, &erh, t)).await?;
            Ok::<_, String>(match checked {
                RefCheck::Ok => None,
                RefCheck::BaseAdvanced { to, commits } => {
                    // Decision 21: the engine hears of an advance once per head.
                    if advanced.as_deref() != Some(to.as_str()) {
                        service.send(EventKind::BaseAdvanced {
                            run_id: ctx.run_id.clone(),
                            to: to.clone(),
                            commits,
                        });
                    }
                    Some(Ok(to))
                }
                RefCheck::Halt { reason } => Some(Err(reason)),
            })
        }
    };
    let reattach = || {
        let (at, branch) = (integration.clone(), run_branch.clone());
        service.write(ctx, move |g, t| git::reattach(g, &at, &branch, t))
    };
    let advanced = match guard(None).await? {
        Some(Err(reason)) => return Ok(OpResult::RefMoved { reason }),
        Some(Ok(to)) => Some(to),
        None => None,
    };
    let (git, t) = (service.git(), ctx.git_timeout);
    let (r, rh, th) = (root.clone(), expected_run_head.clone(), task_head.clone());
    let tree = match blocking(move || git::merge_tree(&git, &r, &rh, &th, t)).await? {
        CandidateStep::Conflict(files) => return Ok(OpResult::Conflict { files }),
        CandidateStep::Tree(tree) => tree,
    };
    let (r, rh, th) = (root.clone(), expected_run_head.clone(), task_head.clone());
    let commit = service
        .write(ctx, move |g, t| {
            git::commit_tree(g, &r, &tree, &[rh.as_str(), th.as_str()], &message, t)
        })
        .await?;
    // Ruling T22-minors, m3: once the candidate is materialized, every way out puts the
    // integration worktree back on the run branch, an error included.
    let mut materialized = false;
    let landed = async {
        if let Some(check) = check {
            let (at, c) = (integration.clone(), commit.clone());
            materialized = true;
            service
                .write(ctx, move |g, t| git::materialize(g, &at, &c, t))
                .await?;
            let at = integration.clone();
            let timeout = Duration::from_secs(timeout_secs);
            let confine = ctx.confine.clone();
            let outcome = blocking(move || {
                Ok(crate::run::confine::confined(
                    &at,
                    &check,
                    &env,
                    timeout,
                    confine.as_deref(),
                ))
            })
            .await?;
            if !outcome.ok {
                reattach().await?;
                return Ok(OpResult::CandidateRed {
                    code: outcome.code,
                    timed_out: outcome.timed_out,
                    tail: outcome.tail,
                    secs: outcome.secs,
                });
            }
        }
        if let Some(Err(reason)) = guard(advanced).await? {
            reattach().await?;
            return Ok(OpResult::RefMoved { reason });
        }
        let (r, b, c, old) = (
            root.clone(),
            run_branch.clone(),
            commit.clone(),
            expected_run_head.clone(),
        );
        let swapped = service
            .write(ctx, move |g, t| git::cas_update(g, &r, &b, &c, &old, t))
            .await?;
        if !swapped {
            reattach().await?;
            return Ok(OpResult::RefMoved {
                reason: format!("refs/heads/{run_branch} moved during the merge"),
            });
        }
        // Final fix batch F1, finding D-7: the run branch holds the candidate now, so
        // the task is merged whatever the reattach does. A failed reattach is a note, as
        // in reconcile; the next candidate's `materialize` checks out over it anyway.
        if let Err(error) = reattach().await {
            tracing::warn!(
                run = %ctx.run_id,
                %error,
                "merged, but the integration worktree could not go back on its branch"
            );
        }
        Ok(OpResult::Merged { commit })
    }
    .await;
    if landed.is_err() && materialized {
        let _ = reattach().await;
    }
    landed
}
