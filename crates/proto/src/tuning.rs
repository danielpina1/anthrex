//! Milestone 9.5 (tuning): race lanes and the test-writer pair as the snapshot shows
//! them, the tuning file (`<repo_dir>/tuning.toml`, decision 10) and the tuning block of
//! `anthrex run stats` (decision 11).
//!
//! Every field a protocol-15 peer never wrote is `#[serde(default)]` where it is added
//! to an older type, so a 9.3 `run.json`, snapshot and `history.jsonl` still decode.
//! The tuning file's types refuse unknown keys at every level: a misspelt key in a file
//! anthrex writes itself means the file is not what anthrex wrote.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::run::{Budget, Route};

/// One of a racing task's two lanes (decision 19): `"a"` or `"b"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RaceLane {
    A,
    B,
}

impl RaceLane {
    /// `"a"` or `"b"`, as `anthrex mcp --lane` takes it and a lane's checkout ends.
    pub fn label(self) -> &'static str {
        match self {
            RaceLane::A => "a",
            RaceLane::B => "b",
        }
    }

    /// The lane racing this one.
    pub fn other(self) -> RaceLane {
        match self {
            RaceLane::A => RaceLane::B,
            RaceLane::B => RaceLane::A,
        }
    }
}

/// Where one lane of a race is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneState {
    Preparing,
    Working,
    Proof,
    Check,
    Review,
    Won,
    Adopted,
    Lost,
    Out,
}

impl LaneState {
    /// The state's serde name, as `run status` and `REPORT.md` print it (task M9.5.21).
    pub fn label(self) -> &'static str {
        match self {
            LaneState::Preparing => "preparing",
            LaneState::Working => "working",
            LaneState::Proof => "proof",
            LaneState::Check => "check",
            LaneState::Review => "review",
            LaneState::Won => "won",
            LaneState::Adopted => "adopted",
            LaneState::Lost => "lost",
            LaneState::Out => "out",
        }
    }
}

/// One lane of a racing task, as shown to a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LaneInfo {
    pub lane: RaceLane,
    pub route: Route,
    pub state: LaneState,
    /// The lane's checkout name, `<task>.<lane>`.
    pub checkout: String,
    pub head: Option<String>,
    pub reason: Option<String>,
    pub salvage_ref: Option<String>,
    /// The lane's checkout was kept, because its racer did not exit in time (decision
    /// 22); it goes with the run's other checkouts at accept or discard. Task 20b.
    #[serde(default, skip_serializing_if = "is_false")]
    pub kept: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// A racing task's lanes and, once decided, the lane that became the task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RaceInfo {
    pub lanes: Vec<LaneInfo>,
    pub winner: Option<RaceLane>,
    pub adopted: bool,
}

/// Which session of a paired task is working (decision 24).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PairPhase {
    Writing,
    Implementing,
}

/// A paired task's test writer and its red commit, as shown to a client.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PairInfo {
    pub phase: PairPhase,
    pub writer_route: Route,
    pub test: Option<String>,
    pub red: Option<String>,
    pub red_checked: Option<bool>,
    pub writer_failures: u8,
}

/// The opt-in pattern a task ran under, as its history record names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskPattern {
    Race,
    Pair,
}

/// The version every tuning file written now carries.
pub const TUNING_VERSION: u32 = 1;

/// `<repo_dir>/tuning.toml` (decision 10). `[budgets.*]` and `[weights]` are written by
/// the automatic refit, `[thresholds]` by `run stats --apply`, and `[dismissed]` by
/// `run stats --dismiss`. `[routes.*]`, which 9.5's `--apply` wrote, is read and ignored
/// (milestone 9.8 decision 30, ruling F20).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TuningFile {
    pub v: u32,
    /// By class key: `"s"`, `"m"`, `"hub"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub budgets: BTreeMap<String, ClassBudget>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub weights: Option<PathWeights>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thresholds: Option<SizeThresholds>,
    /// Milestone 9.8 decision 30: an old file's `[routes.<class>]` tables load, whatever
    /// they hold, and are never written.
    #[serde(default, skip_serializing)]
    pub routes: Ignored,
    /// Proposal id → the proposed value it was dismissed at.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub dismissed: BTreeMap<String, String>,
}

