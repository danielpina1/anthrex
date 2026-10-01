//! Decision 36's merge candidate, one attempt of the merge queue (M8a.22's executor for
//! `OpKind::MergeCandidate`): the ref guard, `merge-tree`, the candidate commit, its
//! check in the integration worktree, the guard again, the compare-and-swap and the
//! reattach. Writes go through the run's `GitQueue`; reads run on `spawn_blocking`.
//!
//! Milestone 9.1 decision 16: a tiered profile's candidate runs its tier-2 job where
//! M8a runs `check`, through the tier executor; the result cache is consulted first,
//! and when every step hits the candidate is not materialized at all. M8a's check waits
//! for its slots in the test scheduler (controller ruling 1).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::ops::{blocking, scheduled::scheduled};
use super::stage_ops::guard_list;
use super::{OpCtx, RunService, tier};
use crate::run::engine::{EventKind, OpKind, OpResult};
use crate::run::git::{self, CandidateStep, RefCheck};
use crate::run::model::OpId;
use crate::run::slots::{Priority, Want};
use crate::run::tiers::TierSpec;

/// Decision 36, one attempt of the merge queue (see `OpKind::MergeCandidate`).
pub(super) async fn candidate(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    op: OpId,
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
        guarded,
        also_integration,
        tier,
    } = kind
    else {
        unreachable!("candidate takes a MergeCandidate");
    };
    let merge = Merge {
        root,
        integration,
        branch: run_branch,
        expected_head: expected_run_head,
        base_branch,
        expected_base,
        other: task_head,
        message,
        check,
        timeout_secs,
        env,
        guarded,
        also_integration,
        tier,
        held_if_contained: false,
    };
    merge_into(service, ctx, op, merge).await
}

/// One merge into a run branch: a task's candidate (decision 36), or since milestone
/// 9.1 a propagate of one stage into the next (decision 50), which runs exactly the
/// same steps with the lower stage's head as `other`.
pub(super) struct Merge {
    pub root: PathBuf,
    pub integration: PathBuf,
    /// The branch the merge lands on, and the head it must be at.
    pub branch: String,
    pub expected_head: String,
    pub base_branch: String,
    pub expected_base: String,
    /// The second parent: the task's claimed commit, or the lower stage's head.
    pub other: String,
    pub message: String,
    pub check: Option<String>,
    pub timeout_secs: u64,
    pub env: Vec<(String, String)>,
    pub guarded: Vec<(String, String)>,
    pub also_integration: bool,
    pub tier: Option<Box<TierSpec>>,
    /// Controller ruling C-22 (2), a propagate's: `other` already in the branch is
    /// `AlreadyHeld`, with nothing written.
    pub held_if_contained: bool,
}

