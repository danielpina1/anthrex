//! Milestone 9.1's tiered testing, the pure half (decision 1): the tier keys of a
//! run's profile, their validation (decision 8), placeholders (decision 7), module
//! graphs (decision 9), the affected set (decision 22), a job's steps (decision 21),
//! failing-test names (decision 32), test-weakening signals (decision 40) and the
//! result cache's key (decision 30). No file system, process, thread or async runtime
//! here (task M9.1.6's grep): the driver and profile verification run the commands and
//! hand the output in.

use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

use proto::{ModuleNames, ProfileSpec, RepoProfile};
use serde::{Deserialize, Serialize};

use super::globs::validate_glob;
use command::{Piece, pieces};

pub mod affected;
pub mod cache_key;
pub mod command;
pub mod failing;
pub mod graph;
pub mod spec;
pub mod steps;
pub mod weakening;

pub use spec::{StepOutcome, TestAtSpec, TierOutcome, TierSpec};

#[cfg(test)]
mod affected_tests;
#[cfg(test)]
mod cache_key_tests;
#[cfg(test)]
mod command_tests;
#[cfg(test)]
mod failing_tests;
#[cfg(test)]
mod graph_tests;
#[cfg(test)]
mod steps_tests;
#[cfg(test)]
mod validate_tests;
#[cfg(test)]
mod weakening_tests;

/// How long a graph command (or `cargo metadata`) may run (decision 9).
pub const GRAPH_TIMEOUT: Duration = Duration::from_secs(60);
/// How long `toolchain_id` may run (decision 11).
pub const TOOLCHAIN_TIMEOUT: Duration = Duration::from_secs(30);
/// The most shards `check` may be split into.
pub const FULL_SHARDS_MAX: u8 = 16;
/// The longest command or filter expression a tier key may hold (decision 8).
pub const COMMAND_CHARS_MAX: usize = 2000;
/// At most this many `skip_markers`, each 1 to [`SKIP_MARKER_CHARS_MAX`] characters.
pub const SKIP_MARKERS_MAX: usize = 32;
pub const SKIP_MARKER_CHARS_MAX: usize = 64;
/// At most this many failing-test names are read from a red step (decision 32).
pub const FAILING_NAMES_MAX: usize = 50;
/// At most this many names are retried one by one with `single_test` (decision 33).
pub const RETRY_NAMES_MAX: usize = 10;
/// At most this many test-weakening signals per claim (decision 40).
pub const SIGNALS_MAX: usize = 20;
/// The highest stage a plan may name (decision 44).
pub const STAGE_MAX: u16 = 32;

/// Where the module graph comes from (decision 9). `"none"`, and an absent key, are
/// [`GraphSource::None`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphSource {
    #[default]
    None,
    Cargo,
    Command(String),
}

impl GraphSource {
    /// The key's value as the profile spells it.
    pub fn from_key(value: Option<&str>) -> GraphSource {
        match value {
            None | Some("none") => GraphSource::None,
            Some("cargo") => GraphSource::Cargo,
            Some(command) => GraphSource::Command(command.to_string()),
        }
    }
}

/// The run's tier keys (decision 5), `Profile.tiers`. Every key is off when absent.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TierProfile {
    pub build_check: Option<String>,
    pub module_test: Option<String>,
    pub module_tests: Option<String>,
    pub module_graph: GraphSource,
    /// Resolved: `cargo` when `module_graph = "cargo"` and the key is absent, else `dir`.
    pub module_names: ModuleNames,
    pub full_triggers: Vec<String>,
    pub slow_tests: Option<String>,
    pub timing_tests: Option<String>,
    pub skip_markers: Vec<String>,
    pub test_paths: Vec<String>,
    pub full_shards: u8,
    pub toolchain_id: Option<String>,
}

impl Default for TierProfile {
    fn default() -> Self {
        TierProfile {
            build_check: None,
            module_test: None,
            module_tests: None,
            module_graph: GraphSource::None,
            module_names: ModuleNames::Dir,
            full_triggers: Vec::new(),
            slow_tests: None,
            timing_tests: None,
            skip_markers: Vec::new(),
            test_paths: Vec::new(),
            full_shards: 1,
            toolchain_id: None,
        }
    }
}

