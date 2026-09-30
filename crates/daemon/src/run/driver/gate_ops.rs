//! The gate ops, executed: decision 33's proof and decision 34's check. Split out of
//! `ops.rs` before milestone 9.1 to keep it under the 600-line rule; a pure move.
//! Milestone 9.1 (controller ruling 1, ruling C-12b): each command, `setup` included,
//! waits for its slots in the test scheduler and runs isolated (`scheduled.rs`); a
//! check in a scratch checkout is a gate's (`Gate`, half the slots), the final check
//! in the integration worktree is a completion's (`FullStage`, all of them).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use super::super::{OpCtx, RunService};
use super::scheduled::{Class, common_dir, hold_blocking, scheduled};
use super::{blocking, failed, setup_output};
use crate::run::engine::{OpResult, ScratchAt};
use crate::run::git;
use crate::run::model::OpId;
use crate::run::proof::{ProofError, ProofOp, ProofStep, SETUP_MARKER, run_proof_scheduled};
use crate::run::slots::{Priority, Want};

/// Decision 33's proof on a blocking thread, each git step through the queue by
/// `Handle::block_on` (the daemon's multi-thread runtime; see `OpKind::Proof`).
pub(super) async fn proof(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    op_id: OpId,
    op: ProofOp,
) -> OpResult {
    let (git, t) = (service.git(), ctx.git_timeout);
    let queue = service.queue.clone();
    let project = ctx.project.clone();
    let handle = tokio::runtime::Handle::current();
    let common = match common_dir(ctx, git.clone()).await {
        Ok(common) => common,
        Err(error) => return failed(error),
    };
    let base = super::super::tier_step::step_base(&ctx.data_dir, &op.path);
    let sched = service.scheduler().clone();
    let ran = tokio::task::spawn_blocking(move || {
        let hook = |step: crate::run::proof::GitStep| handle.block_on(queue.write(&project, step));
        // Ruling C-12b: each run waits for its slots on this blocking thread, never
        // while a git step is queued, and gives them back before the next one.
        let slot = |step: ProofStep| {
            let (want, name) = match step {
                ProofStep::Setup => (Want::One, "setup"),
                ProofStep::Red => (Want::Half, "red"),
                ProofStep::Head => (Want::Half, "head"),
            };
            let class: Class = (Priority::Gate, want, format!("{name} proof"));
            let dir = format!("s{op_id}-{name}");
            hold_blocking(&handle, &sched, &class, (&common, &base, &dir))
                .map(|(extra, held)| (extra, Box::new(held) as Box<dyn Send>))
        };
        run_proof_scheduled(&git, &op, t, &hook, &slot)
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
    op: OpId,
    (dir, scratch): (PathBuf, Option<ScratchAt>),
    command: String,
    timeout_secs: u64,
    env: Vec<(String, String)>,
) -> OpResult {
    let timeout = Duration::from_secs(timeout_secs);
    // Decision 24: a task's check is a gate; the final check (no scratch) is the run's
    // completion, as tier 3 at completion is.
    let class = if scratch.is_some() {
        (Priority::Gate, Want::Half, "check".to_string())
    } else {
        (Priority::FullStage, Want::All, "check final".to_string())
    };
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
                let one = (Priority::Gate, Want::One, "setup".to_string());
                let outcome = scheduled(service, ctx, op, one)
                    .run(&dir, &setup, &env, timeout)
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
    match scheduled(service, ctx, op, class)
        .run(&dir, &command, &env, timeout)
        .await
    {
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
