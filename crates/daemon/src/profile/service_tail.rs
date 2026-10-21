//! A detection's tail. It publishes `Ready` (the alert, `proposal.json`) while still
//! registered, and stays registered through `after_ready` and `unmark` until
//! `unregister`. `confirm` (Use this) and `refuse_if_running` call
//! [`ProfileService::wait_ready_tail`] first, so a request in that window waits for the
//! registration to end instead of refusing "already running (state ready)" (the CI
//! flake of `profile_cli_tui::e2e_tui_queued_goal_review_and_use_this`). It waits at most
//! `PROFILE_READY_TAIL_BOUND`; past it the caller's own check refuses as before.
//! `refuse_if_running` serves `detect`, and `edit` of the stored profile. A detection's
//! `Ready` proposal is a review proposal, which `edit` refuses earlier
//! (`review_waiting`), so in a detection's tail only `confirm` and `detect` wait; `edit`
//! waits only in the tail of an `Edit`-origin proposal's verification.
//!
//! The registration ends through [`Registration`], a drop guard, so a panic anywhere in
//! the background work (or the task dropped or aborted) still ends it: before, the
//! `Active` entry stayed, and every later `confirm` or `detect` waited out the bound.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use common::bounds::PROFILE_READY_TAIL_BOUND;
use proto::{ProposalState, RowEditState};

use super::service::ProfileService;

/// The tail wait's test seam: a bound in place of `PROFILE_READY_TAIL_BOUND`, and how
/// many requests reached the await.
#[cfg(test)]
#[derive(Default)]
pub(super) struct TailSeam {
    pub(super) bound: Option<Duration>,
    pub(super) parked: usize,
    /// The panic seams (`tests_ready_panic.rs`). `after_ready` panics: a detection's
    /// tail panicking after `Ready`.
    pub(super) panic_after_ready: bool,
    /// `save_if_current` panics in its `ready_gap`: a detection's first save there is
    /// `advance(Scouting)`, before any agent starts.
    pub(super) panic_in_gap: bool,
    /// `verify_row_edit` panics after its verification.
    pub(super) panic_in_row_edit: bool,
}

/// One proposal's registration, ended by `unregister` when dropped: at the end of the
/// background work (`detect_in_background`, `verify_in_background` after `end` and
/// `unmark`; `verify_row_edit` under `writes`), or during a panic's unwind, or when the
/// task is dropped or aborted. `unregister` removes the entry only while `generation`
/// still owns it, so a newer registration is never removed. `Drop` takes only the sync
/// table lock (`daemon::lock`, which recovers from poisoning) and awaits nothing.
///
/// A panic skips `end` and `unmark`, which are async. What it leaves: `proposal.json` as
/// the last save wrote it (after `Ready`, a usable `Ready` proposal), and the detection
/// marker with any checkout the work had not discarded. The next `detect` rewrites the
/// marker and discards leftover checkouts, `reject` cleans both, and a restart salvages
/// them and fails a proposal left in progress.
pub(super) struct Registration {
    service: Arc<ProfileService>,
    project: PathBuf,
    generation: u64,
}

impl Registration {
    pub(super) fn new(service: &Arc<ProfileService>, project: &Path, generation: u64) -> Self {
        Registration {
            service: service.clone(),
            project: project.to_path_buf(),
            generation,
        }
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        self.service.unregister(&self.project, self.generation);
    }
}

impl ProfileService {
    /// Records whether `generation`'s last save for `project` published `Ready`
    /// (`save_if_current`, under `writes`). Memory only.
    pub(super) fn set_ready(&self, project: &Path, generation: u64, ready: bool) {
        let mut table = crate::lock(&self.table);
        if let Some(active) = table.active.get_mut(project)
            && active.generation == generation
        {
            active.ready = ready;
        }
    }

    /// Waits while `project`'s registration has itself published `Ready` (not a stale
    /// file under a fresh registration, review m1), is not cancelled, and the file still
    /// says `Ready` with no row edit being checked; returns at once otherwise. Holds no
    /// lock while it waits, and must be called before `hold_writes`: the tail's
    /// `after_ready` takes `writes`.
    ///
    /// No lost wakeup: the receiver is subscribed under the table lock, in the same hold
    /// that sees the registration. `unregister` drops the entry's sender, which closes
    /// every receiver, and a closed receiver's `changed()` ends at once, so an
    /// `unregister` between the subscribe and the await is still seen.
    pub(super) async fn wait_ready_tail(&self, project: &Path) {
        let mut done = {
            let table = crate::lock(&self.table);
            match table.active.get(project) {
                Some(active) if active.ready && !active.token.is_cancelled() => {
                    active.done.subscribe()
                }
                _ => return,
            }
        };
        // Read after the subscribe: whatever the file says, an `unregister` from here on
        // ends the wait.
        let Ok((_, Some(record))) = self.load(project).await else {
            return;
        };
        let checking = record
            .edit
            .as_ref()
            .is_some_and(|edit| edit.state == RowEditState::Verifying);
        if record.state != ProposalState::Ready || checking {
            return;
        }
        self.parked();
        // tokio's timer runs on the monotonic clock. `changed()` errs once the sender is
        // dropped; nothing ever sends on it.
        let _ = tokio::time::timeout(self.tail_bound(), done.changed()).await;
    }

    #[cfg(test)]
    fn parked(&self) {
        crate::lock(&self.tail_seam).parked += 1;
    }

    #[cfg(not(test))]
    fn parked(&self) {}

    /// A panic seam (`TailSeam`'s `panic_*` flags): panics when the test set `at`'s.
    /// The seam's lock is released first, so it is never poisoned.
    #[cfg(test)]
    pub(super) fn panic_seam(&self, at: fn(&TailSeam) -> bool) {
        let panics = at(&crate::lock(&self.tail_seam));
        assert!(!panics, "the test panicked this work at its seam");
    }

    fn tail_bound(&self) -> Duration {
        #[cfg(test)]
        if let Some(bound) = crate::lock(&self.tail_seam).bound {
            return bound;
        }
        PROFILE_READY_TAIL_BOUND
    }
}
