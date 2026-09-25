//! Each engine op, executed (M8a.22): the executor contracts in `engine/ops.rs`'s
//! `OpKind` docs. Git writes go through `GitQueue::write`, keyed by the run's project
//! (decision 18); every other blocking call runs on `spawn_blocking`; sessions start
//! through the manager's `headless_*` methods. No lock is held here.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use super::{DONE_CHECK_GIT_TIMEOUT, OpCtx, RunService, cleanup, merge};
use crate::headless::{HeadlessSpec, SessionArg};
use crate::run::engine::{EventKind, OpKind, OpResult, ResolutionAt, ScratchAt};
use crate::run::exec::{ShellOutcome, run_shell};
use crate::run::git::{self, RefCheck};
use crate::run::globs::{OwnsMatcher, ProtectedMatcher};
use crate::run::proof::{ProofError, ProofOp, SETUP_MARKER, run_proof};
use crate::run::role_launch::{task_objects_dir, with_worker_objects};
use proto::AgentRole;

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
    /// `SetupFailed` with its tail.
    async fn setup(
        &self,
        ctx: &OpCtx,
        dir: &Path,
        setup: Option<String>,
        env: Vec<(String, String)>,
        head: String,
    ) -> OpResult {
        let Some(setup) = setup else {
            return OpResult::Worktree { head };
        };
        let (dir, timeout) = (dir.to_path_buf(), ctx.check_timeout);
        match blocking(move || Ok(run_shell(&dir, &setup, &env, timeout))).await {
            Ok(outcome) if outcome.ok => OpResult::Worktree { head },
            Ok(outcome) => OpResult::SetupFailed {
                output: setup_output(&outcome),
            },
            Err(error) => failed(error),
        }
    }
}

/// Final fix batch F1b: the private object directory of the task whose worktree is
/// `worktree` in the run of `ctx`, `None` when no task of the run has that worktree.
fn task_objects(service: &Arc<RunService>, ctx: &OpCtx, worktree: &Path) -> Option<PathBuf> {
    let state = crate::lock(&service.state);
    let run = state.runs.get(&ctx.run_id)?;
    let task = run.tasks.iter().find(|task| task.worktree == worktree)?;
    Some(task_objects_dir(&ctx.data_dir, task.id()))
}

