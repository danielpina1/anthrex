//! The gate ops, executed: decision 33's proof and decision 34's check. Split out of
//! `ops.rs` before milestone 9.1 to keep it under the 600-line rule; a pure move.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::super::{OpCtx, RunService};
use super::{blocking, failed, setup_output};
use crate::run::confine::confined;
use crate::run::engine::{OpResult, ScratchAt};
use crate::run::git;
use crate::run::proof::{ProofError, ProofOp, SETUP_MARKER, run_proof};

/// Decision 33's proof on a blocking thread, each git step through the queue by
/// `Handle::block_on` (the daemon's multi-thread runtime; see `OpKind::Proof`).
pub(super) async fn proof(service: &Arc<RunService>, ctx: &OpCtx, op: ProofOp) -> OpResult {
    let (git, t) = (service.git(), ctx.git_timeout);
    let queue = service.queue.clone();
    let project = ctx.project.clone();
    let handle = tokio::runtime::Handle::current();
    let ran = tokio::task::spawn_blocking(move || {
        let hook = |step: crate::run::proof::GitStep| handle.block_on(queue.write(&project, step));
        run_proof(&git, &op, t, &hook)
    })
    .await;
    match ran {
        Ok(Ok(runs)) => OpResult::Proof {
            red_failed: runs.red_failed,
            head_passed: runs.head_passed,
            matched: runs.matched,
            red_tail: runs.red_tail,
            head_tail: runs.head_tail,
        },
        Ok(Err(ProofError::SetupFailed { output })) => OpResult::SetupFailed { output },
        Ok(Err(ProofError::Failed(message))) => failed(message),
        Err(error) => failed(format!("the proof did not finish: {error}")),
    }
}

/// Decision 34's check (ruling T13-I3's scratch contract when `scratch` is set).
pub(super) async fn check(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    dir: PathBuf,
    command: String,
    timeout_secs: u64,
    env: Vec<(String, String)>,
    scratch: Option<ScratchAt>,
) -> OpResult {
    let timeout = Duration::from_secs(timeout_secs);
    if let Some(ScratchAt {
        root,
        commit,
        setup,
    }) = scratch
    {
        let (at, c) = (dir.clone(), commit.clone());
        let repo = git::checkout_repo_dir(&ctx.data_dir, &dir);
        if let Err(error) = service
            .write(ctx, move |g, t| {
                git::prepare_scratch_in(g, &root, &at, &c, &repo, t)
            })
            .await
        {
            return failed(error);
        }
        let (git, t, at) = (service.git(), ctx.git_timeout, dir.clone());
        let found = blocking(move || {
            let marker = git::absolute_git_dir(&git, &at, t)?.join(SETUP_MARKER);
            let exists = marker.exists();
            Ok((marker, exists))
        })
        .await;
        let (marker, exists) = match found {
            Ok(found) => found,
            Err(error) => return failed(error),
        };
        if !exists {
            if let Some(setup) = setup {
                let (at, c) = (dir.clone(), commit.clone());
                if let Err(error) = service
                    .write(ctx, move |g, t| git::materialize(g, &at, &c, t))
                    .await
                {
                    return failed(error);
                }
                let (at, env, confine) = (dir.clone(), env.clone(), ctx.confine.clone());
                let outcome =
                    blocking(move || Ok(confined(&at, &setup, &env, timeout, confine.as_deref())))
                        .await;
                match outcome {
                    Ok(outcome) if !outcome.ok => {
                        return OpResult::SetupFailed {
                            output: setup_output(&outcome),
                        };
                    }
                    Ok(_) => {}
                    Err(error) => return failed(error),
                }
            }
            let written = blocking(move || {
                std::fs::write(&marker, "")
                    .map_err(|error| format!("could not write {}: {error}", marker.display()))
            })
            .await;
            if let Err(error) = written {
                return failed(error);
            }
        }
        let at = dir.clone();
        if let Err(error) = service
            .write(ctx, move |g, t| git::materialize(g, &at, &commit, t))
            .await
        {
            return failed(error);
        }
    }
    let confine = ctx.confine.clone();
    match blocking(move || Ok(confined(&dir, &command, &env, timeout, confine.as_deref()))).await {
        Ok(outcome) => OpResult::Check {
            ok: outcome.ok,
            code: outcome.code,
            timed_out: outcome.timed_out,
            tail: outcome.tail,
            secs: outcome.secs,
        },
        Err(error) => failed(error),
    }
}
