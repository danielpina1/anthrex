//! Milestone 9.2's code host (design decisions 5–13): the [`CodeHost`] trait the engine's
//! host ops go through, its types, and [`GhHost`], which runs the user's own `gh` CLI and
//! `git`.
//!
//! Three guards keep anthrex from ever landing anything (decisions 5, 7 and 14): the trait
//! has no method that merges, approves, enables auto-merge, resolves a thread, closes or
//! reopens a pull request, or force-pushes; every command [`GhHost`] builds passes
//! [`allow::check`] before any process starts; and `FakeHost` (task M9.2.5) panics if it
//! is asked to. Every method blocks: call it only on `spawn_blocking`, never under the
//! manager lock or the engine lock (AGENTS.md rules 2 and 10). Every `git` command runs
//! through `worktree::run_git` (rule 11: `--no-optional-locks`, a scrubbed environment, a
//! deadline), and every process starts in [`runner`].

pub mod allow;
pub mod fake;
pub mod gh;
mod gh_git;
pub mod gh_parse;
mod gh_preflight;
pub mod remote;
pub mod runner;
pub mod select;

#[cfg(test)]
pub(crate) mod scripted;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_allow;
#[cfg(test)]
mod tests_allow_git;
#[cfg(test)]
mod tests_git;
#[cfg(test)]
mod tests_limits;
#[cfg(test)]
mod tests_open_logs;
#[cfg(test)]
mod tests_parse;
#[cfg(test)]
mod tests_reply;
#[cfg(test)]
mod tests_seal;

use std::fmt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

pub use proto::PrState;

pub use gh::{GhHost, THREADS_QUERY};
pub use runner::{Capture, Cut, Program, RunOutput, Runner, SystemRunner};

/// Decision 5. Every method blocks: call it only on `spawn_blocking`, never under a lock.
pub trait CodeHost: Send + Sync {
    fn preflight(&self, req: &PreflightReq) -> Result<HostRepo, HostError>;
    /// Decision 3's detection at `profile detect` (task M9.2.12): preflight's first
    /// checks only (the remote's URL, `gh --version`, `gh auth status`, `gh repo view`),
    /// never a push.
    fn detect(&self, root: &Path, remote: &str) -> Result<HostRepo, HostError>;
    fn push(&self, req: &PushReq) -> Result<PushOutcome, HostError>;
    fn fetch(&self, req: &FetchReq) -> Result<FetchOutcome, HostError>;
    fn open_pr(&self, req: &OpenPrReq) -> Result<PrRef, HostError>;
    fn view_pr(&self, repo: &HostRepo, number: u64) -> Result<PrView, HostError>;
    fn failed_logs(
        &self,
        repo: &HostRepo,
        ci_run: u64,
        max_bytes: u64,
        out: &Path,
    ) -> Result<LogFile, HostError>;
    fn rerun_failed(&self, repo: &HostRepo, ci_run: u64) -> Result<(), HostError>;
    /// The comment's id: the existing one when its marker is already there (decision 10).
    fn reply(&self, req: &ReplyReq) -> Result<u64, HostError>;
    fn retarget(&self, repo: &HostRepo, number: u64, base: &str) -> Result<(), HostError>;
    fn permission(&self, repo: &HostRepo, user: &str) -> Result<RepoPermission, HostError>;
    fn delete_branch(&self, req: &DeleteBranchReq) -> Result<(), HostError>;
    /// The controller's ruling (task M9.2.12): a SHA-256 digest of `remote`'s fetch and
    /// push URLs as git resolves them, after `insteadOf` and `pushInsteadOf` (`git remote
    /// get-url --all`, and `--push --all`), read through the allow-list. A digest, never
    /// the URLs: one may carry a token.
    fn remote_seal(&self, root: &Path, remote: &str) -> Result<String, HostError>;
}

/// A GitHub repository as preflight found it (decision 17), frozen into the run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostRepo {
    pub host: String,
    pub owner: String,
    pub name: String,
    pub remote: String,
    pub root: PathBuf,
}