/// A worker's object directories and sandbox, completed at launch (final fix batch F1
/// and its fix round 1; F1b): its private object directory created (by the daemon,
/// never through a link) and named canonically in its environment, with the
/// repository's object store as its read-only alternate; and, when it is sandboxed, its
/// writable roots: that directory plus the commit's files in the worktree's own git
/// directory, found from the repository's side. Nothing of the git common directory.
/// Any other session (a reviewer) is left as it is.
async fn worker_git_dirs(
    service: &Arc<RunService>,
    ctx: &OpCtx,
    spec: &mut HeadlessSpec,
) -> Result<(), String> {
    let task = spec
        .run_ref
        .as_ref()
        .filter(|r| r.role == AgentRole::Worker)
        .and_then(|r| r.task_id.clone());
    let Some(task) = task else {
        return Ok(());
    };
    let common = crate::lock(&service.state)
        .runs
        .get(&ctx.run_id)
        .map(|run| run.git_common_dir.clone())
        .ok_or_else(|| format!("unknown run {}", ctx.run_id))?;
    let objects = task_objects_dir(&ctx.data_dir, &task);
    let sandboxed = spec
        .claude_sandbox
        .as_ref()
        .is_some_and(|s| !s.writable_roots.is_empty())
        || !spec.codex_writable_roots.is_empty();
    let cwd = spec.cwd.clone();
    let (objects, dirs) = service
        .write(ctx, move |_, _| {
            let objects = git::private_dir(&common, &objects)?;
            let dirs = if sandboxed {
                git::worker_git_dirs(&common, &cwd, std::slice::from_ref(&objects))?
            } else {
                Vec::new()
            };
            Ok((objects, dirs))
        })
        .await?;
    let common_objects = crate::lock(&service.state)
        .runs
        .get(&ctx.run_id)
        .map(|run| run.git_common_dir.join("objects"))
        .ok_or_else(|| format!("unknown run {}", ctx.run_id))?;
    spec.env = with_worker_objects(std::mem::take(&mut spec.env), &objects, &common_objects);
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

/// Executes `kind` for the run of `ctx`.
pub(super) async fn run(service: &Arc<RunService>, ctx: &OpCtx, kind: OpKind) -> OpResult {
    let git = service.git();
    let t = ctx.git_timeout;
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
                Ok(head) => service.setup(ctx, &path, setup, env, head).await,
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
            // Final fix batch F1b: the worker's objects are imported from its private
            // directory.
            let objects = task_objects(service, ctx, &path);
            let made = service
                .write(ctx, move |g, t| {
                    git::prepare_task_worktree(g, &root, &branch, &from, &at, objects.as_deref(), t)
                })
                .await;
            match made {
                Ok(head) => service.setup(ctx, &path, setup, env, head).await,
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
        } => {
            tokio::time::sleep(Duration::from_millis(jitter_ms)).await;
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
                            &owns,
                            &generated,
                            &protected,
                            red.clone(),
                            resolution.clone(),
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
        } => settle(
            service
                .write(ctx, move |g, t| {
                    git::count_commits(g, &worktree, &start, &run_head, t)
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
        } => {
            let op = ProofOp {
                root,
                path,
                red,
                head,
                command,
                passed,
                timeout_secs,
                setup,
                env,
            };
            proof(service, ctx, op).await
        }
        OpKind::Check {
            dir,
            command,
            timeout_secs,
            env,
            scratch,
        } => check(service, ctx, dir, command, timeout_secs, env, scratch).await,
        OpKind::PrepareReview {
            root,
            head_ref,
            base_ref,
            path,
        } => settle(
            service
                .write(ctx, move |g, t| {
                    git::prepare_review(g, &root, &head_ref, &base_ref, &path, t)
                })
                .await
                .map(|(base, head, patch)| OpResult::Review { base, head, patch }),
        ),
        OpKind::MergeCandidate { .. } => settle(merge::candidate(service, ctx, kind).await),
        OpKind::HandBack {
            worktree,
            run_head,
            task_head: _,
        } => settle(
            service
                .write(ctx, move |g, t| git::hand_back(g, &worktree, &run_head, t))
                .await
                .map(|h| OpResult::HandedBack {
                    files: h.files,
                    head: Some(h.head),
                    onto: Some(h.onto),
                }),
        ),
        OpKind::AbortMerge { worktree } => settle(
            service
                .write(ctx, move |g, t| git::abort_merge(g, &worktree, t))
                .await
                .map(|()| OpResult::MergeAborted),
        ),
        OpKind::RemoveWorktree {
            root,
            path,
            salvage_ref,
        } => settle(
            cleanup::remove_worktree(service, ctx, root, path, salvage_ref)
                .await
                .map(|salvage_ref| OpResult::Removed { salvage_ref }),
        ),
        OpKind::VerifyRefs {
            root,
            base_branch,
            expected_base,
            run_branch,
            expected_run_head,
        } => {
            let checked = blocking(move || {
                git::guard_refs(
                    &git,
                    &root,
                    &base_branch,
                    &expected_base,
                    &run_branch,
                    &expected_run_head,
                    t,
                )
            })
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
        OpKind::Accept { .. } => cleanup::accept(service, ctx, kind).await,
        OpKind::Discard {
            root,
            worktrees,
            branch_prefix,
        } => cleanup::discard(service, ctx, root, worktrees, branch_prefix).await,
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
    owns: &[String],
    generated: &[String],
    protected: &[String],
    red: Option<String>,
    resolution: Option<ResolutionAt>,
    git_timeout: Duration,
) -> Result<OpResult, String> {
    let t = git_timeout.min(DONE_CHECK_GIT_TIMEOUT);
    let generated = OwnsMatcher::new(generated)?;
    let protected = ProtectedMatcher::new(protected)?;
    let d = git::verify_done(
        git,
        worktree,
        start,
        run_head,
        owns,
        &generated,
        &protected,
        red.as_deref(),
        t,
    )?;
    let resolution_only = resolution.map(|r| {
        git::resolution_only(git, worktree, &d.head, &r.onto, &r.run_head, &r.files, t)
            .unwrap_or(false)
    });
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
    })
}

/// Decision 33's proof on a blocking thread, each git step through the queue by
/// `Handle::block_on` (the daemon's multi-thread runtime; see `OpKind::Proof`).
async fn proof(service: &Arc<RunService>, ctx: &OpCtx, op: ProofOp) -> OpResult {
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
async fn check(
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
        if let Err(error) = service
            .write(ctx, move |g, t| git::prepare_scratch(g, &root, &at, &c, t))
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
                let (at, env) = (dir.clone(), env.clone());
                let outcome = blocking(move || Ok(run_shell(&at, &setup, &env, timeout))).await;
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
    match blocking(move || Ok(run_shell(&dir, &command, &env, timeout))).await {
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
