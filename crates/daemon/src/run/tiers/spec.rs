//! What a tier job and a bisect probe carry, and what they return (Interfaces
//! "daemon", decisions 14, 16, 18, 30–33 and 36): the payloads of `OpKind::Tier`,
//! `OpKind::TestAt` and `OpResult::Tier`. Pure data.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use super::{Affected, CacheCtx, Scope, StepKind, TierProfile};
use crate::run::engine::ScratchAt;
use crate::run::slots::Priority;

/// One tier job (decisions 14, 16, 18); boxed in `OpKind::Tier` (and, from task
/// M9.1.13, in `MergeCandidate.tier` and `Propagate.tier`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierSpec {
    /// 1, 2 or 3.
    pub tier: u8,
    pub stage: u16,
    /// The user's repository, where the diff and the tree are read.
    pub root: PathBuf,
    /// The checkout the steps run in: `<task>.proof` (tier 1), the integration worktree
    /// (tier 2), `.full` (tier 3).
    pub dir: PathBuf,
    /// Tiers 1 and 3: prepare the scratch checkout at the commit first (M8a's check).
    pub scratch: Option<ScratchAt>,
    /// The stage head the change is measured from (decision 15). The executor reads
    /// `<diff_base>...<head>` for tier 1 and `<diff_base> <head>` for tier 2; tier 3
    /// reads no diff.
    pub diff_base: String,
    /// The commit judged: the task's claimed head, the candidate, or the stage head.
    pub head: String,
    pub profile: TierProfile,
    pub check: Option<String>,
    pub hub: Vec<String>,
    pub source: Vec<String>,
    pub modules: Vec<String>,
    /// The profile's `manifests`, which key a command graph's cache (M9.1.7 ruling C-8
    /// (5)): never `[]` in place of the profile's own value.
    #[serde(default)]
    pub manifests: Vec<String>,
    pub single_test: Option<String>,
    /// Each step's bound (M8a's `check_timeout_secs`).
    pub timeout_secs: u64,
    /// The profile's `env`, `{worktree}` expanded.
    pub env: Vec<(String, String)>,
    pub priority: Priority,
    pub critical: bool,
    /// `None` for an untiered profile or an `"unknown"` toolchain (task M9.1.10).
    pub cache: Option<CacheCtx>,
    /// `None`: run `toolchain_id` first (decision 11).
    pub toolchain: Option<String>,
    /// The repository's data directory: the graph and cache files.
    pub repo_dir: PathBuf,
}

/// One bisect probe (decision 36): `materialize` at `commit` in `dir`, then each of
/// `commands` (`single_test` per failing name), each retried once.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TestAtSpec {
    pub root: PathBuf,
    pub dir: PathBuf,
    pub commit: String,
    pub setup: Option<String>,
    pub commands: Vec<String>,
    pub timeout_secs: u64,
    pub env: Vec<(String, String)>,
}

/// One step of a tier job, as it ran.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StepOutcome {
    pub kind: StepKind,
    pub command: String,
    pub ok: bool,
    pub code: Option<i32>,
    pub timed_out: bool,
    pub secs: u64,
    /// A result-cache hit (task M9.1.10): not run.
    pub cached: bool,
    /// Decision 33's retry ran.
    pub retried: bool,
    /// The failing tests' names (decision 32) of a red step.
    pub failing: Vec<String>,
    /// The tests that failed first and passed on the retry (decision 33, ruling C-7).
    pub flaky: Vec<String>,
    /// The slots the step's first run was granted.
    pub granted: u32,
}

/// What a tier job did (`OpResult::Tier`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierOutcome {
    pub tier: u8,
    pub scope: Scope,
    pub affected: Affected,
    /// The judged commit's tree (decision 30's key).
    pub tree: String,
    pub steps: Vec<StepOutcome>,
    pub ok: bool,
    pub secs: u64,
    /// The red step's last 200 lines (M8a's tail), or `""`.
    pub tail: String,
    /// Set when this job ran `toolchain_id` (decision 11).
    pub toolchain: Option<String>,
    /// Decision 9's unknown-graph reason, when this job met one.
    pub graph_note: Option<String>,
}

/// A claim's test-weakening signals (decision 40): at most `SIGNALS_MAX`, deleted test
/// files first, and how many more there were. Boxed in `OpResult::DoneChecked`, which
/// would otherwise make every engine event larger (clippy's `large_enum_variant`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ClaimSignals {
    pub list: Vec<super::Signal>,
    pub more: u32,
    /// The commit the diff was read from (the merge base of the op's `run_head` and
    /// the head): what decision 41's restore command checks out (ruling C-20).
    #[serde(default)]
    pub base: String,
}

/// What `OpKind::VerifyDone` needs to read decision 40's test-weakening signals: the
/// profile's `test_paths` and `skip_markers`. `None` on the op for an untiered profile
/// or one with neither key set (decision 6), and then no `-U0` diff is read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SignalsSpec {
    pub test_paths: Vec<String>,
    pub skip_markers: Vec<String>,
    /// Milestone 9.5 rulings RP-2 and T16-1: a paired task's implementer's red commit.
    /// Its signals are read from red, or from the newest merge after red once one has
    /// landed, and the paths red's own commits touched are read from red as well
    /// (`git::pair_signals`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub red: Option<String>,
}
