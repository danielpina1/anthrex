//! Milestone 9.10 decisions 4 to 11: the goals waiting for a repository's profile.
//!
//! **Locks.** Every read and write of a queue file runs on `blocking` with the
//! service's `writes` mutex held, so the file and `Table.queued` change together and in
//! order with `proposal.json`. The table's lock is held only to read or replace the
//! memory copy, each guard in its own block or expression, never across an await.
//! The starter runs with no lock of this service held.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use proto::{ProposalOrigin, QueuedGoalInfo, RowEditState};

use super::queue::{self, GoalQueue, MAX_QUEUED, QueuedGoal};
use super::service::{ProfileService, blocking};
use super::service_requests::ready_profile;
use super::service_run::confirm_record;
use super::store;
use crate::run::driver::unix_now;

/// Decision 6: starts one drained goal; `Ok` is the run id, `Err` the refusal.
pub type GoalStarter = Arc<
    dyn Fn(QueuedGoal) -> Pin<Box<dyn Future<Output = Result<String, String>> + Send>>
        + Send
        + Sync,
>;

/// Decision 8's reason for the goals `profile reject` drops.
pub const DISCARDED: &str = "the proposal was discarded";
/// Decision 11's reason for a goal whose directory went away.
pub const GONE: &str = "the repository is gone";
/// Final review I1: the reason for a goal a daemon stop caught while it was starting.
/// It may have started (decision 6 starts a goal at most once), so it is never started
/// again; the user looks and starts it again.
pub const INTERRUPTED: &str = "the daemon stopped while the goal was starting; if anthrex run status shows no run for it, start it again";

/// The project's short name in messages: its directory's name.
pub(super) fn name_of(project: &Path) -> String {
    project.file_name().map_or_else(
        || project.display().to_string(),
        |name| name.to_string_lossy().into_owned(),
    )
}

/// Decision 5's refusal of a ninth goal.
fn queue_full(project: &Path) -> String {
    format!(
        "{} already has {MAX_QUEUED} goals waiting for its profile; review it first (C-b P, or anthrex profile status)",
        name_of(project)
    )
}

/// `; starting 1 queued goal` / `; starting <n> queued goals`; empty for none.
fn starting(n: usize) -> String {
    match n {
        0 => String::new(),
        1 => "; starting 1 queued goal".to_string(),
        n => format!("; starting {n} queued goals"),
    }
}

/// `; dropped 1 queued goal` / `; dropped <n> queued goals`; empty for none.
pub(super) fn dropped_note(n: usize) -> String {
    match n {
        0 => String::new(),
        1 => "; dropped 1 queued goal".to_string(),
        n => format!("; dropped {n} queued goals"),
    }
}

impl ProfileService {
    /// Decision 6: installs the callback a drain starts goals with.
    pub fn set_goal_starter(&self, f: GoalStarter) {
        *crate::lock(&self.starter) = Some(f);
    }

    /// `project`'s queue as memory holds it.
    fn queue_of(&self, project: &Path) -> GoalQueue {
        crate::lock(&self.table)
            .queued
            .get(project)
            .cloned()
            .unwrap_or_default()
    }

    /// Replaces `project`'s queue: the file first, then memory. The caller holds
    /// `writes`.
    async fn put_queue(&self, project: &Path, queue: GoalQueue) -> Result<(), String> {
        let (dir, saved) = (self.repo_dir(project), queue.clone());
        blocking(move || queue::save(&dir, &saved).map_err(|e| e.to_string())).await?;
        self.remember_queue(project, queue);
        Ok(())
    }

    /// Memory's copy of `project`'s queue, and decision 10's bump.
    pub(super) fn remember_queue(&self, project: &Path, queue: GoalQueue) {
        let mut table = crate::lock(&self.table);
        if queue.is_empty() {
            table.queued.remove(project);
        } else {
            table.queued.insert(project.to_path_buf(), queue);
        }
        table.setup_generation += 1;
    }

