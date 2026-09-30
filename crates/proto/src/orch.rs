//! Milestone 9: the orchestrator, its approval holds, the per-epic integration reviews,
//! messages to workers and workers' task notes, as the wire carries them.
//!
//! `MessageTarget` and `MessageKind` live here, not in `run.rs` beside `PlanEdit`,
//! because `MessageTarget`'s serde would take `run.rs` past 500 lines (the brief's
//! file-size table allows the move).

use std::fmt;

use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::run::Route;
use crate::types::Runtime;

/// Decision 6: which runtime, and optionally which model, runs a run's orchestrator
/// (`--orchestrator <runtime>[:<model>]` or the TUI goal form).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestratorChoice {
    pub runtime: Runtime,
    #[serde(default)]
    pub model: Option<String>,
}

/// Decision 28: what an approval hold waits on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HoldKind {
    /// A promoted fast-path run's tasks.
    Promotion,
    /// An epic added after the plan gate.
    Epic { epic: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoldState {
    Drafting,
    Awaiting,
    Approved,
    Rejected,
}

/// One approval hold (decision 28), as the run view shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldInfo {
    pub id: String,
    pub kind: HoldKind,
    pub state: HoldState,
    pub tasks: Vec<String>,
    pub created_at: u64,
    pub decided_at: Option<u64>,
    pub decided_by: Option<String>,
}

/// The run's orchestrator window (decision 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestratorInfo {
    pub route: Route,
    pub window_id: Option<u32>,
    pub live: bool,
    pub started_at: u64,
    pub plan_submitted: bool,
    pub summary: Option<String>,
    pub notes: Vec<String>,
    pub wakes: u32,
    /// Milestone 9.0.5 decision 8: a wake-up is held back (`RunOrch.wake_held`).
    #[serde(default)]
    pub wake_held: bool,
}

/// Decision 37: where an epic's integration review stands.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntegrationState {
    #[default]
    NotYet,
    Reviewing,
    Approved,
    Changes,
    /// Closed by the `finish` edit.
    Finished,
}

/// One epic's integration review (decision 37).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IntegrationInfo {
    pub epic: String,
    pub state: IntegrationState,
    pub base: Option<String>,
    pub merges: Vec<String>,
    /// The `<epic>-int<n>` review tasks.
    pub tasks: Vec<String>,
}

/// Decision 42f: what a worker's `task_note` reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskNoteKind {
    Discovery,
    Risk,
    Progress,
}

/// One `task_note`, attributed to its task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskNoteInfo {
    pub task_id: String,
    pub kind: TaskNoteKind,
    pub text: String,
    pub at: u64,
}

/// Decision 42a: what a `message` asks of the worker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    Info,
    Change,
    StopAndWait,
}

/// Decision 42a: who a `message` goes to. Written as a task-id array, the string
/// `stage:<n>`, or the string `running`, so `running` stands alone by construction.
/// An empty array is refused here; the 20-id cap is checked at acceptance.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageTarget {
    Tasks(Vec<String>),
    /// Carried on the wire now and refused at acceptance until milestone 9.1.
    Stage(u32),
    Running,
}

/// `t1,t2`, `stage:3` or `running`: the CLI's recipient argument.
impl fmt::Display for MessageTarget {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            MessageTarget::Tasks(ids) => f.write_str(&ids.join(",")),
            MessageTarget::Stage(n) => write!(f, "{STAGE_PREFIX}{n}"),
            MessageTarget::Running => f.write_str(RUNNING),
        }
    }
}

const RUNNING: &str = "running";
const STAGE_PREFIX: &str = "stage:";

impl Serialize for MessageTarget {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            MessageTarget::Tasks(ids) => ids.serialize(serializer),
            MessageTarget::Stage(n) => serializer.serialize_str(&format!("{STAGE_PREFIX}{n}")),
            MessageTarget::Running => serializer.serialize_str(RUNNING),
        }
    }
}

impl<'de> Deserialize<'de> for MessageTarget {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(TargetVisitor)
    }
}

struct TargetVisitor;

impl<'de> Visitor<'de> for TargetVisitor {
    type Value = MessageTarget;

    fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str("a non-empty array of task ids, \"stage:<n>\" or \"running\"")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<MessageTarget, E> {
        if text == RUNNING {
            return Ok(MessageTarget::Running);
        }
        text.strip_prefix(STAGE_PREFIX)
            .filter(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|n| n.parse().ok())
            .map(MessageTarget::Stage)
            .ok_or_else(|| E::invalid_value(de::Unexpected::Str(text), &self))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<MessageTarget, A::Error> {
        let mut ids = Vec::new();
        while let Some(id) = seq.next_element::<String>()? {
            ids.push(id);
        }
        if ids.is_empty() {
            return Err(de::Error::invalid_length(0, &self));
        }
        Ok(MessageTarget::Tasks(ids))
    }
}