impl Default for TuningFile {
    fn default() -> Self {
        TuningFile {
            v: TUNING_VERSION,
            budgets: BTreeMap::new(),
            weights: None,
            thresholds: None,
            routes: Ignored,
            dismissed: BTreeMap::new(),
        }
    }
}

/// One class's refitted budget, with how many samples it came from and when.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ClassBudget {
    pub tool_calls: u32,
    pub minutes: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<u64>,
    pub samples: u32,
    pub at: u64,
}

/// Critical-path weights in seconds, per class; `derived` names the classes (`"S"`,
/// `"M"`, `"hub"`) whose weight was derived rather than measured.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PathWeights {
    pub s_secs: u64,
    pub m_secs: u64,
    pub hub_secs: u64,
    #[serde(default)]
    pub derived: Vec<String>,
    pub at: u64,
}

/// The line thresholds of size classes S and M (spec §7.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SizeThresholds {
    pub s_lines: u32,
    pub m_lines: u32,
}

impl Default for SizeThresholds {
    fn default() -> Self {
        SizeThresholds {
            s_lines: 20,
            m_lines: 100,
        }
    }
}

/// A value an older file holds and nothing reads any more (milestone 9.8 decision 30):
/// any value deserialises to it; a field of this type is skipped when serialising.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Ignored;

impl<'de> Deserialize<'de> for Ignored {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        serde::de::IgnoredAny::deserialize(deserializer)?;
        Ok(Ignored)
    }
}

/// One class's line of the tuning block (decision 11).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClassTuning {
    /// `"S"`, `"M"` or `"hub"`.
    pub class: String,
    pub samples: u32,
    pub budget: Budget,
    pub refit: RefitState,
    /// Ruling RH-5: `[orchestrator.budget.<class>]` is set in the configuration.
    pub configured: bool,
    /// The refit, whether it is used or not.
    pub refit_budget: Option<Budget>,
    pub weight_secs: Option<u64>,
    pub weight_derived: bool,
}

/// What became of a class's budget refit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefitState {
    NotYet,
    Kept,
    Written { at: u64 },
    Configured,
    Off,
}

/// The exact change a proposal makes to the tuning file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TuningChange {
    Threshold { class: String, lines: u32 },
}

/// One proposal of `run stats`, applied only on the user's confirmation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuningProposal {
    pub id: String,
    pub text: String,
    pub current: String,
    pub proposed: String,
    pub change: TuningChange,
}

/// A proposal named with a value (whole-branch review C, m-2): in `run stats --apply`,
/// the proposed value the user confirmed, which the daemon refuses to apply once the
/// proposal has changed; in [`TuningReport::dismissed`], the value stored as dismissed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalValue {
    pub id: String,
    pub value: String,
}

/// The tuning block of `run stats` (`HistoryStats.tuning`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TuningReport {
    pub path: PathBuf,
    pub min_samples: u32,
    pub refit_budgets: bool,
    pub classes: Vec<ClassTuning>,
    pub proposals: Vec<TuningProposal>,
    pub moved_bad_file: Option<PathBuf>,
    pub applied: Vec<String>,
    /// Each dismissed proposal with the value stored for it (whole-branch review C, m-2).
    pub dismissed: Vec<ProposalValue>,
    /// Ruling T8-2: why the moved bad file did not parse (decision 10's `(<error>)`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
    /// Task M9.5.11's fix round: the repository the tuning belongs to (its main
    /// checkout, the daemon's repository key), which `run stats --apply` names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<PathBuf>,
}

#[cfg(test)]
#[path = "tuning_tests.rs"]
mod tests;
