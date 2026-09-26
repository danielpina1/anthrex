//! Deciders (M8b decisions 16 to 21): one-shot headless calls that return
//! schema-validated JSON, each with a deterministic fallback. Pure except `call.rs`.
//!
//! This file holds the types: what a decider is asked ([`DeciderRequest`]), what it
//! answers ([`DeciderAnswer`]), and the [`Decision`] the engine records. They travel in
//! `OpKind::Decide` and `OpResult::Decided`, so they serialize.

pub mod argv;
pub mod fallback;
pub mod parse;
pub mod prompt;
pub mod schema;

pub use argv::{AnswerSource, DECIDER_CAPS, DeciderCaps};

use crate::headless::argv::CliCaps;
use proto::{DeciderMode, DeciderSource, Route, Scale, Size, TaskKind, TestMode, TokenUsage};
use serde::{Deserialize, Serialize};
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

/// The four decider kinds (decision 17).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeciderKind {
    Triage,
    SizeCheck,
    CheckSummary,
    BlockedReason,
}

impl DeciderKind {
    /// Every kind, in the order the brief lists them.
    pub const ALL: [DeciderKind; 4] = [
        DeciderKind::Triage,
        DeciderKind::SizeCheck,
        DeciderKind::CheckSummary,
        DeciderKind::BlockedReason,
    ];

    /// The kind's name in prompts, schema file names and `fake-agent`'s scripts.
    pub fn label(self) -> &'static str {
        match self {
            DeciderKind::Triage => "triage",
            DeciderKind::SizeCheck => "size_check",
            DeciderKind::CheckSummary => "check_summary",
            DeciderKind::BlockedReason => "blocked_reason",
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
}

impl DeciderRequest {
    pub fn kind(&self) -> DeciderKind {
        match self {
            DeciderRequest::Triage(_) => DeciderKind::Triage,
            DeciderRequest::SizeCheck(_) => DeciderKind::SizeCheck,
            DeciderRequest::CheckSummary(_) => DeciderKind::CheckSummary,
            DeciderRequest::BlockedReason(_) => DeciderKind::BlockedReason,
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
    CheckSummary { lines: Vec<String> },
    BlockedReason { kind: BlockKind, reason: String },
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
/// daemon start (`DeciderContext::new`, M8b.7).
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
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_argv;
#[cfg(test)]
mod tests_prompt;
