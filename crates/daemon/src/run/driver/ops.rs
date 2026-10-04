//! Each engine op, executed (M8a.22): the executor contracts in `engine/ops.rs`'s
//! `OpKind` docs. Git writes go through `GitQueue::write`, keyed by the run's project
//! (decision 18); every other blocking call runs on `spawn_blocking`; sessions start
//! through the manager's `headless_*` methods. No lock is held here.

use std::ffi::OsString;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use super::{DONE_CHECK_GIT_TIMEOUT, OpCtx, RunService, cleanup, merge, stage_ops, tier};
use crate::headless::{HeadlessSpec, SessionArg};
use crate::run::engine::{OpKind, OpResult, ResolutionAt};
use crate::run::exec::ShellOutcome;
use crate::run::git::{self, RefreshedIn};
use crate::run::globs::{OwnsMatcher, ProtectedMatcher};
use crate::run::model::{OpId, SyncCheck};
use crate::run::proof::ProofOp;
use crate::run::role_launch::worker_git_roots;
use crate::run::slots::{Priority, Want};
use crate::run::tiers::SignalsSpec;

// Decision 33's proof and decision 34's check (split out to keep this file under the
// 600-line rule).
#[path = "gate_ops.rs"]
mod gate_ops;
#[path = "lane_ops.rs"]
mod lane_ops;
#[path = "sync_done.rs"]
mod sync_done;
use gate_ops::{check, proof};

// Controller ruling 1 and ruling C-12b: M8a's commands through the test scheduler.
#[path = "scheduled.rs"]
pub(super) mod scheduled;
use scheduled::scheduled;

// Task M9.1.16: `VerifyDone`'s signals against real git.
#[cfg(test)]
#[path = "ops_signals_tests.rs"]
mod signals_tests;

// Task M9.5.18: racers' and test writers' sandboxes and calls.
#[cfg(test)]
#[path = "racer_launch_tests.rs"]
mod racer_launch_tests;

pub(super) fn failed(message: impl Into<String>) -> OpResult {
    OpResult::Failed {
        message: message.into(),
    }
}

/// Runs `f` on a blocking thread.
pub(super) async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(f)
        .await
        .map_err(|error| format!("a blocking step did not finish: {error}"))?
}

/// The tail of a failed setup, with a note when it timed out.
fn setup_output(outcome: &ShellOutcome) -> String {
    if outcome.timed_out {
        format!(
            "{}\n[anthrex: timed out after {} s]",
            outcome.tail, outcome.secs
        )
    } else {
        outcome.tail.clone()
    }
}

/// `OpResult::Failed`'s message, or the op's own result.
fn settle(result: Result<OpResult, String>) -> OpResult {
    result.unwrap_or_else(failed)
}

impl RunService {
    pub(super) fn git(&self) -> OsString {
        self.ctx.git.clone()
    }

    /// A git write through the run's project queue (decision 18).
    pub(super) async fn write<T: Send + 'static>(
        &self,
        ctx: &OpCtx,
        f: impl Fn(&OsString, Duration) -> Result<T, String> + Send + Sync + 'static,
    ) -> Result<T, String> {
        let (git, timeout) = (self.git(), ctx.git_timeout);
        self.queue
            .write(&ctx.project, move || f(&git, timeout))
            .await
    }

    /// `setup` in `dir` after a worktree op (M8a.8's carry T8-I4): a failure is
    /// `SetupFailed` with its tail. It waits for one slot (decision 24, ruling C-12b).
    async fn setup(
        self: &Arc<Self>,
        ctx: &OpCtx,
        op: OpId,
        dir: &Path,
        (setup, env): (Option<String>, Vec<(String, String)>),
        head: String,
    ) -> OpResult {
        let Some(setup) = setup else {
            return OpResult::Worktree { head };
        };
        let class = (Priority::Gate, Want::One, "setup".to_string());
        match scheduled(self, ctx, op, class)
            .run(dir, &setup, &env, ctx.check_timeout)
            .await
        {
            Ok(outcome) if outcome.ok => OpResult::Worktree { head },
            Ok(outcome) => OpResult::SetupFailed {
                output: setup_output(&outcome),
            },
            Err(error) => failed(error),
        }
    }
}