    /// Decision 5: `goal` queued for its project; `Err` is the refusal (the queue is
    /// full, or its file could not be written).
    ///
    /// `Ok(None)`: the repository has a profile file now (fix round 1, I1: a store and
    /// its drain ran since the caller found none). Nothing is queued, since no drain
    /// would come for it; the caller starts the goal at once. Checked under `writes`,
    /// which every store and every drain's take-out also hold.
    pub async fn queue_goal(&self, goal: QueuedGoal) -> Result<Option<QueuedGoalInfo>, String> {
        let project = goal.project.clone();
        {
            let _writes = self.writes.lock().await;
            let dir = self.repo_dir(&project);
            let absent =
                blocking(move || Ok(matches!(store::load(&dir), store::Stored::Absent))).await?;
            if !absent {
                return Ok(None);
            }
            let mut queue = self.queue_of(&project);
            if queue.goals.len() >= MAX_QUEUED {
                return Err(queue_full(&project));
            }
            queue.goals.push(goal.clone());
            self.put_queue(&project, queue).await?;
        }
        let listed = self.queued_goals();
        Ok(Some(
            listed
                .into_iter()
                .find(|info| info.id == goal.id)
                .unwrap_or_else(|| queue::info(&goal, &proto::SetupState::Reading)),
        ))
    }

    /// The snapshot's list; memory only, one short hold of the table.
    pub fn queued_goals(&self) -> Vec<QueuedGoalInfo> {
        let table = crate::lock(&self.table);
        let mut listed = Vec::new();
        // Final review I1: a drain's goals wait for their start, not for a set-up.
        let queues = table
            .queued
            .iter()
            .filter(|(p, _)| !table.draining.contains(*p));
        for (project, queue) in queues {
            let checking = table
                .active
                .get(project)
                .and_then(|active| active.counter.progress());
            let setup = queue::setup_state(table.states.get(project), checking);
            listed.extend(queue.goals.iter().map(|goal| queue::info(goal, &setup)));
        }
        listed
    }

    /// `profile status`'s queued and dropped goals for `project`.
    pub(super) fn queued_for(
        &self,
        project: &Path,
    ) -> (Vec<QueuedGoalInfo>, Vec<proto::DroppedGoal>) {
        let dropped = self.queue_of(project).dropped;
        let queued = self
            .queued_goals()
            .into_iter()
            .filter(|info| info.project == project)
            .collect();
        (queued, dropped)
    }

    /// Decision 10: `ready_generation + setup_generation + progress_ticks`, three
    /// counters that only grow, so the sum moves whenever any of them does.
    pub fn snapshot_generation(&self) -> u64 {
        let (ready, setup) = {
            let table = crate::lock(&self.table);
            (table.ready_generation, table.setup_generation)
        };
        // Acquire pairs with `Steps::count`'s Release: a reader that sees a tick sees
        // its `done` too.
        ready + setup + self.progress_ticks.load(Ordering::Acquire)
    }

    /// Decision 9: when a detection's proposal first turns `Ready`, a queued `yes` goal
    /// stores it, provided verification dropped nothing; otherwise the review alert is
    /// raised as for any ready proposal (nothing to do here).
    pub(super) async fn after_ready(self: &Arc<Self>, project: &Path) {
        if !self.queue_of(project).goals.iter().any(|goal| goal.yes) {
            return;
        }
        match self.store_ready(project, true).await {
            Ok(message) => tracing::info!(%message, "stored a proposal for a queued --yes goal"),
            Err(error) => tracing::info!(%error, "a queued --yes goal waits for the review"),
        }
    }

    /// Decision 9: `confirm` with no `shown` (no `writes` held by the caller): the ready
    /// proposal stored, then the queue drained.
    pub async fn use_ready(self: &Arc<Self>, project: &Path) -> Result<String, String> {
        self.store_ready(project, false).await
    }

