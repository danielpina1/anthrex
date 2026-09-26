//! The `run` wire messages: `ClientMsg::Run(RunRequest)` and `DaemonMsg::Run(RunReply)`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::adapt::TriageInfo;
use crate::history::HistoryStats;
use crate::profile::{
    DroppedCommand, ProfileMeta, ProfileSource, ProfileStatus, ProfileVerification,
};
use crate::run::{AgentRole, FinishAction, PlanEdit, RunState};
use crate::run_info::{BaseMovedInfo, RunsSnapshot};

/// One MCP tool call an agent round makes into the run engine, such as `task_done`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolCall {
    pub run_id: String,
    pub task_id: Option<String>,
    pub role: AgentRole,
    pub window_id: u32,
    pub tool: String,
    pub args: serde_json::Value,
    /// Milestone 8b: the calling scout's id (`anthrex mcp --scout`).
    #[serde(default)]
    pub scout_id: Option<String>,
}

/// Client → daemon, carried inside `ClientMsg::Run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RunRequest {
    Start {
        plan_toml: String,
        dir: PathBuf,
        yes: bool,
        trust_project: bool,
        /// M8a final fix batch F1c round 2: allows checks, proofs and `setup` to run
        /// unconfined where the daemon's platform cannot confine them. Never turns
        /// confinement off where it is available.
        #[serde(default)]
        unconfined_checks: bool,
    },
    Approve {
        run_id: String,
    },
    Reject {
        run_id: String,
    },
    Edit {
        run_id: String,
        edits: Vec<PlanEdit>,
    },
    Retry {
        run_id: String,
        task_id: String,
    },
    Override {
        run_id: String,
        task_id: String,
        reason: String,
    },
    Cancel {
        run_id: String,
    },
    Resume {
        run_id: String,
        rebaseline: bool,
    },
    Finish {
        run_id: String,
        action: FinishAction,
        confirm: Option<String>,
    },
    List,
    Subscribe,
    Unsubscribe,
    Tool(ToolCall),
    // Milestone 8b.
    StartGoal {
        goal: String,
        dir: PathBuf,
        yes: bool,
        trust_project: bool,
        unconfined_checks: bool,
    },
    Promote {
        run_id: String,
    },
    Stats {
        dir: PathBuf,
    },
    Profile(ProfileRequest),
}

/// `anthrex profile …`, carried inside `RunRequest::Profile` (milestone 8b decision 10).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ProfileRequest {
    Status {
        dir: PathBuf,
    },
    Detect {
        dir: PathBuf,
        trust_project: bool,
        unconfined_checks: bool,
    },
    Show {
        dir: PathBuf,
        proposed: bool,
    },
    /// `shown`: the `show_text` the user was shown; a proposal that no longer reads
    /// so is refused, so confirming stores exactly what was shown.
    Confirm {
        dir: PathBuf,
        #[serde(default)]
        shown: Option<String>,
    },
    Reject {
        dir: PathBuf,
    },
    /// `value: None` is `--unset`.
    Edit {
        dir: PathBuf,
        key: String,
        value: Option<String>,
        yes: bool,
        unconfined_checks: bool,
    },
}

/// Daemon → client, carried inside `DaemonMsg::Run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RunReply {
    Started {
        run_id: String,
        state: RunState,
    },
    Done {
        request: String,
        message: String,
    },
    Refused {
        request: String,
        message: String,
    },
    /// `base_moved`: `Some` when accepting would land onto an advanced base and the
    /// client must confirm `"<run id>@<to>"`.
    ConfirmNeeded {
        run_id: String,
        prompt: String,
        base_moved: Option<BaseMovedInfo>,
    },
    Snapshot(RunsSnapshot),
    ToolResult {
        ok: bool,
        text: String,
    },
    // Milestone 8b.
    Triaged {
        triage: TriageInfo,
        run_id: Option<String>,
        message: String,
    },
    /// Boxed so `RunReply` (and `DaemonMsg`, which every broadcast slot holds) stays
    /// small; a `Box` is invisible on the wire.
    Profile(Box<ProfileReply>),
    Stats(HistoryStats),
}

/// The answer to a `ProfileRequest`, carried inside `RunReply::Profile`. Built once per
/// request and boxed there, so its own variants' sizes do not matter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[allow(clippy::large_enum_variant)]
pub enum ProfileReply {
    Status(ProfileStatus),
    Shown {
        source: ProfileSource,
        toml: String,
        meta: Option<ProfileMeta>,
        verification: Option<ProfileVerification>,
        dropped: Vec<DroppedCommand>,
    },
    Done {
        message: String,
    },
    Refused {
        message: String,
    },
}

/// The `request` labels a client matches on, spelled `proto::run_wire::request` at every
/// call site. `proto::messages::request` (`CREATE`, `REMOVE`) already exists, so neither
/// this module nor its contents are re-exported at the crate root.
pub mod request {
    pub const START: &str = "run start";
    pub const APPROVE: &str = "run approve";
    pub const REJECT: &str = "run reject";
    pub const EDIT: &str = "run edit";
    pub const RETRY: &str = "run retry";
    pub const OVERRIDE: &str = "run override";
    pub const CANCEL: &str = "run cancel";
    pub const RESUME: &str = "run resume";
    pub const FINISH: &str = "run finish";
    /// A `DaemonMsg::Error` that refuses a `RunRequest::Tool` outright carries this
    /// label; `anthrex mcp` ends its wait on it and skips every other `Error`. (The
    /// engine's normal answer, refusals included, is `RunReply::ToolResult`.)
    pub const TOOL: &str = "run tool";
    pub const START_GOAL: &str = "run start --goal";
    pub const PROMOTE: &str = "run promote";
    pub const STATS: &str = "run stats";
    pub const PROFILE: &str = "profile";
}
