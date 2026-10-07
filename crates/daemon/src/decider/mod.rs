//! Deciders (M8b decisions 16 to 21): one-shot headless calls that return
//! schema-validated JSON, each with a deterministic fallback. Pure except `call.rs`.
//!
//! This file holds the types: what a decider is asked ([`DeciderRequest`]), what it
//! answers ([`DeciderAnswer`]), and the [`Decision`] the engine records. They travel in
//! `OpKind::Decide` and `OpResult::Decided`, so they serialize.

pub mod argv;
pub mod call;
pub mod ci;
pub mod fallback;
pub mod parse;
pub mod prompt;
pub mod run_name;
pub mod schema;

pub use argv::{AnswerSource, DECIDER_CAPS, DeciderCaps, caps};
pub use ci::{CI_SUMMARY_INPUT_BYTES, CiSummaryInput};
pub use run_name::RunNameInput;

use crate::headless::argv::CliCaps;
use crate::manager::ManagerConfig;
use proto::{
    DeciderMode, DeciderSource, Route, Runtime, Scale, Size, SizeThresholds, TaskKind, TestMode,
    TokenUsage,
};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The decider kinds (decision 17; milestone 9.2 decision 18 appends `CiSummary`, and
/// the run title change `RunName`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeciderKind {
    Triage,
    SizeCheck,
    CheckSummary,
    BlockedReason,
    CiSummary,
    RunName,
}

impl DeciderKind {
    /// Every kind, in the order the briefs list them.
    pub const ALL: [DeciderKind; 6] = [
        DeciderKind::Triage,
        DeciderKind::SizeCheck,
        DeciderKind::CheckSummary,
        DeciderKind::BlockedReason,
        DeciderKind::CiSummary,
        DeciderKind::RunName,
    ];

    /// The kind's name in prompts, schema file names and `fake-agent`'s scripts.
    pub fn label(self) -> &'static str {
        match self {
            DeciderKind::Triage => "triage",
            DeciderKind::SizeCheck => "size_check",
            DeciderKind::CheckSummary => "check_summary",
            DeciderKind::BlockedReason => "blocked_reason",
            DeciderKind::CiSummary => "ci_summary",
            DeciderKind::RunName => "run_name",
        }
    }
}

/// Triage's input: the goal and the repository evidence it is judged against.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageInput {
    pub goal: String,
    /// `profile::summary` of the stored profile, empty when there is none.
    pub profile_summary: String,
    pub report_summary: Option<String>,
    pub report_files: Vec<String>,
    /// Tracked paths (a prefix of `git ls-files`), and how many there are in all.
    pub files: Vec<String>,
    pub files_total: u32,
    /// `[orchestrator] planner_task_cap`: the plan scale's upper bound (M9.3).
    pub planner_task_cap: u32,
    /// Milestone 9.5 decision 13: the repository's line thresholds (`tuning.toml`).
    #[serde(default, skip_serializing_if = "default_thresholds")]
    pub thresholds: SizeThresholds,
}

fn default_thresholds(t: &SizeThresholds) -> bool {
    *t == SizeThresholds::default()
}

/// One task of a size check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeCheckTask {
    pub id: String,
    pub title: String,
    pub brief: String,
    pub acceptance: Vec<String>,
    pub owns: Vec<String>,
    pub deps: Vec<String>,
    pub size: Size,
    pub interface_change: bool,
    pub hub: bool,
}

/// One scout report, as a size check's evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    pub summary: String,
    pub files: Vec<String>,
    pub modules: Vec<String>,
    pub interfaces: Vec<String>,
}

/// A size check's input. The engine names the reports (`evidence_refs`); the driver
/// fills `evidence` before the call.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeCheckInput {
    pub tasks: Vec<SizeCheckTask>,
    pub evidence_refs: Vec<String>,
    pub evidence: Vec<Evidence>,
    pub modules: Vec<String>,
    pub hub: Vec<String>,
    /// Milestone 9.5 decision 13: the run's frozen line thresholds.
    #[serde(default, skip_serializing_if = "default_thresholds")]
    pub thresholds: SizeThresholds,
}

/// A failed check to summarise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckSummaryInput {
    pub task_id: String,
    pub command: String,
    pub code: Option<i32>,
    pub timed_out: bool,
    pub tail: String,
}

/// A free-text `task_blocked` reason to classify.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedReasonInput {
    pub task_id: String,
    pub title: String,
    pub reason: String,
}

/// What a decider is asked.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeciderRequest {
    Triage(TriageInput),
    SizeCheck(SizeCheckInput),
    CheckSummary(CheckSummaryInput),
    BlockedReason(BlockedReasonInput),
    CiSummary(CiSummaryInput),
    RunName(RunNameInput),
}