fn non_blank(value: Option<String>) -> Option<String> {
    value.filter(|s| !s.trim().is_empty())
}

impl TierProfile {
    /// Decision 6: any key set. A key set to what its absence means (`module_graph =
    /// "none"`, `module_names` equal to its default, `full_shards = 1`) switches
    /// nothing on, so it leaves the profile untiered.
    pub fn is_tiered(&self) -> bool {
        *self != TierProfile::default()
    }

    /// Decision 5 for a run: each key from the plan, else from `[orchestrator.profile]`,
    /// else absent; a present-but-blank string resolves to absent, so a plan can switch
    /// a configured key off.
    pub fn resolve(plan: &ProfileSpec, config: &ProfileSpec) -> TierProfile {
        fn pick<T: Clone>(plan: &Option<T>, config: &Option<T>) -> Option<T> {
            plan.clone().or_else(|| config.clone())
        }
        let text = |plan: &Option<String>, config: &Option<String>| non_blank(pick(plan, config));
        TierProfile::build(ProfileSpec {
            build_check: text(&plan.build_check, &config.build_check),
            module_test: text(&plan.module_test, &config.module_test),
            module_tests: text(&plan.module_tests, &config.module_tests),
            module_graph: text(&plan.module_graph, &config.module_graph),
            module_names: pick(&plan.module_names, &config.module_names),
            full_triggers: pick(&plan.full_triggers, &config.full_triggers),
            slow_tests: text(&plan.slow_tests, &config.slow_tests),
            timing_tests: text(&plan.timing_tests, &config.timing_tests),
            skip_markers: pick(&plan.skip_markers, &config.skip_markers),
            test_paths: pick(&plan.test_paths, &config.test_paths),
            full_shards: pick(&plan.full_shards, &config.full_shards),
            toolchain_id: text(&plan.toolchain_id, &config.toolchain_id),
            ..ProfileSpec::default()
        })
    }

    /// A stored profile's keys as they are, a blank command included, so [`validate`]
    /// can refuse it.
    pub fn from_repo(profile: &RepoProfile) -> TierProfile {
        TierProfile::build(profile.spec())
    }

    fn build(spec: ProfileSpec) -> TierProfile {
        let module_graph = GraphSource::from_key(spec.module_graph.as_deref());
        let module_names = spec.module_names.unwrap_or(match module_graph {
            GraphSource::Cargo => ModuleNames::Cargo,
            _ => ModuleNames::Dir,
        });
        TierProfile {
            build_check: spec.build_check,
            module_test: spec.module_test,
            module_tests: spec.module_tests,
            module_graph,
            module_names,
            full_triggers: spec.full_triggers.unwrap_or_default(),
            slow_tests: spec.slow_tests,
            timing_tests: spec.timing_tests,
            skip_markers: spec.skip_markers.unwrap_or_default(),
            test_paths: spec.test_paths.unwrap_or_default(),
            full_shards: spec.full_shards.unwrap_or(1),
            toolchain_id: spec.toolchain_id,
        }
    }
}

/// Decision 11: the 16-hex FNV-1a of the frozen profile's canonical JSON, every field
/// (`env` included), a part of every result-cache key. `build_run` records it.
pub fn profile_hash(profile: &super::model::Profile) -> String {
    let json = serde_json::to_string(profile).unwrap_or_default();
    let mut hash = crate::profile::store::Fnv1a64::new();
    hash.update(json.as_bytes());
    format!("{:016x}", hash.0)
}

/// What a step's result may be reused for (decision 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scope {
    Gate,
    Full,
}

