//! Milestone 9.1's stage ops (decisions 48, 50 and 53): the create-only stage branch,
//! the propagate of one stage into the next (merged as a merge candidate is), and
//! decision 37's ref guard over every run ref (moved here from `ops.rs`, which the
//! stage list would otherwise push past its budget). Writes go through the run's
//! `GitQueue`; reads run on `spawn_blocking`. No lock is held here.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::merge::{Merge, merge_into};
use super::ops::{blocking, failed};
use super::{OpCtx, RunService};
use crate::run::engine::{EventKind, OpKind, OpResult, Rebaseline};
use crate::run::git::{self, RefCheck};
use crate::run::model::{OpId, PropagateSpec};

/// `OpKind::CreateStageBranch`: `refs_tx::create_branch`, a write through the queue. A
/// branch that already exists at `from` (a replay) is created; anywhere else it is a
/// moved ref (decision 21).
pub(super) async fn create(service: &Arc<RunService>, ctx: &OpCtx, kind: OpKind) -> OpResult {
    let OpKind::CreateStageBranch { root, branch, from } = kind else {
        unreachable!("create takes a CreateStageBranch");
    };
    // Controller ruling C-15 (M-4): a symbolic ref where the branch should be is moved.
    let refname = format!("refs/heads/{branch}");
    let (git, t, r, rn) = (
        service.git(),
        ctx.git_timeout,
        root.clone(),
        refname.clone(),
    );
    match blocking(move || git::refs_tx::symbolic(&git, &r, &rn, t)).await {
        Ok(None) => {}
        Ok(Some(target)) => {
            return OpResult::RefMoved {
                reason: format!("{refname} is a symbolic ref to {target}"),
            };
        }
        Err(error) => return failed(error),
    }
    let (r, b, f) = (root.clone(), branch.clone(), from.clone());
    let made = service
        .write(ctx, move |g, t| {
            git::refs_tx::create_branch(g, &r, &b, &f, t)
        })
        .await;
    match made {
        Ok(true) => OpResult::StageCreated,
        Ok(false) => {
            let (git, t) = (service.git(), ctx.git_timeout);
            let refname = format!("refs/heads/{branch}");
            let at = blocking(move || git::read_ref(&git, &root, &refname, t)).await;
            match at {
                Ok(Some(head)) if head == from => OpResult::StageCreated,
                Ok(Some(head)) => OpResult::RefMoved {
                    reason: format!(
                        "refs/heads/{branch} exists at {}, not {}",
                        git::short(&head),
                        git::short(&from)
                    ),
                },
                Ok(None) => failed(format!("refs/heads/{branch} could not be created")),
                Err(error) => failed(error),
            }
        }
        Err(error) => failed(error),
    }
}

/// `OpKind::Propagate` (milestone 9.1 decision 50): the lower stage's head merged into
/// the upper stage exactly as a merge candidate is merged (`merge::merge_into`), with
/// parents `[to head, from head]`; a conflict gives the conflicted tree too.
pub(super) async fn propagate(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    op: OpId,
    kind: OpKind,
) -> Result<OpResult, String> {
    let OpKind::Propagate(spec) = kind else {
        unreachable!("propagate takes a Propagate");
    };
    let PropagateSpec {
        root,
        integration,
        from_head,
        to_branch,
        expected_to_head,
        also_integration,
        base_branch,
        expected_base,
        guarded,
        message,
        tier,
        check,
        timeout_secs,
        env,
        ..
    } = *spec;
    let merge = Merge {
        root,
        integration,
        branch: to_branch,
        expected_head: expected_to_head,
        base_branch,
        expected_base,
        other: from_head,
        message,
        check,
        timeout_secs,
        env,
        guarded,
        also_integration,
        tier,
    };
    merge_into(service, ctx, op, merge).await
}

/// `OpKind::VerifyRefs`: `guard_refs` over `guarded` (the run branch alone in an intent
/// from before milestone 9.1), a read.
pub(super) async fn verify_refs(service: &Arc<RunService>, ctx: &OpCtx, kind: OpKind) -> OpResult {
    let OpKind::VerifyRefs {
        root,
        base_branch,
        expected_base,
        run_branch,
        expected_run_head,
        guarded,
    } = kind
    else {
        unreachable!("verify_refs takes a VerifyRefs");
    };
    let guarded = guard_list(guarded, run_branch, expected_run_head);
    let (git, t) = (service.git(), ctx.git_timeout);
    let checked =
        blocking(move || git::guard_refs(&git, &root, &base_branch, &expected_base, &guarded, t))
            .await;
    match checked {
        Ok(RefCheck::Ok) => OpResult::RefsOk,
        Ok(RefCheck::BaseAdvanced { to, commits }) => {
            service.send(EventKind::BaseAdvanced {
                run_id: ctx.run_id.clone(),
                to,
                commits,
            });
            OpResult::RefsOk
        }
        Ok(RefCheck::Halt { reason }) => OpResult::RefMoved { reason },
        Err(error) => failed(error),
    }
}

