//! Milestone 9.2: stacked-PR delivery's wire types (design decisions 3, 36, 41 and 44).
//! A run is delivered `local`ly (9.1's behaviour) or as a stack of pull requests, one
//! per stage; these are the profile's `[delivery]` table and what the snapshot shows of
//! the stack. Every new field elsewhere is `#[serde(default)]`, so a milestone-9.1
//! `run.json`, snapshot and `profile.toml` still load.

use serde::{Deserialize, Serialize};

/// How a run reaches the base branch (decision 3): `local` (`run accept`, 9.1) or `pr`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    #[default]
    Local,
    Pr,
}

fn origin() -> String {
    "origin".to_string()
}

/// `profile.toml`'s `[delivery]` table (decision 3). Absent means `local`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeliveryProfile {
    pub mode: DeliveryMode,
    #[serde(default = "origin")]
    pub remote: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrState {
    Open,
    Merged,
    Closed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiState {
    None,
    Pending,
    Green,
    Red,
}

/// The `ci_summary` decider's classification of a red CI run (decision 18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CiCategory {
    Test,
    Build,
    Lint,
    Infra,
    Unknown,
}

/// One check of a stage PR's most recent head.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckRunInfo {
    pub name: String,
    pub state: CiState,
    pub fix_task: Option<String>,
}

/// A stage PR's threads by state (decision 29): counts only, never comment text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadCounts {
    pub new: u32,
    pub tasked: u32,
    pub replied: u32,
    pub ignored: u32,
}

/// One stage's pull request, in the snapshot (decision 41).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagePrInfo {
    pub number: u64,
    pub url: String,
    pub state: PrState,
    pub base: String,
    pub opened_at: u64,
    /// The pushed head.
    pub head: String,
    pub ci: CiState,
    /// The most recent head's checks, at most 20.
    pub checks: Vec<CheckRunInfo>,
    pub threads: ThreadCounts,
    /// `"<id> <origin> <state label>"`, in flight first.
    pub fix_tasks: Vec<String>,
    /// Decision 37: a lower stage's PR was closed without merging.
    pub paused: bool,
    /// Decision 43.
    pub human_review_secs: u64,
    pub merged_at: Option<u64>,
    pub merge_commit: Option<String>,
}

/// The run's delivery, in the snapshot; `None` in local mode (decision 41).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryInfo {
    pub mode: DeliveryMode,
    pub remote: String,
    /// `"<owner>/<name>"`.
    pub repo: String,
    pub watching: bool,
    /// Decision 36: every stage has a PR (open or merged) or was skipped, and one is open.
    pub delivering: bool,
    /// The current poll interval base, after rate limits (decision 11).
    pub poll_secs: u64,
    /// Decision 19's empty stages.
    pub skipped_stages: Vec<u16>,
    /// Ruling R-13 (task M9.2.15's fix round): what the user must act on for this
    /// delivery, typed, so the client raises its alerts without reading attention text.
    /// Left out when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub alerts: Vec<DeliveryAlert>,
}

/// What a delivery alert is about (ruling R-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryAlertKind {
    /// Decision 27 step 6: a red CI the engine gave up fixing.
    CiHandedToUser,
    /// Decision 37: a stage PR closed without merging.
    PrClosedUnmerged,
    /// Decision 11: `gh` lost its login.
    GhLoggedOut,
    /// Decision 11 and ruling R-11: a host op that keeps failing, or a held stage.
    HostOpHeld,
    /// Decision 31: a review round over `review_fix_max`.
    ReviewRoundsOverCap,
}

/// One delivery alert: its kind, its stage (`None`: the whole run) and the attention
/// line the daemon wrote for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeliveryAlert {
    pub kind: DeliveryAlertKind,
    #[serde(default)]
    pub stage: Option<u16>,
    pub text: String,
}

/// How a stage PR ended, in its `stage` history line (decision 44).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageOutcome {
    Merged,
    Closed,
    OpenAtCancel,
}

/// How the user merged a stage PR (decision 44, ruling R-4): two or more parents is a
/// merge commit; one is a squash or a rebase, which the commit cannot tell apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeMethod {
    Merge,
    SquashOrRebase,
    None,
}

#[cfg(test)]
#[path = "delivery_alert_tests.rs"]
mod alert_tests;

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