    /// The ready proposal stored and the queue drained, the proposal read, checked and
    /// stored under one hold of `writes`. `unattended` (`after_ready`): only a review
    /// proposal with no row edit and nothing dropped is stored.
    async fn store_ready(
        self: &Arc<Self>,
        project: &Path,
        unattended: bool,
    ) -> Result<String, String> {
        let _writes = self.writes.lock().await;
        let dir = self.repo_dir(project);
        let p = project.to_path_buf();
        let message = blocking(move || {
            let record = store::load_proposal(&dir)?
                .ok_or_else(|| format!("no proposal to use for {}", p.display()))?;
            let review = !matches!(record.origin, ProposalOrigin::Edit { .. })
                && record.edit.is_none()
                && record.dropped.is_empty();
            if unattended && !review {
                return Err(format!("the proposal for {} needs a review", p.display()));
            }
            // Decision 15 (M9.10.6 fix round I1): never a held ✗ of the stored profile.
            if let Some(refusal) = super::row_edit::refuse_held(&record) {
                return Err(refusal);
            }
            // Milestone 9.10.6: never while a row edit of the proposal is being checked.
            if let Some(edit) = record
                .edit
                .as_ref()
                .filter(|e| e.state == RowEditState::Verifying)
            {
                return Err(super::row_edit::still_checking(&edit.key));
            }
            ready_profile(&record)?;
            confirm_record(&dir, &p, &record)
        })
        .await?;
        self.note_proposal(project, None);
        // Counted under `writes`, so no other drain takes these goals meanwhile.
        Ok(format!("{message}{}", self.drain_after_store(project)))
    }

    /// Decision 6, after a store: spawns the drain and says how many goals it starts.
    pub(super) fn drain_after_store(self: &Arc<Self>, project: &Path) -> String {
        let n = self.queue_of(project).goals.len();
        if n > 0 {
            tokio::spawn(self.clone().start_queued(project.to_path_buf()));
        }
        starting(n)
    }

    /// Decision 6: starts `project`'s queued goals in queue order, one drain per
    /// project (a second call while one runs returns; the running one takes every goal).
    /// Final review I1: each goal is taken out alone, moved from `goals` to `starting`
    /// under `writes` (file first, then memory) before the starter gets it, and leaves
    /// `starting` when its start returns. A daemon stop then loses no goal: those not
    /// yet taken wait in `goals` for the restart's drain, and the one being started is
    /// a dropped goal at restore, never started twice. The starter runs with no lock of
    /// this service held. A refused start is recorded as a drop.
    pub(super) async fn start_queued(self: Arc<Self>, project: PathBuf) {
        {
            let _writes = self.writes.lock().await;
            let mut table = crate::lock(&self.table);
            if !table.draining.insert(project.clone()) {
                return;
            }
            table.setup_generation += 1;
        }
        let starter = crate::lock(&self.starter).clone();
        while let Some(goal) = self.take_next(&project).await {
            let started = match &starter {
                Some(start) => start(goal.clone()).await,
                None => Err("the daemon cannot start goals yet".to_string()),
            };
            self.start_returned(&project, &goal, started).await;
        }
    }

    /// The drain's next goal, moved to `starting`; `None` ends the drain, under the same
    /// hold of `writes` that found nothing left. Final review M5: when the move cannot
    /// be written, nothing starts and every goal left is dropped with the error, so none
    /// waits behind the stored profile until a restart.
    async fn take_next(&self, project: &Path) -> Option<QueuedGoal> {
        let _writes = self.writes.lock().await;
        let mut queue = self.queue_of(project);
        if queue.goals.is_empty() {
            self.end_drain(project);
            return None;
        }
        let goal = queue.goals.remove(0);
        queue.starting.push(goal.clone());
        match self.put_queue(project, queue).await {
            Ok(()) => Some(goal),
            Err(error) => {
                let reason =
                    format!("could not take the goal out of its queue ({error}); start it again");
                let mut queue = self.queue_of(project);
                for goal in std::mem::take(&mut queue.goals) {
                    tracing::info!("{}", queue::drop_line(&goal.goal, &reason));
                    queue.record_drop(&goal.goal, &reason, unix_now());
                }
                self.keep_queue(project, queue).await;
                self.end_drain(project);
                None
            }
        }
    }

    /// The drain of `project` over; the caller holds `writes`.
    fn end_drain(&self, project: &Path) {
        let mut table = crate::lock(&self.table);
        table.draining.remove(project);
        table.setup_generation += 1;
    }

