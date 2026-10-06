//! Decision 8: host operations are engine ops. The reducer decides each host call as
//! `OpKind::Host { repo, op: HostOp }` and the driver answers `OpResult::Host(HostResult)`;
//! both are journaled (AS §17 "Intent log"), so a large answer (a CI log, a PR body)
//! stays out of them: `FailedLogs` answers the log file's path, and `OpenPr`'s body is
//! written to a file by the executor. Pure data (design decision 1).

use serde::{Deserialize, Serialize};

use crate::host::{
    Adopt, Contains, FetchOutcome, HostError, LogFile, PrRef, PrView, PushOutcome, ReplyTarget,
    RepoPermission,
};

/// One host call (Interfaces "host ops").
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostOp {
    /// Decisions 20 and 28: `sha` to `refs/heads/anthrex/<run>/stage-<stage>`.
    Push { stage: u16, sha: String },
    /// Decisions 24 and 33; `stage` is `None` for the base. `parents_of` (task
    /// M9.2.11, ruling R-4): a merged stage PR's merge commit whose parents the fetch
    /// counts (`FetchReq.parents_of`).
    Fetch {
        stage: Option<u16>,
        branch: String,
        into: String,
        adopt: Option<Adopt>,
        #[serde(default)]
        parents_of: Option<String>,
        /// Milestone 9.7 decision 5: the base fetch's question (`FetchReq.contains`).
        #[serde(default)]
        /// Boxed (the final fix wave): it keeps `OpKind` small; the JSON is the same.
        contains: Option<Box<Contains>>,
    },
    /// Decision 20; `body` is [`super::body::pr_body`]'s text.
    OpenPr {
        stage: u16,
        base: String,
        head: String,
        title: String,
        body: String,
    },
    /// Decision 23.
    ViewPr { stage: u16, number: u64 },
    /// Decision 27 step 1.
    FailedLogs {
        stage: u16,
        ci_run: u64,
        max_bytes: u64,
    },
    /// Decision 27 step 3.
    RerunFailed { stage: u16, ci_run: u64 },
    /// Decision 30; `body` is the reply without its marker, which the engine builds
    /// (task M9.2.10: its `<sha7>` is the engine's to know) and the host appends.
    Reply {
        stage: u16,
        number: u64,
        thread: String,
        target: ReplyTarget,
        body: String,
        #[serde(default)]
        marker: String,
    },
    /// Decision 35.
    Retarget {
        stage: u16,
        number: u64,
        base: String,
    },
    /// Decision 29.
    Permission { user: String },
    /// Decision 35's `delete_merged_branches`.
    DeleteBranch { stage: u16 },
}

/// What a host op answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostResult {
    Pushed(PushOutcome),
    Fetched(FetchOutcome),
    PrOpened(PrRef),
    /// Boxed: a view is far larger than every other answer.
    PrViewed(Box<PrView>),
    Logs(LogFile),
    Rerun,
    Replied {
        comment_id: u64,
    },
    Retargeted,
    Permission {
        user: String,
        permission: RepoPermission,
    },
    Deleted,
    Error(HostError),
}
