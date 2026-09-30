//! Milestone 9.1's stage model (decisions 46–53): the layout a run is approved with,
//! one record per created stage, and the records the later tasks of the milestone fill
//! (tier 3, bisect, sync and fix tasks). Pure data (design decision 2); every field a
//! run from milestone 9 lacks is `#[serde(default)]`.

use std::collections::BTreeSet;
use std::path::PathBuf;

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
    /// Decision 52's attention line of that red propagate (task M9.1.17).
    #[serde(default)]
    pub propagate_note: Option<String>,
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
            propagate_note: None,
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
    /// Ruling C-18: the executor's own failures on one commit, retried with backoff.
    #[serde(default)]
    pub infra: Option<InfraFailures>,
    /// Decision 57: how many bisects of this stage ended, each one's `bisect` history
    /// line numbered by it (`<run>/bisect/<stage>/<n>`).
    #[serde(default)]
    pub bisects: u32,
    /// Task M9.1.20: every ended bisect of the stage, for the report's "Testing".
    #[serde(default)]
    pub ended: Vec<BisectEnd>,
}

/// One ended bisect (decisions 36–38), as its `bisect` history line records it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BisectEnd {
    pub head: String,
    /// The candidate merges.
    pub range: u32,
    pub probes: u32,
    pub culprit: Option<String>,
    pub fix_task: Option<String>,
    pub reason: Option<String>,
    pub at: u64,
}

/// Ruling C-18: consecutive `Failed` results of tier 3 on `commit`, the last at `at`,
/// with the first non-empty line of its message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InfraFailures {
    pub commit: String,
    pub count: u8,
    pub at: u64,
    pub line: String,
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
    /// The ≤ 40-line summary of the red step: its fallback at once, the check-summary
    /// decider's answer when it comes ([`summary_decider`](Self::summary_decider)).
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub summary_decider: Option<u64>,
    /// `git show --stat` of the lowest red merge probed so far (the culprit's, at the end).
    #[serde(default)]
    pub show: Option<String>,
    /// Ruling C-18 for probes: the executor's own failures of the probe due now, and
    /// when it may be issued again.
    #[serde(default)]
    pub infra: u8,
    #[serde(default)]
    pub retry_at: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Probe {
    Base,
    Head,
    Mid(usize),
}

/// Decision 51: a sync task's merge (task M9.1.17): `onto` the lower stage's head it
/// merges, `base_tree` the conflicted tree, `tasks` what that head holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncState {
    pub onto: String,
    pub base_tree: String,
    pub tasks: BTreeSet<String>,
    #[serde(default)]
    pub handed_back: bool,
    /// The upper stage's head the conflict was found on: the sync task's worktree
    /// starts there, so the hand-back's merge is exactly `base_tree`.
    #[serde(default)]
    pub to_head: String,
    /// Ruling C-21 (3): the upper stage's heads handed back into the worktree after
    /// the first hand-back, in order.
    #[serde(default)]
    pub handed: Vec<String>,
}

/// Ruling C-21 (3, 5): what a sync task's claim is checked against: its head must
/// contain `onto`, and the paths `to_head..upper` changed (the upper stage's commits a
/// later hand-back brought in) are not its own.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncCheck {
    pub onto: String,
    pub to_head: String,
    #[serde(default)]
    pub upper: Option<String>,
}

/// Decision 50's `OpKind::Propagate`: stage `from`'s head merged into stage `to`, as a
/// merge candidate is (guard, `merge-tree`, `commit-tree` with parents `[to head, from
/// head]`, tier 2 or `check`, guard, compare-and-swap, reattach). `from: 0` is reserved
/// for 9.2's base sync and never emitted by 9.1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PropagateSpec {
    pub root: PathBuf,
    pub integration: PathBuf,
    pub from: u16,
    pub to: u16,
    pub from_head: String,
    pub to_branch: String,
    pub expected_to_head: String,
    pub also_integration: bool,
    pub base_branch: String,
    pub expected_base: String,
    pub guarded: Vec<(String, String)>,
    pub message: String,
    pub tier: Option<Box<crate::run::tiers::TierSpec>>,
    pub check: Option<String>,
    pub timeout_secs: u64,
    pub env: Vec<(String, String)>,
    /// The task ids stage `from` held at `from_head`, captured when the op is emitted.
    pub tasks: BTreeSet<String>,
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
    /// Ruling C-18: the commit the job ran on (tier 3; empty for tiers 1 and 2).
    #[serde(default)]
    pub commit: String,
}
