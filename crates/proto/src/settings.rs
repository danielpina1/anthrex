//! The Settings screen's wire types (milestone 9.0.6 decisions 23-26): the document the
//! screen edits, its request and reply, and the ranges `config` and the screen share.

use std::collections::BTreeMap;
use std::ops::RangeInclusive;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::models::ModelTable;

pub const MAX_WRITERS_RANGE: RangeInclusive<u8> = 1..=8;
pub const MAX_READERS_RANGE: RangeInclusive<u8> = 1..=8;
pub const MAX_BOUNCES_RANGE: RangeInclusive<u8> = 1..=5;
pub const STALL_AFTER_SECS_RANGE: RangeInclusive<u64> = 5..=7200;
/// A budget's tool calls and minutes are each at least this.
pub const BUDGET_MIN: u32 = 1;

/// The key paths `Origin` is reported for.
pub mod key {
    /// Milestone 9.8: the role table (`[models]`, or the old model keys it replaces).
    pub const ROLES: &str = "models";
    pub const BUDGET_S_CALLS: &str = "orchestrator.budget.s.tool_calls";
    pub const BUDGET_S_MINUTES: &str = "orchestrator.budget.s.minutes";
    pub const BUDGET_M_CALLS: &str = "orchestrator.budget.m.tool_calls";
    pub const BUDGET_M_MINUTES: &str = "orchestrator.budget.m.minutes";
    pub const BUDGET_L_CALLS: &str = "orchestrator.budget.l.tool_calls";
    pub const BUDGET_L_MINUTES: &str = "orchestrator.budget.l.minutes";
    pub const STALL_AFTER_SECS: &str = "orchestrator.stall_after_secs";
    pub const MAX_WRITERS: &str = "orchestrator.max_writers";
    pub const MAX_READERS: &str = "orchestrator.max_readers";
    pub const MAX_BOUNCES: &str = "orchestrator.max_bounces";
}

/// The eleven keys, in the order of `key`.
pub const SETTINGS_KEYS: [&str; 11] = [
    key::ROLES,
    key::BUDGET_S_CALLS,
    key::BUDGET_S_MINUTES,
    key::BUDGET_M_CALLS,
    key::BUDGET_M_MINUTES,
    key::BUDGET_L_CALLS,
    key::BUDGET_L_MINUTES,
    key::STALL_AFTER_SECS,
    key::MAX_WRITERS,
    key::MAX_READERS,
    key::MAX_BOUNCES,
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetLimit {
    pub tool_calls: u32,
    pub minutes: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsLimits {
    pub budget_s: BudgetLimit,
    pub budget_m: BudgetLimit,
    pub budget_l: BudgetLimit,
    pub stall_after_secs: u64,
    pub max_writers: u8,
    pub max_readers: u8,
    pub max_bounces: u8,
}

/// Everything the Settings screen owns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SettingsDoc {
    pub limits: SettingsLimits,
    /// Ruling T18-2: `[orchestrator.design].default`, which the goal dialog's
    /// `configured` names. Read-only here: a save never writes it. `None` from a daemon
    /// that did not say; left out while `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design_default: Option<crate::design::DesignMode>,
    /// Milestone 9.8: the global role table (`Orchestrator.roles`), which a save writes
    /// as `[models]` (decision 40). It replaced the roster and the orchestrator default.
    #[serde(default, skip_serializing_if = "ModelTable::is_empty")]
    pub roles: ModelTable,
}

/// Whether a key's value came from the file or is the built-in default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Origin {
    File,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettingsRequest {
    Get,
    Put {
        settings: SettingsDoc,
    },
    /// Milestone 9.8 decision 36: the repository's own role table.
    RepoModels {
        project: PathBuf,
    },
    PutRepoModels {
        project: PathBuf,
        table: ModelTable,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettingsReply {
    Current {
        doc: SettingsDoc,
        origin: BTreeMap<String, Origin>,
        path: PathBuf,
    },
    Saved {
        doc: SettingsDoc,
        origin: BTreeMap<String, Origin>,
    },
    Refused {
        problems: Vec<String>,
    },
    RepoModels {
        project: PathBuf,
        table: ModelTable,
        path: PathBuf,
        /// M9.8.12 fix round 1 (I2): the rows of `path` the daemon could not read, each
        /// starting with the path. A save refuses while there are any.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        problems: Vec<String>,
    },
    RepoSaved {
        project: PathBuf,
        table: ModelTable,
    },
}
