//! Run types: the plan the orchestrator parses, plan-edit operations, and the state
//! enums a run and its tasks move through.
//!
//! `crates/proto/src/run_info.rs` holds the read-only snapshot types (`RunInfo`,
//! `TaskInfo`, …) and `run_wire.rs` holds the wire messages (`RunRequest`, `RunReply`).
//! Three files, not one, to keep each under AGENTS.md's roughly-600-line guidance.

use serde::{Deserialize, Serialize};

use crate::orch::{MessageKind, MessageTarget};
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
    /// Milestone 8b: a read-only headless scout (`"scout"`).
    Scout,
    /// Milestone 9: a headless sub-planner, one per epic (`"planner"`).
    Planner,
    /// Milestone 9 decision 43: a decider session (`"decider"`), named only by its
    /// role-routing record. A decider has no rounds or tasks and never runs
    /// `anthrex mcp`, so it is never given anthrex tools.
    Decider,
    /// Milestone 9.5 decision 19: a headless worker in one lane of a race (`"racer"`).
    Racer,
    /// Milestone 9.5 decision 24: the headless session that commits a paired task's
    /// failing test (`"test_writer"`).
    TestWriter,
    /// Milestone 9.6 decision 9: a headless brainstormer and document reviewer.
    Brainstormer,
    DocReviewer,
}

/// Identifies one agent round: which run, optionally which task, which role, and which
/// session of that role (a task can be retried, a reviewer re-run, and so on).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunRef {
    pub run_id: String,
    #[serde(default)]
    pub task_id: Option<String>,
    pub role: AgentRole,
    pub session: u32,
    /// Milestone 9.5: a racer's lane; `None` for every other role, and then left out.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<crate::tuning::RaceLane>,
}

pub use crate::effort::Effort;

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
    /// Milestone 9.8 (task M9.8.14): strength is gone; an old plan's or peer's value is
    /// accepted and ignored, and never written.
    #[serde(default, skip_serializing)]
    pub strength: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
}

/// A fully resolved route: every field filled in. A protocol-18 route's `strength`
/// (and an old history line's) is ignored when read (task M9.8.14).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Route {
    pub runtime: Runtime,
    pub model: String,
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
    // Milestone 9.1 decision 5: the tier keys, every one off when absent.
    #[serde(default)]
    pub build_check: Option<String>,
    #[serde(default)]
    pub module_test: Option<String>,
    #[serde(default)]
    pub module_tests: Option<String>,
    #[serde(default)]
    pub module_graph: Option<String>,
    #[serde(default)]
    pub module_names: Option<crate::profile::ModuleNames>,
    #[serde(default)]
    pub full_triggers: Option<Vec<String>>,
    #[serde(default)]
    pub slow_tests: Option<String>,
    #[serde(default)]
    pub timing_tests: Option<String>,
    #[serde(default)]
    pub skip_markers: Option<Vec<String>>,
    #[serde(default)]
    pub test_paths: Option<Vec<String>>,
    #[serde(default)]
    pub full_shards: Option<u8>,
    #[serde(default)]
    pub toolchain_id: Option<String>,
    #[serde(default)]
    pub env: Option<std::collections::BTreeMap<String, String>>,
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
    /// Milestone 9 decision 24: the range a `review` task reviews.
    #[serde(default)]
    pub review_target: Option<String>,
    /// Milestone 9.1 decision 43: the task's stage, 1 to 32.
    #[serde(default = "first_stage")]
    pub stage: u16,
    /// Decision 54: the one hub task a stage may have that changes an interface
    /// non-additively, with its reason.
    #[serde(default)]
    pub atomic: bool,
    #[serde(default)]
    pub atomic_reason: Option<String>,
    /// Milestone 9.2 decision 31: the review threads (`"<pr>:<t|c|r><id>"`) this task
    /// addresses; the engine then makes it a `review` fix task. Left out when empty, so
    /// a plan, a `run.json` and a snapshot without review fixes are written as 9.1's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub addresses: Vec<String>,
    /// Milestone 9.5 decision 17: race two workers on different runtimes. Left out while
    /// false, so a plan, a `run.json` and a snapshot without it are written as 9.3's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub race: bool,
    /// Milestone 9.5 decision 24: a test writer commits the failing test, then a
    /// different worker implements. Left out while false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub pair: bool,
    /// Milestone 9.6 decision 17: the spec requirements (`R4`) it delivers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub covers: Vec<String>,
}

fn default_kind() -> TaskKind {
    TaskKind::Code
}

fn first_stage() -> u16 {
    1
}

/// Milestone 9.1 decision 44: the highest stage a task may name.
pub const STAGES_MAX: u16 = 32;

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
///
/// `AddTask` holds a whole `PlanTask` unboxed: milestone 9.2's `addresses` took it past
/// clippy's `large_enum_variant` line. An edit is built once per request and lives in
/// a short batch, so its size does not matter, and boxing it would change the
/// constructor every client and test uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
#[allow(clippy::large_enum_variant)]
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
        /// Milestone 9 decision 25: replaces the task's dependencies.
        #[serde(default)]
        deps: Option<Vec<String>>,
        /// Milestone 9.1 decision 43: moves a task that has not started.
        #[serde(default)]
        stage: Option<u16>,
        /// Milestone 9.5 decision 17: sets or clears `race` on a task that has not
        /// started.
        #[serde(default)]
        race: Option<bool>,
        /// Milestone 9.5 decision 24: sets or clears `pair` likewise.
        #[serde(default)]
        pair: Option<bool>,
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
    /// Milestone 9 decision 42a: a message to workers, always alone in its call.
    Message {
        to: MessageTarget,
        text: String,
        kind: MessageKind,
    },
    /// Milestone 9 decision 42e: merge the run's latest merged work into a task's
    /// branch at its next turn boundary, always alone in its call.
    Refresh {
        task_id: String,
    },
    /// Milestone 9.2 decision 30: a reply on a stage PR's thread, always alone in its
    /// call.
    ReplyComment {
        pr: u64,
        thread: String,
        body: String,
    },
    /// Milestone 9.3 decision 30: a new round of a settled run, only ever from
    /// `edit_plan`'s `iterate`, alone in its call.
    Iterate {
        goal: String,
    },
}

/// A batch of edits, the shape `anthrex run edit --file` reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditFile {
    #[serde(rename = "edit")]
    pub edits: Vec<PlanEdit>,
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
    /// Milestone 9 decision 26: the orchestrator is writing the plan.
    Planning,
    /// Milestone 9.6 decision 4: the design flow's first two phases.
    Brainstorming,
    Specifying,
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
    /// Milestone 9 decision 35: a research or review task delivered its report; it
    /// merges nothing.
    Reported,
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
    /// Milestone 9 decision 42c: `stop_and_wait`, shown as `paused(message)`.
    MessagePause,
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
            RunState::Planning => "planning",
            RunState::Brainstorming => "brainstorming",
            RunState::Specifying => "specifying",
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
            TaskState::Reported => "reported",
        }
    }

    /// True for a state the task never leaves: merged, cancelled or reported.
    pub fn is_finished(self) -> bool {
        matches!(
            self,
            TaskState::Merged | TaskState::Cancelled | TaskState::Reported
        )
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

#[cfg(test)]
#[path = "run_tests.rs"]
mod tests;
