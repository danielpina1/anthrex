//! Milestone 9.2's delivery model (design decision 1: `run/delivery/` is pure): what a
//! run freezes at its start (decisions 3 and 16, task M9.2.3), the stages' pull
//! requests, their watermarks, CI records and threads (decisions 19–37, task M9.2.6),
//! the host ops ([`ops`]), the PR title and body ([`body`]), the quoting of untrusted
//! text ([`quote`]) and what the snapshot shows of it all ([`snapshot`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use proto::{CiCategory, CiState, DeliveryMode, MergeMethod, PrState};

use crate::host::{HostRepo, Mergeable, ReplyTarget, RepoPermission};

pub mod body;
pub mod contract;
pub mod digest;
pub mod ops;
pub mod quote;
pub mod reply_edit;
pub mod snapshot;
pub mod templates;
pub mod validate;
pub mod view_trim;

/// Decision 21: a PR body is cut to this many characters (GitHub's limit is 65 536).
pub const BODY_MAX_CHARS: usize = 60_000;
/// Decision 11: failures in a row of one op kind on one stage before the attention line.
pub const FAILURES_BEFORE_ATTENTION: u32 = 5;
/// Decision 27: a stage keeps at most this many CI records, newest last.
pub const CI_RECORDS_MAX: usize = 20;
/// Task M9.2.8's fix round, I1: a PR keeps this many processed review ids.
pub const REVIEWS_SEEN_MAX: usize = 1_000;
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
    /// Task M9.2.8: the run's delivery attention lines by key (`"auth"`, or a failure
    /// key once it reaches [`FAILURES_BEFORE_ATTENTION`]), each removed when what it
    /// reports clears (not in Interfaces' `RunDelivery`).
    pub alerts: BTreeMap<String, String>,
    /// Task M9.2.10 (decision 29): a failed `Permission` op is not asked again before
    /// this time (decision 11's "retry when next due"; not in Interfaces).
    pub permission_retry_at: Option<u64>,
    /// Task M9.2.11 (decision 33): a fetch of the remote base branch is due (a stage
    /// merged, the lowest open PR conflicts, or `sync = "always"` polled it), and a
    /// failed one is not retried before `base_fetch_retry_at` (not in Interfaces).
    pub base_fetch_due: bool,
    pub base_fetch_retry_at: Option<u64>,
    /// Task M9.2.12 (the controller's ruling): a digest of the remote's fetch and push
    /// URLs as preflight found them (`git remote get-url --all`, and `--push`, so
    /// `insteadOf` rewriting counts too). A push or fetch is refused when they changed,
    /// so a worker's `git config` in the shared `.git/config` never redirects one. A
    /// digest, not the URLs: one may carry a token (not in Interfaces).
    pub remote_seal: Option<String>,
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

    /// Decision 29: whether `login` may write: listed in `[delivery] reviewers`, or
    /// GitHub's answer, cached; `None` while unknown. A login the allow-list would
    /// refuse to ask about is not a writer (the task M9.2.4 review: never a halt). The
    /// one rule for the review intake and for what the digest quotes (M9.2.13).
    pub fn writes(&self, login: &str) -> Option<bool> {
        let listed = &self.limits.reviewers;
        if listed.iter().any(|r| r.eq_ignore_ascii_case(login)) {
            return Some(true);
        }
        if !crate::host::remote::owner_ok(login) {
            return Some(false);
        }
        let known = self.permissions.get(&login.to_ascii_lowercase());
        known.map(|p| p.writes())
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
    /// Task M9.2.7: the head the opening push sent, until the stage's PR is recorded
    /// with it as `pushed_head` (not in Interfaces' `StageDelivery`).
    pub pushed: Option<String>,
    /// Task M9.2.7: a failed opening op is not retried before this time (decision 11's
    /// "retry when next due").
    pub retry_at: Option<u64>,
    /// Task M9.2.8 (decision 24): the head a view saw on the remote stage branch that is
    /// not the pushed head; an adoption is due until it is fetched.
    pub remote_head: Option<String>,
    /// Task M9.2.8 (ruling R-11): the remote refused this stage's push; the stage pushes
    /// nothing until `run resume` (the reason, as the host gave it).
    pub held: Option<String>,
    /// Task M9.2.10 (decision 30): the replies due on the stage's PR, oldest first.
    pub replies: Vec<ReplyDue>,
    /// The automatic replies already queued, `"<task>/<thread key>"`, so a restart or
    /// a later pass never queues one twice.
    pub auto_replies: BTreeSet<String>,
    /// The ids of the comments anthrex posted on the stage's PR: anthrex's own, known
    /// by id (never by their text, which anyone can paste), never review input.
    pub own_comments: BTreeSet<u64>,
    /// Decision 31: the batches closed so far (each thread keeps its batch's number),
    /// and the last one counted as a review round.
    pub batches: u32,
    pub round_batch: u32,
    /// Task M9.2.11 (decisions 35 and 37): the PR state whose landing was processed
    /// (`merged` or `closed`), so a restart never processes it twice; `None` while open.
    pub landed: Option<PrState>,
    /// Task M9.2.11 (decision 33): a base sync red on `(base sha, stage head)`; it is
    /// not tried again until either moves.
    pub sync_red: Option<(String, String)>,
    /// Task M9.2.11 (decision 43): since when the open PR has been waiting on a person,
    /// as of the last pass; `None` while anthrex has work on it.
    pub wait_from: Option<u64>,
}

