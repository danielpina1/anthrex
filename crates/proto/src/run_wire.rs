//! The `run` wire messages: `ClientMsg::Run(RunRequest)` and `DaemonMsg::Run(RunReply)`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::adapt::TriageInfo;
use crate::history::HistoryStats;
use crate::orch::OrchestratorChoice;
use crate::profile::{
    DroppedCommand, ProfileMeta, ProfileSource, ProfileStatus, ProfileVerification,
};
use crate::run::{AgentRole, FinishAction, PlanEdit, RunState};
use crate::run_info::{BaseMovedInfo, RunsSnapshot};
use crate::task_detail::TaskDetailInfo;

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
    /// Milestone 9: the calling sub-planner's epic (`anthrex mcp --epic`).
    #[serde(default)]
    pub epic: Option<String>,
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
        /// Milestone 9 decision 13 (M9.7 review fixes, ruling 5): after the batch, the
        /// user submits a planning run's plan, as its orchestrator's `submit` would.
        #[serde(default)]
        submit: bool,
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
        /// Milestone 9 decision 6: `--orchestrator <runtime>[:<model>]`.
        #[serde(default)]
        orchestrator: Option<OrchestratorChoice>,
    },
    Promote {
        run_id: String,
        /// Milestone 9 decision 6.
        #[serde(default)]
        orchestrator: Option<OrchestratorChoice>,
    },
    Stats {
        dir: PathBuf,
    },
    Profile(ProfileRequest),
    // Milestone 9 decision 28: answered with `request::APPROVE` and `request::REJECT`.
    ApproveHold {
        run_id: String,
        hold: String,
    },
    RejectHold {
        run_id: String,
        hold: String,
    },
    // Milestone 9.0.5 decision 7: answered with `RunReply::TaskDetail`, or refused under
    // `request::TASK_DETAIL`.
    TaskDetail {
        run_id: String,
        task_id: String,
    },
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
///
/// Every reply that answers a `RunRequest` ends with `request_id` (milestone 9
/// decision 2): it echoes a `ClientMsg::RunTagged` id, and is `None` for a
/// `ClientMsg::Run` and for a reply from before milestone 9. `Snapshot` is the one
/// exception: it is state, not an answer, and a subscription pushes the same value
/// unasked, so any snapshot answers `List` equally.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RunReply {
    Started {
        run_id: String,
        state: RunState,
        #[serde(default)]
        request_id: Option<u64>,
    },
    Done {
        request: String,
        message: String,
        #[serde(default)]
        request_id: Option<u64>,
    },
    Refused {
        request: String,
        message: String,
        #[serde(default)]
        request_id: Option<u64>,
    },
    /// `base_moved`: `Some` when accepting would land onto an advanced base and the
    /// client must confirm `"<run id>@<to>"`.
    ConfirmNeeded {
        run_id: String,
        prompt: String,
        base_moved: Option<BaseMovedInfo>,
        #[serde(default)]
        request_id: Option<u64>,
    },
    Snapshot(RunsSnapshot),
    ToolResult {
        ok: bool,
        text: String,
        #[serde(default)]
        request_id: Option<u64>,
    },
    // Milestone 8b.
    Triaged {
        triage: TriageInfo,
        run_id: Option<String>,
        message: String,
        #[serde(default)]
        request_id: Option<u64>,
    },
    /// `reply` is boxed so `RunReply` (and `DaemonMsg`, which every broadcast slot
    /// holds) stays small; a `Box` is invisible on the wire. A struct variant since
    /// milestone 9, so it can carry `request_id`.
    Profile {
        reply: Box<ProfileReply>,
        #[serde(default)]
        request_id: Option<u64>,
    },
    /// A struct variant since milestone 9, so it can carry `request_id`.
    Stats {
        stats: HistoryStats,
        #[serde(default)]
        request_id: Option<u64>,
    },
    /// Milestone 9.0.5 decision 2: one task's brief, acceptance and worker summary, on
    /// request only. Boxed for the same reason as `Profile`.
    TaskDetail {
        detail: Box<TaskDetailInfo>,
        #[serde(default)]
        request_id: Option<u64>,
    },
}

impl RunReply {
    /// `Done` for an untagged request (`request_id: None`).
    pub fn done(request: impl Into<String>, message: impl Into<String>) -> RunReply {
        RunReply::Done {
            request: request.into(),
            message: message.into(),
            request_id: None,
        }
    }

    /// `Refused` for an untagged request (`request_id: None`).
    pub fn refused(request: impl Into<String>, message: impl Into<String>) -> RunReply {
        RunReply::Refused {
            request: request.into(),
            message: message.into(),
            request_id: None,
        }
    }

    /// `ToolResult` for an untagged request (`request_id: None`).
    pub fn tool_result(ok: bool, text: impl Into<String>) -> RunReply {
        RunReply::ToolResult {
            ok,
            text: text.into(),
            request_id: None,
        }
    }

    /// `Profile` for an untagged request (`request_id: None`).
    pub fn profile(reply: ProfileReply) -> RunReply {
        RunReply::Profile {
            reply: Box::new(reply),
            request_id: None,
        }
    }

    /// `Stats` for an untagged request (`request_id: None`).
    pub fn stats(stats: HistoryStats) -> RunReply {
        RunReply::Stats {
            stats,
            request_id: None,
        }
    }

    /// Stamps a tagged request's id on every reply that answers a request. A
    /// `Snapshot` is returned as it is.
    pub fn tagged(mut self, id: Option<u64>) -> RunReply {
        match &mut self {
            RunReply::Started { request_id, .. }
            | RunReply::Done { request_id, .. }
            | RunReply::Refused { request_id, .. }
            | RunReply::ConfirmNeeded { request_id, .. }
            | RunReply::ToolResult { request_id, .. }
            | RunReply::Triaged { request_id, .. }
            | RunReply::Profile { request_id, .. }
            | RunReply::Stats { request_id, .. }
            | RunReply::TaskDetail { request_id, .. } => *request_id = id,
            RunReply::Snapshot(_) => {}
        }
        self
    }

    /// The id a tagged request's reply echoes; `None` for a `Snapshot`.
    pub fn request_id(&self) -> Option<u64> {
        match self {
            RunReply::Started { request_id, .. }
            | RunReply::Done { request_id, .. }
            | RunReply::Refused { request_id, .. }
            | RunReply::ConfirmNeeded { request_id, .. }
            | RunReply::ToolResult { request_id, .. }
            | RunReply::Triaged { request_id, .. }
            | RunReply::Profile { request_id, .. }
            | RunReply::Stats { request_id, .. }
            | RunReply::TaskDetail { request_id, .. } => *request_id,
            RunReply::Snapshot(_) => None,
        }
    }
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
    /// Milestone 9.0.5 decision 7.
    pub const TASK_DETAIL: &str = "run task-detail";
}
