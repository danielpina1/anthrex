//! Milestone 9.1's stage model (decisions 46–53): the layout a run is approved with,
//! one record per created stage, and the records the later tasks of the milestone fill
//! (tier 3, bisect, sync and fix tasks). Pure data (design decision 2); every field a
//! run from milestone 9 lacks is `#[serde(default)]`.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use super::OpId;

/// Decision 46: fixed when the run's plan is first approved. `Single` is M8a's layout
/// exactly (stage 1's branch is `anthrex/<run>/integration`, no `stage-*` ref); `Multi`
/// has `anthrex/<run>/stage-<n>` per stage and `integration` as the alias of the
/// highest created one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageLayout {
    #[default]
    Single,
    Multi,
}

/// One created stage (decision 47).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageRecord {
    pub n: u16,
    pub branch: String,
    pub head: String,
    #[serde(default)]
    pub created_from: String,
    #[serde(default)]
    pub created_at: u64,
    /// The first-parent line the engine wrote, in order.
    #[serde(default)]
    pub merges: Vec<StageMerge>,
    /// Decision 48: the task ids whose merges the stage head contains.
    #[serde(default)]
    pub tasks_in: BTreeSet<String>,
    /// The lower stage's last head this one holds.
    #[serde(default)]
    pub synced_from: Option<String>,
    #[serde(default)]
    pub last_green_candidate: Option<String>,
    #[serde(default)]
    pub full: StageFull,
    #[serde(default)]
    pub bisect: Option<BisectRecord>,
    #[serde(default)]
    pub propagate_red: Option<String>,
}

impl StageRecord {
    /// A stage created on `branch` at `from`, holding what `tasks_in` names.
    pub fn new(n: u16, branch: String, from: &str, tasks_in: BTreeSet<String>, now: u64) -> Self {
        StageRecord {
            n,
            branch,
            head: from.to_string(),
            created_from: from.to_string(),
            created_at: now,
            merges: Vec::new(),
            tasks_in,
            synced_from: None,
            last_green_candidate: None,
            full: StageFull::default(),
            bisect: None,
            propagate_red: None,
        }
    }
}

/// One commit on a stage's first-parent line.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageMerge {
    Task { id: String, commit: String },
    Propagate { from: u16, commit: String },
}

/// A stage's tier 3 (decisions 17–19, 35–38; filled from task M9.1.14 on).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageFull {
    #[serde(default)]
    pub green_at: Option<String>,
    #[serde(default)]
    pub red_at: Option<String>,
    #[serde(default)]
    pub bisect_fixes: u8,
    #[serde(default)]
    pub last: Option<TierRecord>,
    #[serde(default)]
    pub note: Option<String>,
}

/// Decision 36's bisect of a red tier 3 (task M9.1.15).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BisectRecord {
    pub head: String,
    pub tests: Vec<String>,
    pub base: String,
    pub candidates: Vec<StageMerge>,
    pub lo: usize,
    pub hi: usize,
    #[serde(default)]
    pub probe: Option<(OpId, Probe)>,
    #[serde(default)]
    pub probes: u32,
    #[serde(default)]
    pub summary: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Probe {
    Base,
    Head,
    Mid(usize),
}

/// Decision 51: a sync task's merge (task M9.1.17).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    pub onto: String,
    pub base_tree: String,
    pub tasks: BTreeSet<String>,
    #[serde(default)]
    pub handed_back: bool,
}

/// What a fix task fixes (decisions 37, 51). 9.2 appends `Ci`, `Review` and `Base`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FixOf {
    Bisect {
        culprit: String,
        stage: u16,
        tests: Vec<String>,
    },
    Propagate {
        from: u16,
        to: u16,
        head: String,
    },
}

/// A tier job's record (decision 55; task M9.1.13 on).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierRecord {
    pub tier: u8,
    pub affected: String,
    pub steps: u8,
    pub cached: u8,
    pub ok: bool,
    pub secs: u64,
    #[serde(default)]
    pub flaky: Vec<String>,
    #[serde(default)]
    pub failing: Vec<String>,
    pub at: u64,
}
