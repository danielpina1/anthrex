//! The repository profile (milestone 8b decisions 4 to 11): the stored type, its
//! verification record, the one pending proposal, and what `anthrex profile status`
//! shows.
//!
//! The profile lives only in anthrex's data directory, never in the repository. It has
//! no confinement key: `cache_dirs` and the `confined_*` tables stay in the user's own
//! config, and `deny_unknown_fields` makes a `profile.toml` that names one fail to parse,
//! so a model-written profile cannot widen what a confined command may reach.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::run::ProfileSpec;
use crate::scout::ScoutInfo;

/// Which test output the `PreToolUse` filter hook keeps (decision 28).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputFilter {
    #[default]
    FailuresOnly,
    Tail,
    None,
}

/// How a module directory is named in tier commands (milestone 9.1 decision 10): its
/// Cargo package name, or its directory's last component.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModuleNames {
    Cargo,
    Dir,
}

/// A repository's confirmed profile, stored as `<repo_dir>/profile.toml`: spec §6's keys
/// plus `sample_test`, `check_timeout_secs`, `manifests` and `filter_prefixes`
/// (decision 5).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoProfile {
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub hub: Vec<String>,
    #[serde(default)]
    pub source: Vec<String>,
    #[serde(default)]
    pub generated: Vec<String>,
    /// Extra entries only: M8a's `BUILTIN_PROTECTED` always applies, and an empty list
    /// never disables it.
    #[serde(default)]
    pub protected: Vec<String>,
    #[serde(default)]
    pub setup: Option<String>,
    #[serde(default)]
    pub check: Option<String>,
    #[serde(default)]
    pub check_timeout_secs: Option<u64>,
    #[serde(default)]
    pub single_test: Option<String>,
    #[serde(default)]
    pub test_passed: Option<String>,
    #[serde(default)]
    pub sample_test: Option<String>,
    #[serde(default)]
    pub output_filter: OutputFilter,
    #[serde(default)]
    pub filter_prefixes: Vec<String>,
    #[serde(default)]
    pub conventions: Vec<String>,
    #[serde(default)]
    pub manifests: Vec<String>,
    // Milestone 9.1 decision 5: the tier keys, every one off when absent.
    #[serde(default)]
    pub build_check: Option<String>,
    #[serde(default)]
    pub module_test: Option<String>,
    #[serde(default)]
    pub module_tests: Option<String>,
    #[serde(default)]
    pub module_graph: Option<String>,
    #[serde(default)]
    pub module_names: Option<ModuleNames>,
    #[serde(default)]
    pub full_triggers: Vec<String>,
    #[serde(default)]
    pub slow_tests: Option<String>,
    #[serde(default)]
    pub timing_tests: Option<String>,
    #[serde(default)]
    pub skip_markers: Vec<String>,
    #[serde(default)]
    pub test_paths: Vec<String>,
    #[serde(default)]
    pub full_shards: Option<u8>,
    #[serde(default)]
    pub toolchain_id: Option<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

impl RepoProfile {
    /// M8a's `ProfileSpec` for this profile (decision 5): a list or `env` becomes `Some`
    /// only when it has entries, so an empty one leaves the key unset.
    pub fn spec(&self) -> ProfileSpec {
        let list = |v: &Vec<String>| (!v.is_empty()).then(|| v.clone());
        ProfileSpec {
            modules: list(&self.modules),
            hub: list(&self.hub),
            source: list(&self.source),
            check: self.check.clone(),
            check_timeout_secs: self.check_timeout_secs,
            single_test: self.single_test.clone(),
            test_passed: self.test_passed.clone(),
            setup: self.setup.clone(),
            generated: list(&self.generated),
            protected: list(&self.protected),
            build_check: self.build_check.clone(),
            module_test: self.module_test.clone(),
            module_tests: self.module_tests.clone(),
            module_graph: self.module_graph.clone(),
            module_names: self.module_names,
            full_triggers: list(&self.full_triggers),
            slow_tests: self.slow_tests.clone(),
            timing_tests: self.timing_tests.clone(),
            skip_markers: list(&self.skip_markers),
            test_paths: list(&self.test_paths),
            full_shards: self.full_shards,
            toolchain_id: self.toolchain_id.clone(),
            env: (!self.env.is_empty()).then(|| self.env.clone()),
        }
    }
}

