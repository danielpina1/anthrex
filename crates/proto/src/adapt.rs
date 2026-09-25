//! Adaptation (milestone 8b): triage and the run path, the deciders' results, diff
//! measurement, per-phase times and usage by role.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::run::{Size, TaskKind};
use crate::run_info::TokenUsage;

/// Which path a run takes (spec §5.1). A fast-path run has one task and no plan gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunPath {
    Fast,
    Plan,
    Large,
}

/// The triage decider's estimate of a goal's scale.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Scale {
    Single,
    Plan,
    Large,
}

/// Whether a decision came from the decider or from its deterministic fallback.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeciderSource {
    Decider,
    Fallback,
}

/// `[orchestrator.deciders] mode`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DeciderMode {
    #[default]
    Claude,
    Codex,
    Off,
}

/// What triage decided for a goal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageInfo {
    pub kinds: Vec<TaskKind>,
    pub scale: Scale,
    pub path: RunPath,
    pub reason: String,
    pub source: DeciderSource,
    pub fallback_reason: Option<String>,
    pub at: u64,
}

/// The size cross-check of one planned task (spec §7.2 rule 5); it can only raise.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SizeCheckInfo {
    pub engine: Size,
    pub decided: Option<Size>,
    pub agreed: bool,
    pub reason: String,
    pub source: DeciderSource,
}

/// A task's merged diff, measured by the engine.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffStats {
    pub files: u32,
    pub hunks: u32,
    pub added: u32,
    pub removed: u32,
}

/// Seconds a task spent in each phase.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseSecs {
    pub queued: u64,
    pub preparing: u64,
    pub working: u64,
    pub proof: u64,
    pub check: u64,
    pub review: u64,
    pub merge: u64,
    pub blocked: u64,
}

/// A run's token usage, in total and by role.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunUsage {
    pub total: TokenUsage,
    /// Keyed `worker`, `reviewer`, `scout`, `decider`, `orchestrator`.
    pub by_role: BTreeMap<String, TokenUsage>,
    pub decider_calls: u32,
    pub decider_fallbacks: u32,
}