/// A worker's sandbox, completed at launch (final fix batch F1 and its fix round 1;
/// F1b; F1c): when it is sandboxed, its writable roots are its checkout's private
/// object directory and its temporary directory (each made by the daemon, never
/// through a link, and named by appending to the engine's own directory, never by
/// resolving a path the worker could have swapped: I1), plus the commit's files in the
/// checkout's own git directory. Nothing of the git common directory. Any other session
/// (a reviewer) is left as it is. Milestone 9.5 (task M9.5.18): a racer's and a test
/// writer's too, a racer's on its own lane's stored checkout; a session whose lane the
/// run does not store is refused, with a logged error.
async fn worker_git_dirs(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    spec: &mut HeadlessSpec,
) -> Result<(), String> {
    let task = spec
        .run_ref
        .as_ref()
        .filter(|r| crate::run::model::writes_task(r.role))
        .and_then(|r| Some((r.task_id.clone()?, r.role, r.lane)));
    let Some((task, role, lane)) = task else {
        return Ok(());
    };
    // Milestone 9.5 ruling RR-1: the session's checkout, a lane's or the task's.
    let (common, checkout) = crate::lock(&service.state)
        .runs
        .get(&ctx.run_id)
        .map(|run| {
            (
                run.git_common_dir.clone(),
                lane_ops::session_checkout(run, &task, role, lane),
            )
        })
        .ok_or_else(|| format!("unknown run {}", ctx.run_id))?;
    let checkout = checkout.inspect_err(|error| {
        tracing::error!(run = %ctx.run_id, %task, %error, "a session's launch was refused");
    })?;
    let roots = worker_git_roots(&ctx.data_dir, &checkout);
    let sandboxed = spec
        .claude_sandbox
        .as_ref()
        .is_some_and(|s| !s.writable_roots.is_empty())
        || !spec.codex_writable_roots.is_empty();
    if !sandboxed {
        return Ok(());
    }
    let cwd = spec.cwd.clone();
    let dirs = service
        .write(ctx, move |_, _| {
            let roots = roots
                .iter()
                .map(|root| git::private_dir(&common, root))
                .collect::<Result<Vec<_>, _>>()?;
            git::worker_git_dirs(&common, &cwd, &roots)
        })
        .await?;
    if let Some(sandbox) = spec.claude_sandbox.as_mut()
        && !sandbox.writable_roots.is_empty()
    {
        sandbox.writable_roots = dirs.clone();
    }
    if !spec.codex_writable_roots.is_empty() {
        spec.codex_writable_roots = dirs;
    }
    Ok(())
}

