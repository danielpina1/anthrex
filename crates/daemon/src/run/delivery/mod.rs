//! Milestone 9.2's delivery model (design decision 1: `run/delivery/` is pure): what a
//! run freezes at its start (decisions 3 and 16, task M9.2.3), the stages' pull
//! requests, their watermarks, CI records and threads (decisions 19–37, task M9.2.6),
//! the host ops ([`ops`]), the PR title and body ([`body`]), the quoting of untrusted
//! text ([`quote`]) and what the snapshot shows of it all ([`snapshot`]).

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use proto::{CiCategory, CiState, DeliveryMode, MergeMethod, PrState};

use crate::host::{HostRepo, Mergeable, RepoPermission};

pub mod body;
pub mod ops;
pub mod quote;
pub mod snapshot;

/// Decision 21: a PR body is cut to this many characters (GitHub's limit is 65 536).
pub const BODY_MAX_CHARS: usize = 60_000;
/// Decision 11: failures in a row of one op kind on one stage before the attention line.
pub const FAILURES_BEFORE_ATTENTION: u32 = 5;
/// Decision 27: a stage keeps at most this many CI records, newest last.
pub const CI_RECORDS_MAX: usize = 20;
/// Decision 41: a stage PR's checks shown, of its most recent head.
pub const CHECKS_MAX: usize = 20;

/// A run's delivery, frozen at its start; `#[serde(default)]` on `Run`, so a 9.1
/// `run.json` loads `local` with the default limits and nothing delivered.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunDelivery {
    /// Decision 3: the resolved mode (the profile's, or `--delivery`).
    pub mode: DeliveryMode,
    /// Decision 17: preflight's answer; `None` in local mode.
    pub repo: Option<HostRepo>,
    /// Decision 16: `[delivery]`, frozen.
    pub limits: DeliveryLimits,
    /// Decision 25: polling is on; true at start in pr mode.
    pub watching: bool,
    /// Decision 11: the poll interval base, `poll_secs` doubled by rate limits.
    pub poll_base_secs: u64,
    /// One per stage, index = stage − 1.
    pub stages: Vec<StageDelivery>,
    /// Decision 29: write access per login, cached for the run.
    pub permissions: BTreeMap<String, RepoPermission>,
    /// Decision 33: the base commit the stages last absorbed.
    pub base_synced: Option<String>,
    /// Decision 33: stage → the base sha it is due to absorb.
    pub base_sync_due: BTreeMap<u16, String>,
    /// Decision 11: `"<stage>/<op>"` → failures in a row.
    pub failures: BTreeMap<String, u32>,
}

impl RunDelivery {
    /// Stage `n`'s delivery, when it has one.
    pub fn stage(&self, n: u16) -> Option<&StageDelivery> {
        self.stages.get(usize::from(n).checked_sub(1)?)
    }

    /// Stage `n`'s pull request, when it has one.
    pub fn pr(&self, n: u16) -> Option<&PrRecord> {
        self.stage(n)?.pr.as_ref()
    }

    /// Decision 36: a `pr`-mode run is *delivering* when each of its `stage_count`
    /// stages has an open or merged PR or was skipped, and at least one PR is open.
    pub fn delivering(&self, stage_count: u16) -> bool {
        if self.mode != DeliveryMode::Pr || stage_count == 0 {
            return false;
        }
        let state = |n: u16| self.pr(n).map(|p| p.state);
        let covered = (1..=stage_count).all(|n| {
            self.stage(n).is_some_and(|s| s.skipped)
                || matches!(state(n), Some(PrState::Open | PrState::Merged))
        });
        covered && (1..=stage_count).any(|n| state(n) == Some(PrState::Open))
    }
}

/// One stage's delivery (decisions 19–37).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StageDelivery {
    pub pr: Option<PrRecord>,
    /// Decision 19: every task cancelled, so no PR opens.
    pub skipped: bool,
    /// Decision 44's `time_to_open`: when the stage's tasks had all finished.
    pub ready_at: Option<u64>,
    /// Decision 37: the lower stage whose PR was closed without merging.
    pub paused_by: Option<u16>,
    /// Decision 27, newest last, at most [`CI_RECORDS_MAX`].
    pub ci: Vec<CiRecord>,
    /// Decisions 29–31.
    pub threads: Vec<ThreadRecord>,
    pub batch: Option<Batch>,
    pub review_rounds: u32,
    /// Decision 43: seconds open with no fix task in flight and no CI running.
    pub review_wait_secs: u64,
    pub history_written: bool,
}