/// The refs `run resume --rebaseline` reads (decision 21; milestone 9.1 decision 47).
pub(super) struct RunRefs {
    pub run_id: String,
    pub root: PathBuf,
    pub project: PathBuf,
    pub base_branch: String,
    pub run_branch: String,
    /// A `Multi` run's created stages, `(n, branch)`; empty for a `Single` one.
    pub stages: Vec<(u16, String)>,
    pub timeout: Duration,
    /// Controller ruling C-16: the run is `halted` or `paused`, so the engine will take
    /// this resume; in any other state the refs are only read, never written.
    pub may_move: bool,
    /// Unix seconds, for the salvage ref's name.
    pub now: u64,
}

/// `run resume --rebaseline`'s reads: the base, `integration` and every created stage.
/// Controller ruling C-15 (I-1): a `Multi` run adopts every stage ref, and when
/// `integration` is not at the highest stage's head (a user moved either), the driver
/// puts it back there, in one transaction that keeps the commit it was at under
/// `refs/anthrex/salvage/<run>/_integration-<unix secs>` and moves it only from the
/// value read (`--no-deref`). anthrex moves only its own ref: no force, never the base.
pub(super) async fn rebaseline(service: &RunService, refs: RunRefs) -> Result<Rebaseline, String> {
    let RunRefs {
        run_id,
        root,
        project,
        base_branch,
        run_branch,
        stages,
        timeout,
        may_move,
        now,
    } = refs;
    let git = service.git();
    let stage_branches: Vec<String> = stages.iter().map(|(_, b)| b.clone()).collect();
    let (r, b, rb) = (root.clone(), base_branch.clone(), run_branch.clone());
    let mut read_all = blocking(move || {
        let read = |branch: &str| {
            let refname = format!("refs/heads/{branch}");
            git::read_ref(&git, &r, &refname, timeout)?
                .ok_or_else(|| format!("{refname} does not exist"))
        };
        let mut read_all = Rebaseline {
            base: read(&b)?,
            head: read(&rb)?,
            ..Rebaseline::default()
        };
        for (n, branch) in stages {
            read_all.stages.push((n, read(&branch)?));
        }
        Ok(read_all)
    })
    .await?;
    let Some((_, top)) = read_all.stages.iter().max_by_key(|(n, _)| *n).cloned() else {
        return Ok(read_all);
    };
    // Controller ruling C-16: a run the engine will refuse gets no write.
    if top == read_all.head || !may_move {
        return Ok(read_all);
    }
    let base = format!("refs/anthrex/salvage/{run_id}/_integration-{now}");
    let old = read_all.head.clone();
    // Controller ruling C-16 (3): every stage ref read is verified in the transaction.
    let verifies: Vec<(String, String)> = stage_branches
        .iter()
        .zip(&read_all.stages)
        .map(|(branch, (_, head))| (format!("refs/heads/{branch}"), head.clone()))
        .collect();
    let (g, r, b, new, o) = (
        service.git(),
        root,
        run_branch.clone(),
        top.clone(),
        old.clone(),
    );
    let (salvage, moved) = service
        .queue
        .write(&project, move || {
            // Controller ruling C-16 (4): a free name, inside the queue's turn.
            let salvage = git::refs_tx::free_ref(&g, &r, &base, timeout)?;
            let swap = git::refs_tx::salvage_and_move(
                &g,
                &r,
                &salvage,
                &b,
                (&new, &o),
                &verifies,
                timeout,
            )?;
            Ok((salvage, swap))
        })
        .await?;
    match moved {
        git::refs_tx::Swap::Done => {
            // Controller ruling C-16: recorded at once, before the engine hears of it.
            tracing::info!(
                run = %run_id,
                salvage = %salvage,
                old = %old,
                new = %top,
                "rebaseline: integration moved back to the highest stage; its commit salvaged"
            );
            read_all.head = top;
            read_all.salvaged = Some((old, salvage));
            Ok(read_all)
        }
        git::refs_tx::Swap::Moved(refname) => Err(format!(
            "{refname} moved while the refs were read; resume with --rebaseline again"
        )),
    }
}

/// An op's guard list: `guarded`, or the run branch at its expected head when the op
/// was journaled before milestone 9.1 (M8a's guard).
pub(super) fn guard_list(
    guarded: Vec<(String, String)>,
    run_branch: String,
    expected: String,
) -> Vec<(String, String)> {
    if guarded.is_empty() {
        vec![(run_branch, expected)]
    } else {
        guarded
    }
}

#[cfg(test)]
#[path = "stage_ops_tests.rs"]
mod tests;