/// Decision 30: one reply due on a stage PR's thread (task M9.2.10; not in
/// Interfaces). An automatic reply waits until a push carrying its fix task's merge
/// has landed on the remote (`push`, then `ready`); a `reply_comment` is ready at once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyDue {
    /// The thread's key (`t<id>`, `c<id>`, `r<id>`).
    pub thread: String,
    pub target: ReplyTarget,
    /// The reply without its marker (the host appends it).
    pub body: String,
    /// `<!-- anthrex:reply <run> <pr>:<key> <sha7> -->`: the host finds a reply it
    /// already posted by it (decision 10).
    pub marker: String,
    /// The fix task it reports; `None` for a `reply_comment`.
    #[serde(default)]
    pub task: Option<String>,
    /// The head of the push that carries the fix task's merge.
    #[serde(default)]
    pub push: Option<String>,
    /// The final fix wave's I-1: that push was answered, so the remote branch holds it;
    /// the reply is `ready` once a view of the still-open PR shows the pushed head (a
    /// push answered after the user's merge never reached the merged PR).
    #[serde(default)]
    pub push_done: bool,
    #[serde(default)]
    pub ready: bool,
    /// Sent at least once, unanswered: a comment with its marker may exist.
    #[serde(default)]
    pub sent: bool,
    /// The fix round's m3: failures in a row; at `FAILURES_BEFORE_ATTENTION` the reply
    /// is dropped with an attention line, so it never blocks the stage's next one.
    #[serde(default)]
    pub failures: u32,
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
    /// Task M9.2.11 (decision 35): the base the PR was opened against; `base` is the
    /// host's latest, which proves nothing about what anthrex did (not in Interfaces).
    #[serde(default)]
    pub opened_base: Option<String>,
    #[serde(default)]
    pub branch_deleted: bool,
    /// The final fix wave's I-1: the newest head the open PR was seen to carry (the head
    /// it opened with, then each view of the open PR showing the pushed head). A local
    /// head that is this one, or the head the host reports at the merge, was delivered.
    #[serde(default)]
    pub confirmed: Option<String>,
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
    /// The highest `fullDatabaseId` seen per comment class (ruling R-2). A
    /// conversation comment is new above `issue_comment` (its id is given when it is
    /// posted, which is when it shows); reviews and review-thread comments are new by
    /// key (task M9.2.8's fix round, I1: GitHub gives a review and its comments their
    /// ids when the pending review is started, not when it is submitted), so for them
    /// these are only what the page check reads.
    pub issue_comment: u64,
    pub review: u64,
    pub review_comment: u64,
    /// I1: the reviews processed, by id, the newest [`REVIEWS_SEEN_MAX`].
    pub reviews_seen: std::collections::BTreeSet<u64>,
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
    /// Task M9.2.9 (the controller's ruling): the red checks' Actions run and job ids
    /// (`<run>/<job>`; a check without a job, its name), sorted. A re-run gives its jobs
    /// new ids, so a second red on the same head is seen; names alone would hide it.
    pub jobs: Vec<String>,
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
    /// Task M9.2.9's working state (not in Interfaces' `CiRecord`), each
    /// `#[serde(default)]`: the red checks' names, the job keys of the red (`CiSeen`),
    /// the lines of checks with no Actions run (decision 27 step 1), whether every red
    /// conclusion was a cancellation or a startup failure (step 2).
    #[serde(default)]
    pub checks: Vec<String>,
    #[serde(default)]
    pub jobs: Vec<String>,
    #[serde(default)]
    pub external: Vec<String>,
    #[serde(default)]
    pub infra_only: bool,
    /// The runs whose failed logs came back, and their last 48 KiB as one text (the
    /// engine reads no file); the text is dropped once the record is tasked or handed
    /// to the user, whose brief or line keeps what it needs.
    #[serde(default)]
    pub fetched: Vec<u64>,
    #[serde(default)]
    pub text: String,
    /// The `ci_summary` decider asked, and where its summary came from (`decider`, or
    /// `fallback (<reason>)`).
    #[serde(default)]
    pub decider: Option<u64>,
    #[serde(default)]
    pub source: Option<String>,
    /// The re-runs GitHub answered (`reruns` holds each from the step that issued it, so
    /// a restart never issues one twice).
    #[serde(default)]
    pub reruns_answered: Vec<u64>,
    /// The re-runs that timed out once (issued again once, never a third time), and the
    /// failures in a row of the failed log being fetched (fix round 1).
    #[serde(default)]
    pub rerun_timeouts: Vec<u64>,
    #[serde(default)]
    pub log_failures: u32,
    /// The local reproduction in flight (it holds `Run.full_op`), its executor failures,
    /// when it may be tried again, and the command that reproduced the red.
    #[serde(default)]
    pub probe: Option<u64>,
    #[serde(default)]
    pub probe_failures: u8,
    #[serde(default)]
    pub retry_at: u64,
    #[serde(default)]
    pub command: Option<String>,
}

