//! Milestone 9.10 decisions 4 to 6: the goals waiting for a repository's profile, kept
//! in `<repo_dir>/queued_goals.json` ([`QUEUE_FILE`]) beside `proposal.json`.
//!
//! Pure except [`load`] and [`save`], the file's only I/O, which the service calls on
//! `spawn_blocking` under its `writes` mutex (`service_queue.rs`).

use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use proto::{CheckProgress, DroppedGoal, ProposalState, QueuedGoalInfo, SetupState};
use serde::{Deserialize, Serialize};

/// Decision 4: the queue's file in the repository's data directory.
pub const QUEUE_FILE: &str = "queued_goals.json";
/// Decision 5: the most goals one repository may queue.
pub const MAX_QUEUED: usize = 8;
/// Decision 6: how many dropped goals are remembered.
pub const DROPPED_KEPT: usize = 5;

/// Decision 5: a `StartGoal` waiting for its repository's profile, the whole request.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedGoal {
    pub id: String,
    pub project: PathBuf,
    pub dir: PathBuf,
    pub goal: String,
    /// Unix seconds.
    pub queued_at: u64,
    pub yes: bool,
    pub trust_project: bool,
    pub unconfined_checks: bool,
    pub orchestrator: Option<proto::OrchestratorChoice>,
    pub delivery: Option<proto::DeliveryMode>,
    pub design: Option<proto::DesignMode>,
}

/// One repository's queue and its recent drops (decision 4).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GoalQueue {
    pub goals: Vec<QueuedGoal>,
    pub dropped: Vec<DroppedGoal>,
}

impl GoalQueue {
    /// Nothing queued and nothing dropped: the file is removed.
    pub fn is_empty(&self) -> bool {
        self.goals.is_empty() && self.dropped.is_empty()
    }

    /// Decision 6: remembers a goal that could not start, the last [`DROPPED_KEPT`].
    pub fn record_drop(&mut self, goal: &str, reason: &str, at: u64) {
        self.dropped.push(DroppedGoal {
            goal: goal.to_string(),
            reason: reason.to_string(),
            at,
        });
        let extra = self.dropped.len().saturating_sub(DROPPED_KEPT);
        self.dropped.drain(..extra);
    }
}

/// Decision 6's log line for a dropped goal, its text cut to 60 characters.
pub fn drop_line(goal: &str, reason: &str) -> String {
    let short: String = goal.chars().take(60).collect();
    format!("dropped the queued goal \"{short}\": {reason}")
}

/// The process-wide counter in [`next_id`], as `TRIAGE_SEQ`.
static QUEUE_SEQ: AtomicU64 = AtomicU64::new(1);

/// Decision 5: `q-<unix nanos>-<n>`, unique even for two ids in one nanosecond.
pub fn next_id(now_nanos: u128) -> String {
    let n = QUEUE_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("q-{now_nanos}-{n}")
}

/// Decision 10: where the set-up stands, from the repository's proposal state (the
/// table's `states`) and its running verification's progress.
pub fn setup_state(state: Option<&ProposalState>, checking: Option<CheckProgress>) -> SetupState {
    match state {
        None | Some(ProposalState::Preparing | ProposalState::Scouting) => SetupState::Reading,
        Some(ProposalState::Verifying) => SetupState::Checking { progress: checking },
        Some(ProposalState::Ready) => SetupState::NeedsReview,
        Some(ProposalState::Failed { reason }) => SetupState::Failed {
            reason: reason.clone(),
        },
    }
}

/// A queued goal as the snapshot and `profile status` list it.
pub fn info(goal: &QueuedGoal, setup: &SetupState) -> QueuedGoalInfo {
    QueuedGoalInfo {
        id: goal.id.clone(),
        project: goal.project.clone(),
        goal: goal.goal.clone(),
        queued_at: goal.queued_at,
        yes: goal.yes,
        trust_project: goal.trust_project,
        unconfined_checks: goal.unconfined_checks,
        setup: setup.clone(),
    }
}

/// The queue in `repo_dir`; an absent file is an empty queue. Blocking.
pub fn load(repo_dir: &Path) -> Result<GoalQueue, String> {
    let path = repo_dir.join(QUEUE_FILE);
    match std::fs::read(&path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|error| format!("{} does not parse: {error}", path.display())),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(GoalQueue::default()),
        Err(error) => Err(format!("could not read {}: {error}", path.display())),
    }
}

/// Writes `queue` atomically (temp file and rename, as `store::save_proposal`), or
/// removes the file when the queue is empty. Blocking.
pub fn save(repo_dir: &Path, queue: &GoalQueue) -> io::Result<()> {
    let path = repo_dir.join(QUEUE_FILE);
    if queue.is_empty() {
        return match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        };
    }
    let json = serde_json::to_vec_pretty(queue).map_err(io::Error::other)?;
    super::store::write_atomic(&path, &json)
}