/// Executes `kind`, op `op`, for the run of `ctx`.
pub(super) async fn run(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    op: OpId,
    kind: OpKind,
) -> OpResult {
    let git = service.git();
    match kind {
        OpKind::CreateRunBranch {
            root,
            branch,
            base_sha,
            path,
            setup,
            env,
        } => {
            let at = path.clone();
            let made = service
                .write(ctx, move |g, t| {
                    git::create_run_branch(g, &root, &branch, &base_sha, &at, t)
                })
                .await;
            match made {
                Ok(head) => service.setup(ctx, op, &path, (setup, env), head).await,
                Err(error) => failed(error),
            }
        }
        OpKind::PrepareWorktree {
            root,
            branch,
            from,
            path,
            setup,
            env,
        } => {
            let at = path.clone();
            // Final fix batch F1c (3a): the checkout's own repository, in the run's data
            // directory.
            let repo = git::checkout_repo_dir(&ctx.data_dir, &path);
            let made = service
                .write(ctx, move |g, t| {
                    git::prepare_task_worktree(g, &root, &branch, &from, &at, &repo, t)
                })
                .await;
            match made {
                Ok(head) => service.setup(ctx, op, &path, (setup, env), head).await,
                Err(error) => failed(error),
            }
        }
        OpKind::CreateWindow {
            name,
            spec,
            session_uuid,
            first_turn,
            project,
            worktree,
            jitter_ms,
            extract,
        } => {
            tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
            let first_turn = service.fill_extract(ctx, extract, first_turn).await;
            let mut spec = spec;
            if let Err(error) = worker_git_dirs(service, ctx, &mut spec).await {
                return failed(error);
            }
            let session = SessionArg::New { uuid: session_uuid };
            match service
                .manager
                .create_headless(name, *spec, session, first_turn, project, worktree)
                .await
            {
                Ok(info) => OpResult::Window {
                    window_id: info.id,
                    pid: service.manager.headless_pid(info.id),
                },
                Err(error) => failed(error.to_string()),
            }
        }
        OpKind::ResumeSession {
            window_id,
            session_id,
            message,
            jitter_ms,
        } => {
            let jitter = Duration::from_millis(jitter_ms);
            match service
                .manager
                .headless_resume(window_id, &session_id, &message, jitter)
                .await
            {
                Ok(()) => OpResult::Resumed,
                Err(error) => OpResult::ResumeFailed {
                    error: error.to_string(),
                },
            }
        }
        OpKind::VerifyDone {
            worktree,
            start,
            run_head,
            owns,
            generated,
            protected,
            spill_exempt: _,
            red,
            resolution,
            not_own,
            not_run,
            signals,
            spill_base,
            sync,
        } => {
            // Final fix batch F1b: through the queue, since each of these imports the
            // worker's commits and records them on the task's branch first.
            settle(
                service
                    .write(ctx, move |g, t| {
                        verify_done(
                            g,
                            &worktree,
                            &start,
                            &run_head,
                            (&owns, &generated, &protected),
                            (
                                red.clone(),
                                resolution.as_deref().cloned(),
                                signals.as_ref(),
                            ),
                            (
                                RefreshedIn::of(&not_own, &not_run),
                                spill_base.clone(),
                                sync.clone(),
                            ),
                            t,
                        )
                    })
                    .await,
            )
        }
        OpKind::CountCommits {
            worktree,
            start,
            run_head,
            not_own,
            not_run,
        } => settle(
            service
                .write(ctx, move |g, t| {
                    let refreshed = RefreshedIn::of(&not_own, &not_run);
                    git::count_commits_excluding(g, &worktree, &start, &run_head, &refreshed, t)
                        .map(|(count, head)| OpResult::Commits { count, head })
                })
                .await,
        ),
        OpKind::DiffSoFar {
            worktree,
            start,
            run_head,
        } => settle(
            service
                .write(ctx, move |g, t| {
                    git::diff_so_far(g, &worktree, &start, &run_head, t)
                        .map(|(stat, patch)| OpResult::Diff { stat, patch })
                })
                .await,
        ),
        OpKind::Proof {
            root,
            path,
            red,
            head,
            command,
            passed,
            timeout_secs,
            setup,
            env,
            red_only,
        } => {
            let op_id = op;
            let op = ProofOp {
                root,
                repo: git::checkout_repo_dir(&ctx.data_dir, &path),
                path,
                red,
                head,
                command,
                passed,
                timeout_secs,
                setup,
                env,
                confine: ctx.confine.as_deref().cloned(),
                red_only,
            };
            proof(service, ctx, op_id, op).await
        }
        OpKind::Check {
            dir,
            command,
            timeout_secs,
            env,
            scratch,
        } => {
            let at = (dir, scratch);
            check(service, ctx, op, at, command, timeout_secs, env).await
        }
        OpKind::PrepareReview {
            root,
            head_ref,
            base_ref,
            path,
            base_tree,
        } => settle({
            let repo = git::checkout_repo_dir(&ctx.data_dir, &path);
            service
                .write(ctx, move |g, t| {
                    let (base, head, patch) =
                        git::prepare_review_in(g, &root, &head_ref, &base_ref, &path, &repo, t)?;
                    // Controller ruling C-21 (6): a sync task's resolution only.
                    match &base_tree {
                        Some(tree) => Ok((
                            tree.clone(),
                            git::tree_patch(g, &root, tree, &head, t)?,
                            head,
                        )),
                        None => Ok((base, patch, head)),
                    }
                })
                .await
                .map(|(base, patch, head)| OpResult::Review { base, head, patch })
        }),
        OpKind::MergeCandidate { .. } => settle(merge::candidate(service, ctx, op, kind).await),
        OpKind::HandBack {
            worktree,
            run_head,
            list_merged,
            ..
        } => settle(
            service
                .write(ctx, move |g, t| {
                    git::hand_back_listing(g, &worktree, &run_head, list_merged, t)
                })
                .await
                .map(|(h, merged)| OpResult::HandedBack {
                    files: h.files,
                    head: Some(h.head),
                    onto: Some(h.onto),
                    merged: merged.lines,
                    merged_total: merged.total,
                }),
        ),
        OpKind::AbortMerge { worktree } => settle(
            service
                .write(ctx, move |g, t| git::abort_merge(g, &worktree, t))
                .await
                .map(|()| OpResult::MergeAborted),
        ),
        // Milestone 9.5 decision 22: with a race lane's variants.
        OpKind::RemoveWorktree { .. } => settle(lane_ops::remove_lane(service, ctx, kind).await),
        OpKind::VerifyRefs { .. } => stage_ops::verify_refs(service, ctx, kind).await,
        // Milestone 9.1 decision 48.
        OpKind::CreateStageBranch { .. } => stage_ops::create(service, ctx, kind).await,
        // Decision 50.
        OpKind::Propagate(_) => settle(stage_ops::propagate(service, ctx, op, kind).await),
        OpKind::Accept { .. } => cleanup::accept(service, ctx, kind).await,
        OpKind::Discard {
            root,
            worktrees,
            branch_prefix,
        } => cleanup::discard(service, ctx, root, worktrees, branch_prefix).await,
        OpKind::Decide { request, .. } => service.decide(ctx, request).await,
        kind @ (OpKind::MeasureDiff { .. } | OpKind::AppendHistory { .. }) => {
            service.history_op(ctx, kind).await
        }
        // Milestone 9 (task M9.13).
        kind @ (OpKind::CreateOrchestrator { .. }
        | OpKind::RestartOrchestrator { .. }
        | OpKind::StartScout { .. }
        | OpKind::StartPlanner { .. }
        | OpKind::StartDesignAgent { .. }
        | OpKind::ResolveTarget { .. }) => super::orch_ops::run(service, ctx, kind).await,
        // Milestone 9.1 (task M9.1.9): a tier job and a bisect probe.
        OpKind::Tier(spec) => {
            let (sched, queue) = (service.scheduler(), &service.queue);
            let cache = service.test_cache();
            tier::run_tier(ctx, sched, cache, queue, &git, op, &spec).await
        }
        OpKind::TestAt(spec) => {
            let (sched, queue) = (service.scheduler(), &service.queue);
            tier::run_test_at(ctx, sched, queue, &git, op, &spec).await
        }
        // Milestone 9.2 decision 8 (`driver/host_ops.rs`).
        OpKind::Host { repo, op } => service.host_op(ctx, repo, op).await,
        // Milestone 9.5 decision 21.
        OpKind::CrownRacer { .. } => lane_ops::crown_racer(service, ctx, kind).await,
    }
}

