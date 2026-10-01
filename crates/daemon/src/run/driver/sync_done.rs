//! Controller ruling C-21 (3, 5): a sync task's claim, after `verify_done`. Its head must
//! still contain the lower stage's head it merged (`kept`), and the paths the upper
//! stage's own commits changed since the task's worktree started, which a later
//! hand-back brought in, are not its spill or its signals. Blocking; called on the
//! `VerifyDone` executor's queue turn.

use std::ffi::OsStr;
use std::path::Path;
use std::time::Duration;

use crate::run::git::{self, DoneChecked};
use crate::run::model::SyncCheck;
use crate::run::tiers::{ClaimSignals, Signal};

/// Reads `sync`'s check for `d.head`, drops the upper stage's paths from `d` and from
/// `signals`, and returns whether the head kept its merge.
pub(super) fn apply(
    git: &OsStr,
    worktree: &Path,
    sync: &SyncCheck,
    (d, signals): (&mut DoneChecked, &mut Option<Box<ClaimSignals>>),
    timeout: Duration,
) -> Result<bool, String> {
    let upper = (
        sync.onto.as_str(),
        sync.to_head.as_str(),
        sync.upper.as_deref(),
    );
    let (kept, paths) = git::sync_read(git, worktree, &d.head, upper, timeout)?;
    let theirs = |p: &String| paths.contains(p);
    d.outside_owns.retain(|p| !theirs(p));
    d.generated_outside_owns.retain(|p| !theirs(p));
    d.protected_changed.retain(|p| !theirs(p));
    if let Some(signals) = signals.as_mut() {
        signals.list.retain(|s| !path_of(s).is_some_and(theirs));
    }
    Ok(kept)
}

fn path_of(signal: &Signal) -> Option<&String> {
    match signal {
        Signal::DeletedTestFile { path }
        | Signal::SkipMarker { path, .. }
        | Signal::AssertionLoss { path, .. }
        | Signal::TestCodeRemoved { path, .. } => Some(path),
        Signal::DiffTooLarge => None,
    }
}
