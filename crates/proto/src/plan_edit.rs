//! The plan-edit operations and the batch file that carries them (moved from
//! `run.rs` in milestone 9.9 before the orchestrator's five ops joined them).

use serde::{Deserialize, Serialize};

use crate::orch::{MessageKind, MessageTarget};
use crate::run::{PlanTask, RouteSpec, Size, TestMode};

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
    /// Milestone 9.9 (OFA §4.2): the orchestrator's `run retry`, alone in its call.
    Retry {
        task_id: String,
        reason: String,
    },
    /// The orchestrator's `run override`.
    Override {
        task_id: String,
        reason: String,
    },
    /// The orchestrator's `run resume` without `--rebaseline`; with `stage`, only that
    /// held stage's tier 3 or push is released.
    ResumeRun {
        reason: String,
        #[serde(default)]
        stage: Option<u16>,
    },
    /// The orchestrator's approval of an awaiting hold.
    ApproveHold {
        hold: String,
        reason: String,
    },
    /// The orchestrator accepts the red tier 3 on stage `stage`'s head.
    AcceptRed {
        stage: u16,
        reason: String,
    },
}

/// A batch of edits, the shape `anthrex run edit --file` reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditFile {
    #[serde(rename = "edit")]
    pub edits: Vec<PlanEdit>,
}