impl DeciderRequest {
    pub fn kind(&self) -> DeciderKind {
        match self {
            DeciderRequest::Triage(_) => DeciderKind::Triage,
            DeciderRequest::SizeCheck(_) => DeciderKind::SizeCheck,
            DeciderRequest::CheckSummary(_) => DeciderKind::CheckSummary,
            DeciderRequest::BlockedReason(_) => DeciderKind::BlockedReason,
            DeciderRequest::CiSummary(_) => DeciderKind::CiSummary,
            DeciderRequest::RunName(_) => DeciderKind::RunName,
        }
    }
}

/// Triage's one task, for scale `single`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageTask {
    pub title: String,
    pub brief: String,
    pub acceptance: Vec<String>,
    pub owns: Vec<String>,
    pub size: Size,
    pub interface_change: bool,
    pub test_mode: TestMode,
    pub test_mode_reason: Option<String>,
    pub test_to_write: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageAnswer {
    pub kinds: Vec<TaskKind>,
    pub scale: Scale,
    pub reason: String,
    /// `Some` exactly when `scale` is `Single`.
    pub task: Option<TriageTask>,
}

/// One task's size as the size check judged it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeVerdict {
    pub id: String,
    pub size: Size,
    pub reason: String,
}

/// How the blocked-reason decider classified a free-text block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockKind {
    Question,
    MisSized,
    Environment,
}

/// A validated answer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DeciderAnswer {
    Triage(TriageAnswer),
    SizeCheck(Vec<SizeVerdict>),
    CheckSummary {
        lines: Vec<String>,
    },
    BlockedReason {
        kind: BlockKind,
        reason: String,
    },
    CiSummary {
        lines: Vec<String>,
        failing_tests: Vec<String>,
        category: proto::CiCategory,
    },
    /// Empty `title` and `slug`: the fallback (`run_name::fallback`).
    RunName {
        title: String,
        slug: String,
    },
}

/// A decider's result: always an answer, from the decider or from its fallback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Decision {
    pub kind: DeciderKind,
    pub answer: DeciderAnswer,
    pub source: DeciderSource,
    /// One of decision 16's reasons, exactly, when `source` is `Fallback`.
    pub fallback_reason: Option<String>,
    pub usage: Option<TokenUsage>,
    pub secs: u64,
}

/// Everything a decider call needs besides its request (decision 16). Built once at
/// daemon start ([`DeciderContext::new`]).
#[derive(Debug, Clone)]
pub struct DeciderContext {
    pub mode: DeciderMode,
    pub program: OsString,
    pub route: Route,
    pub timeout: Duration,
    /// `<data_dir>/deciders/cwd`, an empty directory outside every repository.
    pub cwd: PathBuf,
    /// `<data_dir>/deciders/schemas`, where Codex's schema files go.
    pub schema_dir: PathBuf,
    pub caps: CliCaps,
    /// Milestone 9.5 (rulings RL-2, I6; decision 9a): what each call routes over.
    pub routing: call::Routing,
    /// The manager's launch gate: a Codex call waits for it (the startup version probe)
    /// before choosing its sandbox dialect (final review I1, ruling R6).
    pub launch_gate: crate::launch::LaunchGate,
}

impl DeciderContext {
    /// The context every decider call of this daemon uses (decision 16), built from the
    /// config and the manager's resolved commands. Pure: the call creates `cwd` and
    /// `schema_dir` itself.
    /// - `program`: `ANTHREX_DECIDER_BIN` (`manager.decider_bin`), else the mode's
    ///   runtime command (`claude_bin` or `codex_bin`; `claude_bin` when off, unused).
    /// - `route`: on the mode's runtime, the first roster entry at the lowest strength at
    ///   or above `deciders.strength`, else that runtime's first entry, else no model
    ///   (the CLI's default), at `deciders.effort`.
    pub fn new(cfg: &config::Orchestrator, manager: &ManagerConfig, data_dir: &Path) -> Self {
        let deciders = &cfg.deciders;
        let runtime = match deciders.mode {
            DeciderMode::Codex => Runtime::Codex,
            DeciderMode::Claude | DeciderMode::Off => Runtime::Claude,
        };
        let routing = call::Routing::new(cfg, manager);
        let route = call::ladder_route(
            &cfg.models,
            runtime,
            deciders.strength,
            deciders.effort.clone(),
        );
        let root = data_dir.join("deciders");
        DeciderContext {
            mode: deciders.mode,
            program: routing.program(runtime),
            route,
            timeout: Duration::from_secs(deciders.timeout_secs),
            cwd: root.join("cwd"),
            schema_dir: root.join("schemas"),
            caps: manager.cli_caps,
            routing,
            launch_gate: manager.launch_gate.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_argv;
#[cfg(test)]
mod tests_ci;
#[cfg(test)]
mod tests_context;
#[cfg(test)]
mod tests_prompt;
#[cfg(test)]
mod tests_route;
#[cfg(test)]
mod tests_run_name;
#[cfg(test)]
mod tests_thresholds;
