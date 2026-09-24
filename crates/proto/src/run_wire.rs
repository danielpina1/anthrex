//! The `run` wire messages: `ClientMsg::Run(RunRequest)` and `DaemonMsg::Run(RunReply)`.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

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
}

/// Client → daemon, carried inside `ClientMsg::Run`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RunRequest {
    Start {
        plan_toml: String,
        dir: PathBuf,
        yes: bool,
        trust_project: bool,
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
}
