//! Milestone 9.1's snapshot types for tiered testing and stages: where a task came from,
//! a stage's full-suite state, a task's last tier record and its test-weakening signals.
//!
//! Every field a later sender might add is `#[serde(default)]` on the struct that holds
//! these, so an older snapshot still decodes (see `run_info.rs`).

use serde::{Deserialize, Serialize};

/// Who made a task (decision 39). Milestone 9.1 creates `plan`, `bisect` and `sync`
/// tasks; `ci` and `review` are milestone 9.2's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskOrigin {
    #[default]
    Plan,
    Bisect,
    Sync,
    Ci,
    Review,
}

/// A stage's tier 3 (decisions 17-19, 35-38).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FullState {
    #[default]
    None,
    Running,
    Green,
    Red,
    Bisecting,
}

/// A stage's full suite, as shown to a client.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FullInfo {
    pub state: FullState,
    pub at: Option<u64>,
    pub secs: Option<u64>,
    /// The commit the last job ran on.
    pub commit: Option<String>,
    pub shards: u8,
    pub flaky: Vec<String>,
    /// The last red job's failing test names, the first 20.
    pub failing: Vec<String>,
    pub bisect_fixes: u8,
    /// `"no single culprit: …"`, `"still red after 2 fix tasks"`, …
    pub note: Option<String>,
}

/// One stage of a run (decision 55; TT §7 without `pr`, which milestone 9.2 adds).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageInfo {
    pub n: u16,
    pub branch: String,
    /// `None` while the stage branch is not created yet.
    pub head: Option<String>,
    pub tasks: u32,
    pub merged: u32,
    pub full: FullInfo,
    pub fix_tasks: Vec<String>,
    pub propagate_red: Option<String>,
}

/// A task's last tier record (decision 55).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierInfo {
    pub tier: u8,
    /// `"3 modules (a, b, c)"`, `"build only"`, `"full suite (full trigger: Cargo.lock)"`.
    pub affected: String,
    pub steps: u8,
    pub cached: u8,
    pub ok: bool,
    pub secs: u64,
    pub flaky: Vec<String>,
}

/// One test-weakening signal (decision 40); `id` is `W1`, `W2`, …
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalInfo {
    pub id: String,
    /// `"deleted_test_file"`, `"skip_marker"` or `"assertion_loss"`.
    pub kind: String,
    pub path: String,
    pub line: Option<u32>,
    pub text: String,
    /// `"accepted: <reason>"`, `"critical"`, `"engine finding"`, or `None` while
    /// unreviewed.
    pub answered: Option<String>,
}

#[cfg(test)]
#[path = "tiers_tests.rs"]
mod tests;