/// `verify_done`, then `resolution_only` when the claim follows a conflicted hand-back
/// (ruling T14-R2): an error there counts as `Some(false)` and never fails the check.
#[allow(clippy::too_many_arguments)]
fn verify_done(
    git: &OsString,
    worktree: &Path,
    start: &str,
    run_head: &str,
    (owns, generated, protected): (&[String], &[String], &[String]),
    (red, resolution, signals): (Option<String>, Option<ResolutionAt>, Option<&SignalsSpec>),
    (refreshed, spill_base, sync): (RefreshedIn, Option<String>, Option<Box<SyncCheck>>),
    git_timeout: Duration,
) -> Result<OpResult, String> {
    let t = git_timeout.min(DONE_CHECK_GIT_TIMEOUT);
    let generated = OwnsMatcher::new(generated)?;
    let protected = ProtectedMatcher::new(protected)?;
    let mut d = git::verify_done_spilling(
        git,
        worktree,
        start,
        run_head,
        owns,
        (&generated, &protected),
        red.as_deref(),
        (&refreshed, spill_base.as_deref()),
        t,
    )?;
    let resolution_only = resolution.map(|r| {
        git::resolution_only(git, worktree, &d.head, &r.onto, &r.run_head, &r.files, t)
            .unwrap_or(false)
    });
    // Milestone 9.1 decision 40: only when the op asks (never for an untiered profile).
    // Controller ruling C-21 (2): a sync task's from its conflicted tree.
    // Milestone 9.5 rulings RP-2 and T16-7: a paired task's implementer's: the writer's
    // paths from its red commit, every other path from the merge base with the run head.
    let red_at = signals.and_then(|s| s.red.as_deref());
    let mut signals =
        match (signals, red_at.or(spill_base.as_deref())) {
            (Some(spec), Some(red)) if red_at.is_some() && !d.head.is_empty() => Some(Box::new(
                git::pair_signals(git, worktree, (start, red, run_head, &d.head), spec, t)?,
            )),
            (Some(spec), Some(base)) if !d.head.is_empty() => Some(Box::new(
                git::done_signals_from(git, worktree, (base, &d.head), spec, t)?,
            )),
            (Some(spec), _) => Some(Box::new(git::done_signals(
                git, worktree, run_head, &d.head, spec, t,
            )?)),
            (None, _) => None,
        };
    let sync_kept = match sync.filter(|_| !d.head.is_empty()) {
        Some(sync) => Some(sync_done::apply(
            git,
            worktree,
            &sync,
            (&mut d, &mut signals),
            t,
        )?),
        None => None,
    };
    Ok(OpResult::DoneChecked {
        commits: d.commits,
        dirty_tracked: d.dirty_tracked,
        merge_in_progress: d.merge_in_progress,
        untracked_in_owns: d.untracked_in_owns,
        outside_owns: d.outside_owns,
        generated_outside_owns: d.generated_outside_owns,
        protected_changed: d.protected_changed,
        red_ok: d.red_ok,
        head: d.head,
        head_branch: d.head_branch,
        resolution_only,
        signals,
        sync_kept,
    })
}
