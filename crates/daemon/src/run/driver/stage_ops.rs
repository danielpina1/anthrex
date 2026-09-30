//! Milestone 9.1's stage ops (decisions 48 and 53): the create-only stage branch, and
//! decision 37's ref guard over every run ref (moved here from `ops.rs`, which the
//! stage list would otherwise push past its budget). Writes go through the run's
//! `GitQueue`; reads run on `spawn_blocking`. No lock is held here.

use std::sync::Arc;

use super::ops::{blocking, failed};
use super::{OpCtx, RunService};
use crate::run::engine::{EventKind, OpKind, OpResult};
use crate::run::git::{self, RefCheck};

/// `OpKind::CreateStageBranch`: `refs_tx::create_branch`, a write through the queue. A
/// branch that already exists at `from` (a replay) is created; anywhere else it is a
/// moved ref (decision 21).
pub(super) async fn create(service: &Arc<RunService>, ctx: &OpCtx, kind: OpKind) -> OpResult {
    let OpKind::CreateStageBranch { root, branch, from } = kind else {
        unreachable!("create takes a CreateStageBranch");
    };
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
