//! Run types: the plan the orchestrator parses, plan-edit operations, and the state
//! enums a run and its tasks move through.
//!
//! `crates/proto/src/run_info.rs` holds the read-only snapshot types (`RunInfo`,
//! `TaskInfo`, …) and `run_wire.rs` holds the wire messages (`RunRequest`, `RunReply`).
//! Three files, not one, to keep each under AGENTS.md's roughly-600-line guidance.

use serde::{Deserialize, Serialize};

use crate::types::Runtime;

/// Who is running an agent round. `Orchestrator` is used from M9; `Worker` and
/// `Reviewer` are used from this milestone.
///
/// `snake_case`, not `lowercase`: the three variants existing today serialize the same
/// either way (`"orchestrator"`, `"worker"`, `"reviewer"`), but a later multi-word
/// variant (M9.5's `TestWriter`) then serializes as `"test_writer"` instead of
/// `"testwriter"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentRole {
    Orchestrator,
    Worker,
    Reviewer,
}

/// Identifies one agent round: which run, optionally which task, which role, and which
/// session of that role (a task can be retried, a reviewer re-run, and so on).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRef {
    pub run_id: String,
    pub task_id: Option<String>,
    pub role: AgentRole,
    pub session: u32,
}

/// How capable a route's model should be. Ordered: a task can only be "raised", never
/// lowered, so the ordering matters, not just the set of values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Strength {
    Fast,
    Standard,
    Frontier,
}

/// How much reasoning effort a route asks the model for. Ordered for the same reason as
/// [`Strength`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
}

/// How large a task is expected to be. Serializes as a capital letter (`"S"`, `"M"`,
/// `"L"`), not a word, because the plan author writes it that way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Size {
    S,
    M,
    L,
}

/// What kind of work a task is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskKind {
    Code,
    Docs,
    Research,
    Review,
}

/// Whether a task's worker writes a failing test before implementing, only checks, or
/// does neither.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TestMode {
    Tdd,
    Check,
    None,
}

/// A plan-file `[task.route]` table: every field optional, filled in by policy defaults
/// when absent.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteSpec {
    #[serde(default)]
    pub runtime: Option<Runtime>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub strength: Option<Strength>,
    #[serde(default)]
    pub effort: Option<Effort>,
}

/// A fully resolved route: every field filled in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub runtime: Runtime,
    pub model: String,
    pub strength: Strength,
    pub effort: Effort,
}

/// A task's resource ceiling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    pub tool_calls: u32,
    pub minutes: u32,
    /// Decision 40: an optional token ceiling, on top of the always-present tool-call
    /// and wall-clock ones.
    #[serde(default)]
    pub tokens: Option<u64>,
}

/// A plan-file `[profile]` table: every field optional, added to (never replacing) the
/// resolved profile's built-in defaults.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileSpec {
    #[serde(default)]
    pub modules: Option<Vec<String>>,
    #[serde(default)]
    pub hub: Option<Vec<String>>,
    #[serde(default)]
    pub source: Option<Vec<String>>,
    #[serde(default)]
    pub check: Option<String>,
    #[serde(default)]
    pub check_timeout_secs: Option<u64>,
    #[serde(default)]
    pub single_test: Option<String>,
    #[serde(default)]
    pub test_passed: Option<String>,
    #[serde(default)]
    pub setup: Option<String>,
    #[serde(default)]
    pub generated: Option<Vec<String>>,
    /// Added to the built-in protected paths (decision 56); a plan can never remove a
    /// built-in.
    #[serde(default)]
    pub protected: Option<Vec<String>>,
    #[serde(default)]
    pub env: Option<std::collections::BTreeMap<String, String>>,
    /// Directories the engine's confined checks and proofs may write besides the
    /// checkout and its temporary directory (M8a final fix batch F1c, I2): a build
    /// cache, for example. `~/` is the daemon's `$HOME`.
    #[serde(default)]
    pub cache_dirs: Option<Vec<String>>,
}

/// One `[[task]]` table in a plan file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlanTask {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub epic: Option<String>,
    #[serde(default = "default_kind")]
    pub kind: TaskKind,
    pub size: Size,
    #[serde(default)]
    pub interface_change: bool,
    #[serde(default)]
    pub test_mode: Option<TestMode>,
    #[serde(default)]
    pub test_mode_reason: Option<String>,
    pub owns: Vec<String>,
    #[serde(default)]
    pub deps: Vec<String>,
    #[serde(default)]
    pub priority: i32,
    pub brief: String,
    pub acceptance: Vec<String>,
    #[serde(default)]
    pub test_to_write: Option<String>,
    #[serde(default)]
    pub scout_refs: Vec<String>,
    #[serde(default)]
    pub route: RouteSpec,
    #[serde(default)]
    pub budget: Option<Budget>,
}

fn default_kind() -> TaskKind {
    TaskKind::Code
}