/// A module graph (decision 9): each module's directory, when known, and the modules
/// it depends on.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleGraph {
    pub modules: BTreeMap<String, ModuleInfo>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModuleInfo {
    pub dir: Option<String>,
    pub deps: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GraphState {
    Known(ModuleGraph),
    Unknown(String),
}

/// Decision 22's affected set: `Full(reason)` runs `check`; `Modules` names the modules
/// whose tests run, empty when every changed path was ignored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Affected {
    Full(String),
    Modules(BTreeSet<String>),
}

impl Affected {
    /// The `affected` part of a cache key (decision 30): the sorted names joined by `,`,
    /// or `full:<reason>`.
    pub fn key(&self) -> String {
        match self {
            Affected::Full(reason) => format!("full:{reason}"),
            Affected::Modules(names) => names.iter().cloned().collect::<Vec<_>>().join(","),
        }
    }
}

/// One step of a tier job (decision 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StepKind {
    Build,
    Tests,
    Timing,
    /// Shard `k` (1-based) of `of`.
    Shard {
        k: u8,
        of: u8,
    },
}

/// A step: its command after substitution, whether it must run alone (decision 25),
/// and the `affected` part of its cache key (`-` for a build step).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Step {
    pub kind: StepKind,
    pub command: String,
    pub exclusive: bool,
    pub affected_key: String,
}

/// The steps a tier job runs, in order (decision 21).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TierPlan {
    pub scope: Scope,
    pub affected: Affected,
    pub steps: Vec<Step>,
}

/// What a result-cache key takes besides the step (decision 30); `None` on a `TierSpec`
/// for an untiered profile or an `"unknown"` toolchain.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CacheCtx {
    pub profile_hash: String,
    pub toolchain: String,
}

/// A test-weakening signal (decision 40). `line` is on the new side for a skip marker
/// and on the old side for an assertion loss.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Signal {
    DeletedTestFile {
        path: String,
    },
    SkipMarker {
        path: String,
        line: u32,
        marker: String,
    },
    AssertionLoss {
        path: String,
        line: u32,
        removed: u32,
        added: u32,
    },
}

/// Which placeholders a command key may hold (decision 7's "Allowed in").
fn allowed(key: &str, piece: &Piece<'_>) -> bool {
    match piece {
        Piece::Text(_) => true,
        Piece::Module => key == "module_test",
        Piece::Modules | Piece::ModulesEach(_) => key == "module_tests",
        Piece::Filter(_) => matches!(key, "module_test" | "module_tests" | "check"),
        Piece::Shard | Piece::Shards => key == "check",
        Piece::Test => false,
    }
}

/// Whether a template holds a `%` that is not part of a `%%`.
fn names_the_value(template: &str) -> bool {
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some('%') => {}
                _ => return true,
            }
        }
    }
    false
}

fn has_filter(command: &str) -> bool {
    pieces(command)
        .iter()
        .any(|piece| matches!(piece, Piece::Filter(_)))
}