/// [`Merge`]'s steps: the guard, `merge-tree`, the commit with parents `[expected_head,
/// other]`, tier 2 or `check` on it, the guard again, the compare-and-swap, the
/// reattach.
pub(super) async fn merge_into(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    op: OpId,
    merge: Merge,
) -> Result<OpResult, String> {
    let Merge {
        root,
        integration,
        branch: run_branch,
        expected_head: expected_run_head,
        base_branch,
        expected_base,
        other: task_head,
        message,
        check,
        timeout_secs,
        env,
        guarded,
        also_integration,
        tier: tier_spec,
        held_if_contained,
    } = merge;
    // Milestone 9.1 decision 53: every run ref is guarded, and the integration worktree
    // stays on `integration`, the alias a stage branch's merge leaves it on.
    let guarded = guard_list(guarded, run_branch.clone(), expected_run_head.clone());
    let alias = git::refs_tx::alias_of(&run_branch);
    let guard = |advanced: Option<String>| {
        let (git, t) = (service.git(), ctx.git_timeout);
        let (root, base, eb, list) = (
            root.clone(),
            base_branch.clone(),
            expected_base.clone(),
            guarded.clone(),
        );
        async move {
            let checked =
                blocking(move || git::guard_refs(&git, &root, &base, &eb, &list, t)).await?;
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
        let (at, branch) = (integration.clone(), alias.clone());
        service.write(ctx, move |g, t| git::reattach(g, &at, &branch, t))
    };
    let advanced = match guard(None).await? {
        Some(Err(reason)) => return Ok(OpResult::RefMoved { reason }),
        Some(Ok(to)) => Some(to),
        None => None,
    };
    if held_if_contained {
        let (git, t) = (service.git(), ctx.git_timeout);
        let (r, rh, th) = (root.clone(), expected_run_head.clone(), task_head.clone());
        let contained =
            blocking(move || git::is_ancestor(git::Git::new(&git, t), &r, &th, &rh)).await?;
        if contained {
            return Ok(OpResult::AlreadyHeld);
        }
    }
    let (git, t) = (service.git(), ctx.git_timeout);
    let (r, rh, th) = (root.clone(), expected_run_head.clone(), task_head.clone());
    let tree = match blocking(move || git::merge_tree(&git, &r, &rh, &th, t)).await? {
        CandidateStep::Conflict { files, tree } => {
            let tree = Some(tree);
            return Ok(OpResult::Conflict { files, tree });
        }
        CandidateStep::Tree(tree) => tree,
    };
    let (r, rh, th) = (root.clone(), expected_run_head.clone(), task_head.clone());
    let commit = service
        .write(ctx, move |g, t| {
            git::commit_tree(g, &r, &tree, &[rh.as_str(), th.as_str()], &message, t)
        })
        .await?;
    // Milestone 9.1 decision 16: tier 2 judges this candidate; the cache first.
    let tier_spec = tier_spec.map(|mut spec| {
        spec.head = commit.clone();
        spec
    });
    let (sched, cache, git) = (service.scheduler(), service.test_cache(), service.git());
    let mut green = None;
    if let Some(spec) = &tier_spec {
        let shared = (sched.as_ref(), service.queue.as_ref(), git.as_os_str());
        green = tier::cached_outcome(ctx, shared, cache, op, spec)
            .await
            .map(Box::new);
    }
    // Ruling T22-minors, m3: once the candidate is materialized, every way out puts the
    // integration worktree back on the run branch, an error included.
    let mut materialized = false;
    let landed = async {
        let materialize = || {
            let (at, c) = (integration.clone(), commit.clone());
            service.write(ctx, move |g, t| git::materialize(g, &at, &c, t))
        };
        if let Some(spec) = tier_spec.as_ref().filter(|_| green.is_none()) {
            materialized = true;
            materialize().await?;
            let queue = &service.queue;
            match tier::run_tier(ctx, sched, cache, queue, &git, op, spec).await {
                OpResult::Tier(outcome) if outcome.ok => green = Some(outcome),
                OpResult::Tier(outcome) => {
                    reattach().await?;
                    let red = outcome.steps.iter().rev().find(|s| !s.ok);
                    return Ok(OpResult::CandidateRed {
                        code: red.and_then(|s| s.code),
                        timed_out: red.is_some_and(|s| s.timed_out),
                        tail: outcome.tail.clone(),
                        secs: outcome.secs,
                        tier: Some(outcome),
                    });
                }
                OpResult::Failed { message } => return Err(message),
                other => return Err(format!("tier 2 did not run: {other:?}")),
            }
        } else if let Some(check) = check.filter(|_| tier_spec.is_none()) {
            materialized = true;
            materialize().await?;
            let at = integration.clone();
            let timeout = Duration::from_secs(timeout_secs);
            let label = "candidate check".to_string();
            let outcome = scheduled(service, ctx, op, (Priority::Candidate, Want::All, label))
                .run(&at, &check, &env, timeout)
                .await?;
            if !outcome.ok {
                reattach().await?;
                return Ok(OpResult::CandidateRed {
                    code: outcome.code,
                    timed_out: outcome.timed_out,
                    tail: outcome.tail,
                    secs: outcome.secs,
                    tier: None,
                });
            }
        }
        if let Some(Err(reason)) = guard(advanced).await? {
            reattach().await?;
            return Ok(OpResult::RefMoved { reason });
        }
        // Decision 53: into a `Multi` run's highest stage, its ref and `integration`
        // move together or not at all. Controller ruling C-15 (M-3): every other
        // guarded ref is verified in the same transaction, so one that moved after the
        // guard refuses the swap; (M-2) the refusal names the ref that differed.
        let mut moving = vec![run_branch.clone()];
        if also_integration {
            moving.push(alias.clone());
        }
        let refs = |b: &str| format!("refs/heads/{b}");
        let updates: Vec<(String, String, String)> = moving
            .iter()
            .map(|b| (refs(b), commit.clone(), expected_run_head.clone()))
            .collect();
        let verifies: Vec<(String, String)> = guarded
            .iter()
            .filter(|(b, _)| !moving.contains(b))
            .map(|(b, head)| (refs(b), head.clone()))
            .collect();
        let r = root.clone();
        let swapped = service
            .write(ctx, move |g, t| {
                git::refs_tx::cas(g, &r, &updates, &verifies, t)
            })
            .await?;
        if let git::refs_tx::Swap::Moved(refname) = swapped {
            reattach().await?;
            return Ok(OpResult::RefMoved {
                reason: format!("{refname} moved during the merge"),
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
        Ok(OpResult::Merged {
            commit,
            tier: green,
        })
    }
    .await;
    if landed.is_err() && materialized {
        let _ = reattach().await;
    }
    landed
}

#[cfg(test)]
#[path = "merge_tier_tests.rs"]
mod tier_tests;
