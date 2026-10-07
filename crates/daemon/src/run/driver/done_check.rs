//! `VerifyDone`'s check (M8a.22, ruling T14-R2, milestone 9.1 decision 40, controller
//! ruling C-21, milestone 9.5 rulings RP-2 and T16-7): a worker's claim against real
//! git, run on `spawn_blocking` by `ops.rs`. Split out of `ops.rs` by the 600-line rule.

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use super::super::DONE_CHECK_GIT_TIMEOUT;
use super::sync_done;
use crate::run::engine::{OpResult, ResolutionAt};
use crate::run::git::{self, RefreshedIn};
use crate::run::globs::{OwnsMatcher, ProtectedMatcher};
use crate::run::model::SyncCheck;
use crate::run::tiers::SignalsSpec;

/// `verify_done`, then `resolution_only` when the claim follows a conflicted hand-back
/// (ruling T14-R2): an error there counts as `Some(false)` and never fails the check.
#[allow(clippy::too_many_arguments)]
pub(super) fn verify_done(
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