/// Decision 8: every problem of the tier keys, each `(key, message)`. `check` is the
/// profile's own; its placeholders are checked only for a tiered profile, so an
/// untiered one is refused for nothing M8a would accept.
pub fn validate(
    tiers: &TierProfile,
    check: Option<&str>,
    modules: &[String],
) -> Vec<(String, String)> {
    let mut problems: Vec<(String, String)> = Vec::new();
    let mut push = |key: &str, message: String| problems.push((key.to_string(), message));
    let graph_command = match &tiers.module_graph {
        GraphSource::Command(command) => Some(command),
        _ => None,
    };
    let commands = [
        ("build_check", tiers.build_check.as_ref()),
        ("module_test", tiers.module_test.as_ref()),
        ("module_tests", tiers.module_tests.as_ref()),
        ("module_graph", graph_command),
        ("slow_tests", tiers.slow_tests.as_ref()),
        ("timing_tests", tiers.timing_tests.as_ref()),
        ("toolchain_id", tiers.toolchain_id.as_ref()),
    ];
    for (key, value) in commands {
        let Some(value) = value else {
            continue;
        };
        if value.trim().is_empty() {
            push(key, "must not be blank".to_string());
        } else if value.chars().count() > COMMAND_CHARS_MAX {
            push(key, format!("at most {COMMAND_CHARS_MAX} characters"));
        }
    }
    let tiered = tiers.is_tiered();
    let templates = [
        ("build_check", tiers.build_check.as_deref()),
        ("module_test", tiers.module_test.as_deref()),
        ("module_tests", tiers.module_tests.as_deref()),
        ("module_graph", graph_command.map(String::as_str)),
        ("toolchain_id", tiers.toolchain_id.as_deref()),
        ("check", check.filter(|_| tiered)),
    ];
    for (key, template) in templates {
        for piece in template.map(pieces).unwrap_or_default() {
            if !allowed(key, &piece) {
                push(key, format!("{} is not allowed here", piece.name()));
            }
        }
    }
    if let Some(template) = &tiers.module_test {
        let count = pieces(template)
            .iter()
            .filter(|piece| matches!(piece, Piece::Module))
            .count();
        if count != 1 {
            push(
                "module_test",
                "must contain {module} exactly once".to_string(),
            );
        }
        if modules.is_empty() {
            push("module_test", "needs modules".to_string());
        }
    }
    if let Some(template) = &tiers.module_tests {
        if !pieces(template)
            .iter()
            .any(|piece| matches!(piece, Piece::Modules | Piece::ModulesEach(_)))
        {
            push(
                "module_tests",
                "must contain {modules} or {modules:<template>}".to_string(),
            );
        }
        // A template with no unescaped `%` (`{modules:}`, `{modules:-p}`) names no
        // module: the command would run without the affected set.
        if pieces(template)
            .iter()
            .any(|piece| matches!(piece, Piece::ModulesEach(t) if !names_the_value(t)))
        {
            push(
                "module_tests",
                "{modules:<template>} needs a % for the module name".to_string(),
            );
        }
        if modules.is_empty() {
            push("module_tests", "needs modules".to_string());
        }
    }
    if tiers.module_names == ModuleNames::Cargo && tiers.module_graph != GraphSource::Cargo {
        push(
            "module_names",
            "cargo needs module_graph = \"cargo\"".to_string(),
        );
    }
    for (filter, what) in [
        (tiers.slow_tests.as_ref(), "slow_tests"),
        (tiers.timing_tests.as_ref(), "timing_tests"),
    ] {
        if filter.is_none() {
            continue;
        }
        let tests = what.trim_end_matches("_tests");
        for (key, template) in [
            ("module_test", tiers.module_test.as_deref()),
            ("module_tests", tiers.module_tests.as_deref()),
        ] {
            if template.is_some_and(|t| !has_filter(t)) {
                push(
                    what,
                    format!("{key} must contain {{filter:<template>}} to leave {tests} tests out"),
                );
            }
        }
    }
    if !(1..=FULL_SHARDS_MAX).contains(&tiers.full_shards) {
        push(
            "full_shards",
            format!("must be between 1 and {FULL_SHARDS_MAX}"),
        );
    } else if tiers.full_shards > 1 {
        let check = check.map(pieces).unwrap_or_default();
        let shard = check.iter().any(|piece| matches!(piece, Piece::Shard));
        let shards = check.iter().any(|piece| matches!(piece, Piece::Shards));
        if !(shard && shards) {
            push(
                "full_shards",
                "check must contain {shard} and {shards}".to_string(),
            );
        }
    }
    for (key, globs) in [
        ("full_triggers", &tiers.full_triggers),
        ("test_paths", &tiers.test_paths),
    ] {
        for glob in globs {
            if let Err(e) = validate_glob(glob) {
                push(key, format!("{glob} {e}"));
            }
        }
    }
    if tiers.skip_markers.len() > SKIP_MARKERS_MAX
        || tiers
            .skip_markers
            .iter()
            .any(|m| m.is_empty() || m.chars().count() > SKIP_MARKER_CHARS_MAX)
    {
        push(
            "skip_markers",
            format!(
                "at most {SKIP_MARKERS_MAX} markers, each 1 to {SKIP_MARKER_CHARS_MAX} characters"
            ),
        );
    }
    problems
}
