//! A detection's tail. It publishes `Ready` (the alert, `proposal.json`) while still
//! registered, and stays registered through `after_ready` and `unmark` until
//! `unregister`. `confirm` (Use this) and `refuse_if_running` (`detect`, `edit`) call
//! [`ProfileService::wait_ready_tail`] first, so a request in that window waits for the
//! registration to end instead of refusing "already running (state ready)" (the CI
//! flake of `profile_cli_tui::e2e_tui_queued_goal_review_and_use_this`). It waits at most
//! `PROFILE_READY_TAIL_BOUND`; past it the caller's own check refuses as before.

use std::path::Path;
use std::time::Duration;

use common::bounds::PROFILE_READY_TAIL_BOUND;
use proto::{ProposalState, RowEditState};

use super::service::ProfileService;

impl ProfileService {
    /// Waits while `project` is registered, not cancelled, and its proposal is `Ready`
    /// with no row edit being checked; returns at once otherwise. Holds no lock while it
    /// waits, and must be called before `hold_writes`: the tail's `after_ready` takes
    /// `writes`.
    ///
    /// No lost wakeup: the receiver is subscribed under the table lock, in the same hold
    /// that sees the registration. `unregister` drops the entry's sender, which closes
    /// every receiver, and a closed receiver's `changed()` ends at once, so an
    /// `unregister` between the subscribe and the await is still seen.
    pub(super) async fn wait_ready_tail(&self, project: &Path) {
        let mut done = {
            let table = crate::lock(&self.table);
            match table.active.get(project) {
                Some(active) if !active.token.is_cancelled() => active.done.subscribe(),
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
        // tokio's timer runs on the monotonic clock. `changed()` errs once the sender is
        // dropped; nothing ever sends on it.
        let _ = tokio::time::timeout(self.tail_bound(), done.changed()).await;
    }

    fn tail_bound(&self) -> Duration {
        #[cfg(test)]
        if let Some(bound) = *crate::lock(&self.tail_bound) {
            return bound;
        }
        PROFILE_READY_TAIL_BOUND
    }
}