impl CiRecord {
    /// A new record of a red run on `head`, fetching its logs first.
    pub fn new(head: &str) -> CiRecord {
        CiRecord {
            head: head.to_string(),
            ci_runs: Vec::new(),
            phase: CiPhase::Logs,
            log: None,
            category: None,
            failing_tests: Vec::new(),
            lines: Vec::new(),
            reruns: Vec::new(),
            key: String::new(),
            fix_task: None,
            checks: Vec::new(),
            jobs: Vec::new(),
            external: Vec::new(),
            infra_only: false,
            fetched: Vec::new(),
            text: String::new(),
            decider: None,
            source: None,
            reruns_answered: Vec::new(),
            rerun_timeouts: Vec::new(),
            log_failures: 0,
            probe: None,
            probe_failures: 0,
            retry_at: 0,
            command: None,
        }
    }
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
    /// A review thread's processed comments, in order (task M9.2.8's fix round, I1 and
    /// m5): a comment is new when its id is not here, and each is kept for M9.2.10.
    /// Empty for a `c<id>` or `r<id>` record, whose one text is `text`.
    #[serde(default)]
    pub comments: Vec<SeenComment>,
    /// Task M9.2.10 (decision 29): the logins whose write access decides whether the
    /// thread's fresh comments count (not bots, not anthrex's own), until it does.
    #[serde(default)]
    pub candidates: Vec<String>,
    /// The thread counts: a writer's or a listed reviewer's, it joined its stage's
    /// batch (decision 31).
    #[serde(default)]
    pub counted: bool,
    /// The closed batch (1, 2, …) the thread was handed out in; 0 before.
    #[serde(default)]
    pub batch: u32,
    /// The fix round's m4: when the first of `candidates` was seen; past
    /// `PERMISSION_WAIT_SECS` an unanswered login does not hold the stage's batch.
    #[serde(default)]
    pub waiting_since: u64,
    /// The fix round's m3: replies queued on the thread since it last counted, at most
    /// `reply_edit::REPLIES_PER_THREAD` of them from `reply_comment`.
    #[serde(default)]
    pub replies: u32,
}

/// One processed comment of a review thread (host text, quoted only when used).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeenComment {
    pub id: u64,
    pub author: String,
    #[serde(default)]
    pub text: String,
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
#[cfg(test)]
mod tests_view_trim;
