//! Milestone 9 decision 42e, driver side (task M9.13a): a refresh's clean-tree check
//! at acceptance, before the edit reaches the engine, for `run edit` and the
//! orchestrator's `edit_plan` alike. The executor checks again at the turn boundary
//! (`git::hand_back_listing`).
//!
//! The check imports the worker's `HEAD` first, as `verify_done` does (the engine never
//! reads the worker's private objects), so it runs behind the repository's write queue,
//! on `spawn_blocking`, bounded by `DONE_CHECK_GIT_TIMEOUT`, with no engine or manager
//! lock held: the engine's lock is taken only to read the task's worktree. Every git
//! command goes through `worktree::run_git`, which passes `-C <dir>
//! --no-optional-locks` and the scrubbed environment (AGENTS.md rule 11).

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use proto::{PlanEdit, TaskState};

use super::{DONE_CHECK_GIT_TIMEOUT, OpCtx, RunService};

/// Whether `worktree` has a tracked change, staged or not: `git --no-optional-locks
/// status --porcelain -z --untracked-files=no`, after the import. Blocking; behind the
/// write queue.
pub fn dirty(git: &OsStr, worktree: &Path, timeout: Duration) -> Result<bool, String> {
    crate::run::git::tracked_changes(git, worktree, timeout)
}

/// Decision 42e's refusal of a refresh whose task has uncommitted changes.
pub fn uncommitted(task_id: &str) -> String {
    format!("task {task_id} has uncommitted changes; send it a message asking it to commit first")
}

impl RunService {
    /// Decision 42e: a call whose one edit is a `refresh` of a `working` or
    /// `paused(message)` task (and that carries no `submit` or `summary`: `alone`) is
    /// refused when the task's checkout has a tracked change. Any other call, and a
    /// refresh the engine will refuse anyway, pass unchecked.
    pub(super) async fn refresh_precheck(
        &self,
        run_id: &str,
        edits: &[PlanEdit],
        alone: bool,
    ) -> Result<(), String> {
        let [PlanEdit::Refresh { task_id }] = edits else {
            return Ok(());
        };
        if !alone {
            return Ok(());
        }
        let found = crate::lock(&self.state).runs.get(run_id).and_then(|run| {
            let task = run.task(task_id)?;
            let paused = crate::run::edits_state::is_paused(task);
            let live = task.state == TaskState::Working || paused;
            let mut ctx = OpCtx::of(run);
            ctx.git_timeout = ctx.git_timeout.min(DONE_CHECK_GIT_TIMEOUT);
            live.then(|| (task.worktree.clone(), ctx))
        });
        let Some((worktree, ctx)) = found else {
            return Ok(());
        };
        let check = self.write(&ctx, move |git, timeout| dirty(git, &worktree, timeout));
        match tokio::time::timeout(DONE_CHECK_GIT_TIMEOUT, check).await {
            Ok(Ok(false)) => Ok(()),
            Ok(Ok(true)) => Err(uncommitted(task_id)),
            Ok(Err(error)) => Err(format!("could not read task {task_id}'s worktree: {error}")),
            Err(_) => Err(format!(
                "git did not answer within {} s",
                DONE_CHECK_GIT_TIMEOUT.as_secs()
            )),
        }
    }
}