/// Where a run's profile came from (decision 6). `None` is degraded mode (spec §6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfileSource {
    Stored,
    Plan,
    None,
}

/// One proposed command, run by the engine in a scratch checkout (decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandCheck {
    pub command: String,
    pub ok: bool,
    pub code: Option<i32>,
    pub timed_out: bool,
    pub secs: u64,
    /// The last 40 lines of its output.
    pub tail: String,
}

/// What verifying a proposal ran, and whether it ran confined.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileVerification {
    pub at: u64,
    pub confined: bool,
    pub setup: Option<CommandCheck>,
    pub check: Option<CommandCheck>,
    pub single_test: Option<CommandCheck>,
    // Milestone 9.1 decision 12: the tier commands, verified after M8b's three.
    #[serde(default)]
    pub build_check: Option<CommandCheck>,
    #[serde(default)]
    pub module_graph: Option<CommandCheck>,
    #[serde(default)]
    pub module_test: Option<CommandCheck>,
    #[serde(default)]
    pub module_tests: Option<CommandCheck>,
    #[serde(default)]
    pub toolchain_id: Option<CommandCheck>,
}

/// A proposed command that did not pass, and so is not proposed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DroppedCommand {
    pub key: String,
    pub command: String,
    pub reason: String,
    pub tail: String,
}

/// Decision 8's proposal states.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ProposalState {
    Preparing,
    Scouting,
    Verifying,
    Ready,
    Failed { reason: String },
}

/// What started a proposal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "origin", rename_all = "snake_case")]
pub enum ProposalOrigin {
    Detect,
    Auto { stale: Vec<String> },
    Goal,
    Edit { keys: Vec<String> },
}

/// The one pending proposal of a repository, `<repo_dir>/proposal.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProposalRecord {
    pub project: PathBuf,
    pub state: ProposalState,
    pub origin: ProposalOrigin,
    pub started_at: u64,
    pub updated_at: u64,
    /// The commit the scout read and the commands ran at.
    pub base_sha: String,
    pub scout_id: Option<String>,
    pub window_id: Option<u32>,
    /// After verification: only the commands that passed.
    pub profile: Option<RepoProfile>,
    pub verification: Option<ProfileVerification>,
    pub dropped: Vec<DroppedCommand>,
    /// The scout's raw proposal, or the edited profile.
    pub proposed: Option<RepoProfile>,
    /// Decision 12, when `--trust-project` was given.
    pub trusted_project: Vec<String>,
    /// Decision 9, when `--unconfined-checks` was given.
    pub unconfined_checks: bool,
    /// `profile edit --yes`.
    pub auto_confirm: bool,
}

/// `<repo_dir>/profile.meta.json`: when and from what the stored profile was confirmed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileMeta {
    pub confirmed_at: u64,
    /// The scout report id, or `None` after an edit-only change.
    pub report: Option<String>,
    pub verification: Option<ProfileVerification>,
    /// Path → `"<16 hex>:<len>"`, or `"missing"` (decision 7).
    pub fingerprint: BTreeMap<String, String>,
    pub edited_keys: Vec<String>,
    /// The project (main checkout) the profile was confirmed for, so the daemon can
    /// check its staleness at start (decision 7). Absent in a meta written before
    /// M8b.11's review: such a profile is checked at the next `profile status`.
    #[serde(default)]
    pub project: Option<PathBuf>,
}

/// `anthrex profile status`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileStatus {
    pub project: PathBuf,
    pub repo_dir: PathBuf,
    /// `Stored` or `None`.
    pub source: ProfileSource,
    pub confirmed_at: Option<u64>,
    pub stale: Vec<String>,
    pub unparseable: Option<String>,
    pub proposal: Option<ProposalRecord>,
    pub scout: Option<ScoutInfo>,
    /// Whether verification would run confined here (decision 9).
    pub verify_confined: bool,
}