impl HostRepo {
    /// `<owner>/<name>`, as `gh --repo` takes it.
    pub fn full(&self) -> String {
        format!("{}/{}", self.owner, self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightReq {
    pub root: PathBuf,
    pub remote: String,
    pub base_branch: String,
    pub base_sha: String,
    /// Eight hexadecimal characters: the dry run's `anthrex/preflight-<nonce>`.
    pub nonce: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushReq {
    pub repo: HostRepo,
    pub run_id: String,
    pub stage: u16,
    pub sha: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PushOutcome {
    Pushed,
    UpToDate,
    /// `[rejected]`: the remote branch is not an ancestor of the commit (decision 13).
    Rejected {
        reason: String,
    },
    /// Ruling R-11: `[remote rejected]` (branch protection, a ruleset, a hook). `reason`
    /// is the whole text, `the remote refused the push of <branch>: <reason>`.
    Refused {
        reason: String,
    },
}

/// Decision 24: move `local_ref` (a branch name such as `anthrex/<run>/stage-1`) from
/// `expected_local` to the fetched commit, and `anthrex/<run>/integration` with it when
/// `also_integration` (the highest stage of a `Multi` run).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Adopt {
    pub local_ref: String,
    pub expected_local: String,
    pub also_integration: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FetchReq {
    pub repo: HostRepo,
    /// The run whose private ref `into` is (fix round 1, m2: never read from `into`).
    pub run_id: String,
    pub branch: String,
    /// `refs/anthrex/<run>/remote/<name>`: anthrex's own ref, never a branch.
    pub into: String,
    pub adopt: Option<Adopt>,
    /// Ruling R-4 (task M9.2.11): after the fetch, count this merge commit's parents
    /// locally (`git rev-list --parents -n 1 <oid>`), for decision 44's merge method.
    #[serde(default)]
    pub parents_of: Option<String>,
    /// Fix wave A2 (review A, M2): when the op's bound ends, counted from the moment it
    /// took the project's git queue. An adoption checks it just before its
    /// compare-and-swap and answers `TimedOut` rather than move a ref after the op's
    /// answer has gone. `None` outside the executor. Never serialized.
    #[serde(skip)]
    pub deadline: Option<std::time::Instant>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FetchOutcome {
    Fetched {
        sha: String,
        /// Ruling R-4: the parents of `FetchReq.parents_of`, when it was asked and is
        /// known locally after the fetch.
        #[serde(default)]
        parents: Option<u32>,
    },
    Adopted {
        sha: String,
    },
    NotDescendant {
        remote: String,
    },
    /// The local ref was not at `expected_local`; `local` is where it is now (empty when
    /// it no longer exists). Nothing moved.
    LocalMoved {
        local: String,
    },
    /// The remote has no such branch.
    Missing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OpenPrReq {
    pub repo: HostRepo,
    /// The run whose stage branch `head` must be (fix round 1, m2).
    pub run_id: String,
    pub base: String,
    pub head: String,
    pub title: String,
    pub body_file: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrRef {
    pub number: u64,
    pub url: String,
    pub state: PrState,
    pub existed: bool,
    /// Fix wave A1 (review A, M1): for an adopted PR (`existed`), the base branch the
    /// host reports (`pr list`'s `baseRefName`), which can differ from the base asked
    /// for when an earlier create timed out after it succeeded; the engine retargets it.
    /// `None` for a PR this call created: its base is the one asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogFile {
    pub path: PathBuf,
    pub bytes: u64,
    pub truncated: bool,
    /// Task M9.2.9: the file's last `CI_SUMMARY_INPUT_BYTES` as text, for the engine,
    /// which reads no file: the decider's input (decision 18) and the fix task's quote.
    #[serde(default)]
    pub tail: String,
}

/// From the REST permission endpoint's `role_name` (ruling R-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepoPermission {
    Admin,
    Maintain,
    Write,
    Triage,
    Read,
    None,
}

impl RepoPermission {
    /// Decision 29: admin, maintain and write count as write access.
    pub fn writes(self) -> bool {
        matches!(
            self,
            RepoPermission::Admin | RepoPermission::Maintain | RepoPermission::Write
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ReplyTarget {
    /// A review thread, answered in the thread; `comment_id` is its first comment.
    Thread { comment_id: u64 },
    /// The PR's conversation (a `c<id>` comment or an `r<id>` review body).
    Conversation,
}

/// `body` is the reply without its marker; [`GhHost`] posts `<body>\n\n<marker>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplyReq {
    pub repo: HostRepo,
    pub number: u64,
    pub target: ReplyTarget,
    pub body: String,
    pub marker: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeleteBranchReq {
    pub repo: HostRepo,
    pub run_id: String,
    pub stage: u16,
}

/// What one poll saw (decision 23). Reviews, conversation comments and review threads
/// come from the one GraphQL read (ruling R-2), newest first, at most
/// [`VIEW_ITEMS_MAX`] of each class, every body cut to [`VIEW_TEXT_MAX`] characters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PrView {
    pub number: u64,
    pub state: PrState,
    pub merged_at: Option<u64>,
    pub merge_commit: Option<MergeCommit>,
    pub base_ref: String,
    pub head_oid: String,
    pub mergeable: Mergeable,
    pub review_decision: Option<String>,
    pub checks: Vec<CheckRun>,
    pub reviews: Vec<Review>,
    pub comments: Vec<IssueComment>,
    pub threads: Vec<ReviewThread>,
}

/// Ruling R-4: `gh pr view` gives no parent count; the parents are counted locally
/// after the base fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MergeCommit {
    pub oid: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mergeable {
    Mergeable,
    Conflicting,
    Unknown,
}

/// Ruling R-5: a check is pending until it completed with a conclusion anthrex knows;
/// any value it does not know counts as pending, never green.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Completed,
    Pending,
}

/// A check run's conclusion, and a status context's final `state` (`Error`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Conclusion {
    Success,
    Failure,
    TimedOut,
    Cancelled,
    ActionRequired,
    StartupFailure,
    Neutral,
    Skipped,
    Stale,
    Error,
}

impl Conclusion {
    /// Ruling R-5's red set.
    pub fn is_red(self) -> bool {
        matches!(
            self,
            Conclusion::Failure
                | Conclusion::TimedOut
                | Conclusion::Cancelled
                | Conclusion::ActionRequired
                | Conclusion::StartupFailure
                | Conclusion::Error
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRun {
    pub name: String,
    pub status: CheckStatus,
    pub conclusion: Option<Conclusion>,
    /// The GitHub Actions run, from `/actions/runs/<id>` in its URL (decision 27).
    pub ci_run: Option<u64>,
    pub url: String,
}

/// Ruling R-3: `bot` is GraphQL's `__typename == "Bot"`, never read from the login.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub login: String,
    pub bot: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReviewState {
    Approved,
    ChangesRequested,
    Commented,
    Dismissed,
    Pending,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Review {
    pub id: u64,
    pub author: Author,
    pub state: ReviewState,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IssueComment {
    pub id: u64,
    pub author: Author,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewThread {
    pub resolved: bool,
    pub path: Option<String>,
    pub line: Option<u32>,
    pub comments: Vec<ThreadComment>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadComment {
    pub id: u64,
    pub author: Author,
    pub body: String,
    pub diff_hunk: String,
}

/// Decision 11. `Forbidden` is a bug: the allow-list refused a command anthrex built.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostError {
    Missing(String),
    Auth(String),
    NotFound(String),
    RateLimited(String),
    Rejected(String),
    Forbidden(String),
    TimedOut(String),
    Failed(String),
}

impl HostError {
    pub fn text(&self) -> &str {
        match self {
            HostError::Missing(t)
            | HostError::Auth(t)
            | HostError::NotFound(t)
            | HostError::RateLimited(t)
            | HostError::Rejected(t)
            | HostError::Forbidden(t)
            | HostError::TimedOut(t)
            | HostError::Failed(t) => t,
        }
    }
}

impl fmt::Display for HostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.text())
    }
}

impl std::error::Error for HostError {}

/// Decision 9: `gh` reads, `git config`/`ls-remote`/`rev-parse`/`merge-base`.
pub const HOST_READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Decision 9: `gh pr create`, `gh pr edit`, `gh pr comment`, the reply `POST`, `gh run
/// rerun`.
pub const HOST_WRITE_TIMEOUT: Duration = Duration::from_secs(60);
/// Decision 9: `git push` (the dry run too), `git fetch`, a branch delete.
pub const PUSH_TIMEOUT: Duration = Duration::from_secs(120);

/// Preflight's whole bound (task M9.2.12): its six checks' own (decision 9), the seal's
/// two reads, and a margin, so a host that ignores its per-command timeouts still
/// answers. The CLI's `run start` reply bound is derived from it.
pub const PREFLIGHT_BOUND: Duration =
    Duration::from_secs(7 * HOST_READ_TIMEOUT.as_secs() + PUSH_TIMEOUT.as_secs() + 5);
/// Decision 9: `gh run view --log-failed`.
pub const LOG_TIMEOUT: Duration = Duration::from_secs(120);
/// Each body kept from a view is cut to this many characters; a view keeps at most
/// [`VIEW_ITEMS_MAX`] comments of each class, newest first.
pub const VIEW_TEXT_MAX: usize = 8000;
pub const VIEW_ITEMS_MAX: usize = 200;
/// [`gh::THREADS_QUERY`]'s page: the newest this many review threads, reviews and
/// conversation comments (task M9.2.8's fix round, I2: a full page may hide older ones).
pub const VIEW_PAGE: usize = 100;
/// [`gh::THREADS_QUERY`] reads a review thread's first this many comments.
pub const THREAD_PAGE: usize = 50;