/// A stage's pull request (decisions 20, 23, 35).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrRecord {
    pub number: u64,
    pub url: String,
    pub base: String,
    pub opened_at: u64,
    pub pushed_head: String,
    pub state: PrState,
    #[serde(default)]
    pub merged_at: Option<u64>,
    #[serde(default)]
    pub merge_commit: Option<String>,
    #[serde(default)]
    pub merge_method: Option<MergeMethod>,
    #[serde(default)]
    pub next_poll_at: u64,
    #[serde(default)]
    pub unchanged_views: u32,
    #[serde(default)]
    pub last_view_at: Option<u64>,
    #[serde(default)]
    pub watermark: Watermark,
    /// The last view's checks of its head, at most [`CHECKS_MAX`] (decision 41's
    /// `StagePrInfo.checks` and `ci`; not in Interfaces' `PrRecord`, which has nowhere
    /// else to keep them).
    #[serde(default)]
    pub checks: Vec<CheckSeen>,
    #[serde(default)]
    pub retargeted_to: Option<String>,
    #[serde(default)]
    pub branch_deleted: bool,
}

/// One check of a view's head, as the snapshot shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckSeen {
    pub name: String,
    pub state: CiState,
    /// The GitHub Actions run, when the check has one (decision 27).
    #[serde(default)]
    pub ci_run: Option<u64>,
}

/// Decision 23: what a PR's poll has already processed.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Watermark {
    /// The highest `fullDatabaseId` processed per comment class (ruling R-2).
    pub issue_comment: u64,
    pub review: u64,
    pub review_comment: u64,
    /// Head → the failing checks acted on there.
    pub ci: BTreeMap<String, CiSeen>,
    pub mergeable: Option<Mergeable>,
    pub head: String,
}

/// Decision 27: the failing check runs of a head that were acted on, and when.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct CiSeen {
    pub failing: Vec<String>,
    pub at: u64,
}

/// Decision 27's state machine for one red CI run on one head.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiPhase {
    Logs,
    Summarising,
    Rerunning,
    Reproducing,
    Bisecting,
    Tasked,
    ToUser,
}

/// Decision 27: one red CI run of a stage's head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CiRecord {
    pub head: String,
    pub ci_runs: Vec<u64>,
    pub phase: CiPhase,
    #[serde(default)]
    pub log: Option<PathBuf>,
    #[serde(default)]
    pub category: Option<CiCategory>,
    #[serde(default)]
    pub failing_tests: Vec<String>,
    #[serde(default)]
    pub lines: Vec<String>,
    #[serde(default)]
    pub reruns: Vec<u64>,
    /// Decision 27 step 6: the sorted failing tests, or the category.
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub fix_task: Option<String>,
}

/// Decisions 29–31: one review thread, conversation comment or review body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadRecord {
    /// `t<id>`, `c<id>` or `r<id>` (decision 4).
    pub key: String,
    pub author: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub diff_hunk: String,
    /// Cut to `VIEW_TEXT_MAX`; quoted only when used (decision 22).
    #[serde(default)]
    pub text: String,
    pub state: ThreadState,
    pub seen_at: u64,
    #[serde(default)]
    pub last_comment_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadState {
    New,
    Tasked { task: String },
    Replied { comment_id: u64 },
    Ignored { reason: String },
}

/// Decision 31: the new threads of one PR gathered within `review_batch_secs`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Batch {
    pub started_at: u64,
    pub last_at: u64,
    pub threads: Vec<String>,
}

/// Decision 33's `[delivery] sync`, as a run records it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncPolicy {
    #[default]
    OnConflict,
    Always,
}

impl From<config::SyncPolicy> for SyncPolicy {
    fn from(sync: config::SyncPolicy) -> Self {
        match sync {
            config::SyncPolicy::OnConflict => SyncPolicy::OnConflict,
            config::SyncPolicy::Always => SyncPolicy::Always,
        }
    }
}

/// Decision 16: the `[delivery]` keys a run is frozen with at start, so a later config
/// edit never changes a live run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DeliveryLimits {
    pub poll_secs: u64,
    pub poll_max_secs: u64,
    pub ci_log_max_bytes: u64,
    pub ci_fix_max: u32,
    pub review_fix_max: u32,
    pub review_batch_secs: u64,
    pub reviewers: Vec<String>,
    pub reply_to_comments: bool,
    pub sync: SyncPolicy,
    pub delete_merged_branches: bool,
    pub stage_target_lines: (u32, u32),
}

impl From<&config::Delivery> for DeliveryLimits {
    fn from(d: &config::Delivery) -> Self {
        DeliveryLimits {
            poll_secs: d.poll_secs,
            poll_max_secs: d.poll_max_secs,
            ci_log_max_bytes: d.ci_log_max_bytes,
            ci_fix_max: d.ci_fix_max,
            review_fix_max: d.review_fix_max,
            review_batch_secs: d.review_batch_secs,
            reviewers: d.reviewers.clone(),
            reply_to_comments: d.reply_to_comments,
            sync: d.sync.into(),
            delete_merged_branches: d.delete_merged_branches,
            stage_target_lines: d.stage_target_lines,
        }
    }
}

impl Default for DeliveryLimits {
    fn default() -> Self {
        (&config::Delivery::default()).into()
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_body;
#[cfg(test)]
mod tests_snapshot;
