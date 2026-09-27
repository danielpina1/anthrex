//! Scouts (milestone 8b decisions 12 to 16): read-only headless sessions that report
//! through `submit_scout_report`. The onboarding scout is one; milestone 9's area scouts
//! reuse the same types.

use serde::{Deserialize, Serialize};

use crate::profile::RepoProfile;
use crate::run::Route;
use crate::run_info::TokenUsage;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoutKind {
    Onboarding,
    Area,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoutState {
    Starting,
    Working,
    Reported,
    Failed,
}

/// One file a scout read, and why it matters.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoutFile {
    pub path: String,
    pub why: String,
}

/// A scout's stored report: `<data_dir>/runs/<run>/scouts/<id>.json` for a run scout,
/// `<repo_dir>/scouts/<id>.json` for a repository one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoutReport {
    pub id: String,
    pub kind: ScoutKind,
    pub run_id: Option<String>,
    pub question: String,
    pub summary: String,
    pub files: Vec<ScoutFile>,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub interfaces: Vec<String>,
    #[serde(default)]
    pub risks: Vec<String>,
    #[serde(default)]
    pub profile: Option<RepoProfile>,
    pub route: Route,
    pub window_id: Option<u32>,
    pub started_at: u64,
    pub finished_at: u64,
    pub tool_calls: u32,
    pub usage: TokenUsage,
}

/// One scout, as a snapshot shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScoutInfo {
    pub id: String,
    pub kind: ScoutKind,
    pub question: String,
    pub state: ScoutState,
    pub failure: Option<String>,
    pub window_id: Option<u32>,
    pub route: Route,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub tool_calls: u32,
    pub report_bytes: Option<u32>,
    pub files: Vec<String>,
    pub usage: TokenUsage,
}