/// A whole plan file, parsed with `toml::from_str`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Plan {
    pub goal: String,
    #[serde(default)]
    pub max_writers: Option<u8>,
    #[serde(default)]
    pub max_readers: Option<u8>,
    #[serde(default)]
    pub max_bounces: Option<u8>,
    #[serde(default)]
    pub profile: ProfileSpec,
    #[serde(rename = "task")]
    pub tasks: Vec<PlanTask>,
}

/// One edit a running plan can be given, either interactively (`run edit`) or in a
/// batch file (`run edit --file`, [`EditFile`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlanEdit {
    AddTask {
        task: PlanTask,
    },
    SplitTask {
        task_id: String,
        into: Vec<PlanTask>,
    },
    CancelTask {
        task_id: String,
    },
    AmendTask {
        task_id: String,
        #[serde(default)]
        brief: Option<String>,
        #[serde(default)]
        acceptance: Option<Vec<String>>,
        #[serde(default)]
        route: Option<RouteSpec>,
        #[serde(default)]
        test_mode: Option<TestMode>,
        #[serde(default)]
        test_mode_reason: Option<String>,
        #[serde(default)]
        priority: Option<i32>,
        #[serde(default)]
        size: Option<Size>,
    },
    AddDep {
        task_id: String,
        dep: String,
    },
    Answer {
        task_id: String,
        text: String,
    },
    Pause,
    Resume,
    Finish,
}

/// A batch of edits, the shape `anthrex run edit --file` reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditFile {
    #[serde(rename = "edit")]
    pub edits: Vec<PlanEdit>,
}

/// One row of the policy's model table.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEntry {
    pub runtime: Runtime,
    pub model: String,
    pub strength: Strength,
    pub note: String,
}

/// The lifecycle a run moves through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    AwaitingApproval,
    Running,
    Paused,
    Halted,
    Complete,
    Accepted,
    Discarded,
    Failed,
}

/// The lifecycle a task moves through.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pending,
    Queued,
    Preparing,
    Working,
    Proof,
    Check,
    Review,
    MergeQueue,
    Merged,
    Blocked,
    Cancelled,
}

/// Why a task is [`TaskState::Blocked`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BlockReason {
    MisSized,
    Human,
    Conflict,
    DepCancelled,
    Question,
    Environment,
}

/// Which gate a bounce count belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateKind {
    Done,
    Proof,
    Check,
    Review,
    Merge,
}

/// How many times each gate has bounced a task back.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateCounts {
    pub done: u8,
    pub proof: u8,
    pub check: u8,
    pub review: u8,
    pub merge: u8,
}

/// A reviewer's overall verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    Approve,
    Changes,
}

/// How serious a review finding is. Ordered: `Critical` is the most severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Critical,
    Important,
    Minor,
}

/// One line of review feedback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub severity: Severity,
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub line: Option<u32>,
    #[serde(default)]
    pub input: Option<String>,
    pub text: String,
}

/// How a task's `done` gate was reached: the agent called `task_done`, or its turn
/// simply ended and the engine fell back to checking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoneSignal {
    TaskDone,
    TurnEndFallback,
}

/// What `run finish` does with a completed run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FinishAction {
    Accept,
    Discard,
}

impl RunState {
    pub fn label(self) -> &'static str {
        match self {
            RunState::AwaitingApproval => "awaiting_approval",
            RunState::Running => "running",
            RunState::Paused => "paused",
            RunState::Halted => "halted",
            RunState::Complete => "complete",
            RunState::Accepted => "accepted",
            RunState::Discarded => "discarded",
            RunState::Failed => "failed",
        }
    }

    /// True for a state the run never leaves on its own.
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            RunState::Accepted | RunState::Discarded | RunState::Failed
        )
    }
}

impl TaskState {
    pub fn label(self) -> &'static str {
        match self {
            TaskState::Pending => "pending",
            TaskState::Queued => "queued",
            TaskState::Preparing => "preparing",
            TaskState::Working => "working",
            TaskState::Proof => "proof",
            TaskState::Check => "check",
            TaskState::Review => "review",
            TaskState::MergeQueue => "merge_queue",
            TaskState::Merged => "merged",
            TaskState::Blocked => "blocked",
            TaskState::Cancelled => "cancelled",
        }
    }

    /// True for a state the task never leaves: merged or cancelled.
    pub fn is_finished(self) -> bool {
        matches!(self, TaskState::Merged | TaskState::Cancelled)
    }
}

impl Size {
    /// Raises a size one step: `S` → `M`, `M` → `L`, `L` stays `L`.
    pub fn raised(self) -> Size {
        match self {
            Size::S => Size::M,
            Size::M => Size::L,
            Size::L => Size::L,
        }
    }
}

impl Effort {
    /// Raises an effort one step, or `None` when already at the top (`High`).
    pub fn raised(self) -> Option<Effort> {
        match self {
            Effort::Low => Some(Effort::Medium),
            Effort::Medium => Some(Effort::High),
            Effort::High => None,
        }
    }
}

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