    /// `goal`'s start returned: it leaves `starting`, and a refusal is recorded as a
    /// drop (decision 6).
    async fn start_returned(
        &self,
        project: &Path,
        goal: &QueuedGoal,
        started: Result<String, String>,
    ) {
        let _writes = self.writes.lock().await;
        let mut queue = self.queue_of(project);
        queue.starting.retain(|g| g.id != goal.id);
        match started {
            Ok(run) => tracing::info!(%run, goal = %goal.id, "started a queued goal"),
            Err(reason) => {
                tracing::info!("{}", queue::drop_line(&goal.goal, &reason));
                queue.record_drop(&goal.goal, &reason, unix_now());
            }
        }
        self.keep_queue(project, queue).await;
    }

    /// Writes `queue` and keeps it in memory even when the write fails (logged): memory
    /// then says what happened, and the next write that succeeds puts it on file. The
    /// caller holds `writes`.
    async fn keep_queue(&self, project: &Path, queue: GoalQueue) {
        let (dir, saved) = (self.repo_dir(project), queue.clone());
        if let Err(error) =
            blocking(move || queue::save(&dir, &saved).map_err(|e| e.to_string())).await
        {
            tracing::warn!(%error, project = %project.display(), "could not write the goal queue");
        }
        self.remember_queue(project, queue);
    }

    /// Decision 8: every goal queued for `project` dropped with `reason`; how many.
    /// Production code holds `writes` already (`reject`) and calls
    /// [`Self::drop_queued_locked`]; this takes the hold itself, for the tests.
    #[cfg(test)]
    pub(super) async fn drop_queued(&self, project: &Path, reason: &str) -> usize {
        let writes = self.writes.lock().await;
        self.drop_queued_locked(&writes, project, reason)
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "could not drop the queued goals");
                0
            })
    }

    /// [`Self::drop_queued`] under the caller's hold of `writes` (`reject`, before it
    /// deletes the proposal, so no goal queued after it is dropped and a crash never
    /// leaves goals waiting on a proposal that is gone). `Err`: the file could not be
    /// written, and nothing changed.
    pub(super) async fn drop_queued_locked(
        &self,
        _writes: &tokio::sync::MutexGuard<'_, ()>,
        project: &Path,
        reason: &str,
    ) -> Result<usize, String> {
        let mut queue = self.queue_of(project);
        let goals = std::mem::take(&mut queue.goals);
        if goals.is_empty() {
            return Ok(0);
        }
        for goal in &goals {
            queue.record_drop(&goal.goal, reason, unix_now());
        }
        self.put_queue(project, queue).await?;
        for goal in &goals {
            tracing::info!("{}", queue::drop_line(&goal.goal, reason));
        }
        Ok(goals.len())
    }

    /// Decision 11, at start: drops `project`'s goals whose directory is gone, then
    /// drains the queue when the profile is stored.
    pub(super) async fn drain_at_start(self: Arc<Self>, project: PathBuf) {
        {
            let _writes = self.writes.lock().await;
            let mut queue = self.queue_of(&project);
            let dirs: Vec<PathBuf> = queue.goals.iter().map(|goal| goal.dir.clone()).collect();
            let n = dirs.len();
            let present: Vec<bool> =
                blocking(move || Ok(dirs.iter().map(|dir| dir.is_dir()).collect()))
                    .await
                    .unwrap_or_else(|_| vec![true; n]);
            let mut kept = Vec::new();
            let mut gone = 0;
            for (goal, present) in std::mem::take(&mut queue.goals).into_iter().zip(present) {
                if present {
                    kept.push(goal);
                } else {
                    tracing::info!("{}", queue::drop_line(&goal.goal, GONE));
                    queue.record_drop(&goal.goal, GONE, unix_now());
                    gone += 1;
                }
            }
            queue.goals = kept;
            if gone > 0
                && let Err(error) = self.put_queue(&project, queue).await
            {
                tracing::warn!(%error, "could not drop a queued goal");
            }
        }
        let dir = self.repo_dir(&project);
        let stored = blocking(move || Ok(matches!(store::load(&dir), store::Stored::Found { .. })))
            .await
            .unwrap_or(false);
        if stored {
            self.start_queued(project).await;
        }
    }
}
