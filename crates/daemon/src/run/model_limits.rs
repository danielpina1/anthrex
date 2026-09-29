//! [`RunLimits`]: the limits a run is frozen with at start, and the [`ClaudeAuth`] they
//! hold. Split out of `model.rs` before milestone 9.1 to keep it under the 600-line
//! rule; a pure move. Pure.

use proto::{Budget, Runtime};
use serde::{Deserialize, Serialize};

use super::adapt;

/// `[orchestrator.claude] auth`, mirrored here with serde because `config::ClaudeAuth`
/// has no serde derive (the config crate does not depend on serde) and [`RunLimits`] is
/// persisted with the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeAuth {
    #[default]
    Login,
    ApiKey,
}

impl From<config::ClaudeAuth> for ClaudeAuth {
    fn from(auth: config::ClaudeAuth) -> Self {
        match auth {
            config::ClaudeAuth::Login => ClaudeAuth::Login,
            config::ClaudeAuth::ApiKey => ClaudeAuth::ApiKey,
        }
    }
}

impl From<ClaudeAuth> for config::ClaudeAuth {
    fn from(auth: ClaudeAuth) -> Self {
        match auth {
            ClaudeAuth::Login => config::ClaudeAuth::Login,
            ClaudeAuth::ApiKey => config::ClaudeAuth::ApiKey,
        }
    }
}

/// The limits a run is frozen with at start: `[orchestrator]`, with the plan's
/// `max_writers`, `max_readers` and `max_bounces` winning when set.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunLimits {
    pub max_writers: u8,
    pub max_readers: u8,
    pub max_bounces: u8,
    pub max_tasks: u32,
    pub max_windows: u32,
    pub default_runtime: Runtime,
    pub review_small: bool,
    pub budget_s: Budget,
    pub budget_m: Budget,
    pub budget_l: Budget,
    pub stall_after_secs: u64,
    pub rate_limit_retry_secs: u64,
    pub denials_before_block: u32,
    pub git_timeout_secs: u64,
    pub worker_permission_mode: String,
    pub worker_allowed_tools: Vec<String>,
    pub worker_codex_sandbox: String,
    pub worker_sandbox: bool,
    /// M8a final fix batch F1c round 2: checks, proofs and `setup` run unconfined
    /// (the platform cannot confine them, and the user allowed it at `run start`).
    /// Absent from a run recorded before: `false`.
    #[serde(default)]
    pub unconfined_checks: bool,
    pub claude_auth: ClaudeAuth,
    /// `[orchestrator.claude] api_key_helper`, passed to Claude sessions under
    /// `auth = "api_key"` (decision 50). Added by M8a.11: a session spec is built from
    /// the run alone.
    #[serde(default)]
    pub api_key_helper: Option<String>,
    /// M8b decision 18: `[orchestrator.deciders] mode`. Absent from a run recorded
    /// before milestone 8b: `off`, so a restored run gains no decider.
    #[serde(default = "adapt::decider_mode_absent")]
    pub decider_mode: proto::DeciderMode,
    /// M8b decision 18: `[orchestrator.deciders] slot_wait_secs`.
    #[serde(default = "adapt::slot_wait_absent")]
    pub decider_slot_wait_secs: u64,
    /// Milestone 9 (ruling D-5): the orchestrator settings, frozen at run start.
    #[serde(default)]
    pub orch: crate::run::orch::OrchLimits,
}
