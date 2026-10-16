//! Milestone 9, the orchestrator and sub-planners
//! (`docs/milestones/M9-orchestrator-and-subplanners.md`). Pure.
//!
//! The model types of decision 1 (Interfaces "daemon"): where an edit batch comes from
//! ([`EditSource`]), the run's orchestrator, epics, approval holds and run scouts
//! ([`RunOrch`]), and a task's hold, messages, notes and refresh ([`TaskOrch`]). Task
//! M9.6 added the fields the digest, the context and the task result read; the tasks
//! that produce them (M9.7 on) fill them. Every field is `#[serde(default)]`.
//!
//! [`OrchLimits`] is `RunLimits.orch`: the orchestrator settings a run is frozen with at
//! start (ruling D-5), so a later config edit cannot change the rules of a live run. The
//! config crate has no serde, so `config::PlannerConfig` cannot be persisted; these are
//! its serde mirrors, of proto types.

use std::collections::BTreeMap;
use std::path::PathBuf;

use proto::{HoldKind, HoldState, MessageKind, Route, TaskNoteKind, TokenUsage};
use serde::{Deserialize, Serialize};

use super::model::OpId;
use crate::scout::report::ScoutReportArgs;

mod attention;
pub mod context;
pub mod contract;
pub mod contract_design;
pub mod contract_rounds;
pub mod digest;
mod epics;
pub mod extract;
pub mod handled;
pub mod installed;
pub(crate) mod json;
pub mod launch;
mod limits;
pub mod plan_frame;
pub mod result;
pub mod roles;
pub mod rules;
#[cfg(test)]
pub(crate) mod test_support;
pub mod tools;
pub mod untrusted;

pub use attention::{START_PROMPT, WAKE_HELD, orchestrator_lines};
pub use epics::{EpicRecord, PlannerPhase, PlannerSession};
pub use limits::{AgentLimits, DesignLimits, OrchLimits, PlannerLimits};

/// Who sent an edit batch. Plan files and the user's `run edit` are [`EditSource::User`]
/// and keep M8a's rules only; the orchestrator's `edit_plan` and a sub-planner's
/// `submit_epic` also meet decision 23's ([`rules::check`]).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EditSource {
    User,
    Orchestrator,
    Planner { epic: String },
}

impl EditSource {
    /// `user`, `orchestrator` or `planner:<e>` (decision 40's edit-log source).
    pub fn label(&self) -> String {
        match self {
            EditSource::User => "user".into(),
            EditSource::Orchestrator => "orchestrator".into(),
            EditSource::Planner { epic } => format!("planner:{epic}"),
        }
    }
}

/// `Run.orch`: a run's milestone 9 state. Absent from an older run: empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunOrch {
    /// The run's orchestrator window (decisions 4, 11), once it exists.
    pub orchestrator: Option<OrchestratorRecord>,
    /// Every epic the orchestrator created with `spawn_subplanner`, in creation order.
    pub epics: Vec<EpicRecord>,
    /// Decision 28's approval holds, in creation order.
    pub gate_holds: Vec<GateHoldRecord>,
    /// Decision 20's run scouts, in the order they were asked for.
    pub run_scouts: Vec<RunScout>,
    /// Decision 16: the revision `run_status` waits on, bumped only when
    /// [`digest::fingerprint`] changes, and the fingerprint it was bumped for.
    pub digest_rev: u64,
    pub digest_fp: u64,
    /// When the orchestrator last read the digest (`OrchEvent::DigestRead`): the
    /// digest's `gate.holds` keeps a hold decided since then. `None`: never read.
    pub digest_read_at: Option<u64>,
    /// Decision 17: whether each runtime's configured binary was found when the run
    /// was built, keyed by `Runtime::label`.
    pub installed: BTreeMap<String, bool>,
    /// The goal request's `yes`: a submitted plan starts at once (decisions 26, 27).
    pub yes: bool,
    pub research_report: Option<PathBuf>,
    pub planner_usage: TokenUsage,
    /// The last [`WorkerNote::seq`] handed out by [`add_worker_note`].
    pub note_seq: u64,
    /// Whole-branch fix round 2, item 2: a wake-up waits only because the orchestrator's
    /// window was at a prompt (`WindowManager::attention_open`); [`WAKE_HELD`] shows it.
    /// In memory only: the driver reports it again after a restart.
    #[serde(skip)]
    pub wake_held: bool,
    /// Milestone 9.3 decision 11: a round's (or a next goal's) wake, delivered whole,
    /// held until the orchestrator is woken with it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_wake: Option<String>,
    /// Milestone 9.5 decision 38, in memory only, reset by every launch: `McpReady` came;
    /// since when a pending first turn waits; whether past `FIRST_TURN_WAIT_SECS`; when
    /// the window first signalled while it waited (ruling T5a-1's grace).
    #[serde(skip)]
    pub mcp_ready: bool,
    #[serde(skip)]
    pub first_turn_since: Option<u64>,
    #[serde(skip)]
    pub first_turn_late: bool,
    #[serde(skip)]
    pub first_signal_at: Option<u64>,
    /// Decision 39, in memory only: the driver reports the window at a start prompt.
    #[serde(skip)]
    pub start_prompt: bool,
    /// Whole-branch review B, M8, in memory only: what `run promote`'s check found the
    /// orchestrator's window has installed, which the promotion reads its `orchestrator`
    /// list over (taken by the promotion).
    #[serde(skip)]
    pub promote_window: Option<BTreeMap<String, bool>>,
    /// Milestone 9.6 decision 2: a design run's state; `None` (and not written) for a
    /// run without the design flow, which behaves as 9.5's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub design: Option<crate::run::design::state::DesignState>,
    /// Milestone 9.9 decision 12: what the orchestrator resolved on its own (newest last,
    /// at most `handled::HANDLED_KEPT`) and how many in all.
    /// Not written while empty, so a run that never used the ops keeps its old `run.json`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub handled: Vec<handled::HandledRecord>,
    #[serde(skip_serializing_if = "is_zero")]
    pub handled_total: u32,
    /// Milestone 9.9 decision 19: the highest wake-note seq the orchestrator had seen
    /// when it last called a tool or read the digest; since when a note beyond it has
    /// waited; and when that wait passed `stall_after_secs`.
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub acted_seq: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub waiting_since: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stalled_at: Option<u64>,
    /// Milestone 9.9 decision 15: the orchestrator's pending `ask_user`, and the last
    /// id handed out. Neither is written while empty.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ask: Option<AskRecord>,
    #[serde(skip_serializing_if = "is_zero_u64")]
    pub ask_seq: u64,
    /// Milestone 9.13 decision 18: the session read text from outside the run; the
    /// first such read, kept for the session's life. Not written while `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub untrusted: Option<untrusted::UntrustedRecord>,
    /// Milestone 9.16 decision 29 (ruling C8), in memory only: the live orchestrator's
    /// last measured context fill, for its attention line.
    #[serde(skip)]
    pub fill: Option<crate::transcript::fill::Fill>,
    /// Milestone 9.17 decision 18: each round's plan frame, at most one per round.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_frames: Vec<plan_frame::PlanFrame>,
}

/// Milestone 9.9 decision 15: a question the orchestrator put to the user. The text is
/// the orchestrator's, kept as given (bounded by the tool's parse); every client
/// sanitises where it draws.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskRecord {
    pub id: u64,
    pub question: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub context: String,
    pub asked_at: u64,
    /// Milestone 9.13 decision 21: the tainted orchestrator's op the user decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<proto::PlanEdit>,
}

fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}

/// `Task.orch`: a task's milestone 9 state. Absent from an older run: empty.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TaskOrch {
    /// Decision 28: the approval hold the task was added under.
    pub gate_hold: Option<String>,
    /// Decision 35: a research task's report.
    pub research: Option<ScoutReportArgs>,
    /// Decision 36: a review task's resolved `(base, head)`.
    pub review_range: Option<(String, String)>,
    /// Decision 37: the epic an engine-made integration review task reviews.
    pub integration_of: Option<String>,
    /// Decision 42d: every accepted `message` to the task, and its `task_note`s.
    pub messages: Vec<TaskMessage>,
    pub worker_notes: Vec<WorkerNote>,
    /// Decision 42e: a refresh due or in flight, and the merge commits refreshes made.
    pub refresh: Option<RefreshState>,
    pub refresh_merges: Vec<String>,
    /// The run heads refreshes merged in, clean or conflicted, each once (M9.13a
    /// re-review): what they reach is run work, never the task's.
    pub refresh_targets: Vec<String>,
    /// Decision 25's restarts of the task by a rewrite (M9.9 review fixes, M2).
    pub rewrite_restarts: u32,
    /// Milestone 9.13 decision 14: the orchestrator's retries of the task since the user
    /// last retried it; at `MAX_ORCH_RETRIES` the task's next block is the user's.
    #[serde(skip_serializing_if = "is_zero_u32")]
    pub orch_retries: u32,
    /// Text the next research session's first turn ends with (a worker's
    /// `FreshSession::append`, M9.9 second review, M-d).
    pub research_append: Option<String>,
}

/// The run's orchestrator (decision 1). `otlp_token` never reaches the snapshot or
/// the digest (decision 14a).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrchestratorRecord {
    pub route: Route,
    pub window_id: Option<u32>,
    pub launch_op: Option<OpId>,
    pub live: bool,
    pub started_at: u64,
    pub exited_at: Option<u64>,
    pub first_prompt: String,
    pub plan_submitted: bool,
    /// Decision 19's `summary`, the last one given.
    pub summary: Option<String>,
    /// Decision 39's pending wake notes, oldest first, at most 20.
    pub notes: Vec<String>,
    /// Each note's seq, one per note of `notes`: a `DigestRead` or
    /// `OrchestratorWoken` drops the notes up to the seq its answer included.
    #[serde(default)]
    pub note_seqs: Vec<u64>,
    /// The last seq handed to a note.
    #[serde(default)]
    pub last_note_seq: u64,
    pub last_wake_rev: u64,
    pub wakes: u32,
    pub otlp_token: String,
    /// Decision 43: 1 at launch, +1 per restart.
    pub session: u32,
    /// Decision 13: why the last `CreateOrchestrator` failed, until one succeeds.
    #[serde(default)]
    pub start_error: Option<String>,
    /// M9.13 review: +1 with every launch or restart that succeeded. The driver's
    /// report of the window carries the count it saw, so a report older than the last
    /// restart is dropped.
    #[serde(default)]
    pub launches: u64,
    /// Decision 43: its route's resolution, which every launch's record keeps.
    #[serde(default)]
    pub routing: roles::RoleSnapshot,
    /// Milestone 9.5 decision 37: the adopted session's usage counter when this run
    /// took its window (`chains::adopt`); the OTLP totals count from it. Zero for a
    /// session the run launched, and again after a restore (the ledger restarts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_at_adopt: Option<TokenUsage>,
    /// Milestone 9.5 decision 38: `first_prompt` waits to be pasted as the first turn.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub first_turn_pending: bool,
}

impl OrchestratorRecord {
    /// Decision 13's attention line: the launch failed, or the window exited while the
    /// run goes on (a finished run's window is a plain one, decision 30).
    pub fn attention(&self, terminal: bool) -> Option<String> {
        if let Some(error) = &self.start_error {
            return Some(format!("the orchestrator could not start: {error}"));
        }
        let n = self.window_id?;
        (!terminal && !self.live && self.exited_at.is_some()).then(|| {
            format!("the orchestrator (window {n}) exited; restart it with anthrex restart {n}")
        })
    }
}

/// Decision 28's approval hold, as the daemon keeps it (the wire's is `HoldInfo`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateHoldRecord {
    pub id: String,
    pub kind: HoldKind,
    pub state: HoldState,
    pub tasks: Vec<String>,
    pub created_at: u64,
    pub decided_at: Option<u64>,
    pub decided_by: Option<String>,
}

/// A run scout's state (decision 20). `Queued` has no window yet.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunScoutState {
    Queued,
    Running,
    Reported,
    Failed { reason: String },
}

impl RunScoutState {
    /// `queued`, `running`, `reported` or `failed`: the digest's labels.
    pub fn label(&self) -> &'static str {
        match self {
            RunScoutState::Queued => "queued",
            RunScoutState::Running => "running",
            RunScoutState::Reported => "reported",
            RunScoutState::Failed { .. } => "failed",
        }
    }
}

/// One run scout (decision 20); `id` is the full `<h4>-<id>`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunScout {
    pub id: String,
    pub question: String,
    pub area: Vec<String>,
    pub web: bool,
    pub state: RunScoutState,
    pub queued_at: u64,
    pub started_at: Option<u64>,
    pub ended_at: Option<u64>,
    pub window_id: Option<u32>,
}

/// Decision 42f: one `task_note` of a task's worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerNote {
    pub at: u64,
    pub kind: TaskNoteKind,
    pub text: String,
    /// The run-wide order the note was added in (1, 2, …; 0 for a note added before
    /// it existed): the digest breaks a tie on `at` by it, newest first.
    #[serde(default)]
    pub seq: u64,
    /// Milestone 9.5 decision 19 (ruling RR-4): the race lane whose racer wrote it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lane: Option<proto::RaceLane>,
}

/// Appends `note` to task `task`'s notes with the next run-wide `seq`; false when
/// the run has no such task. Every `task_note` goes through here (M9.13a), so a note
/// added in the same second as others still reaches the digest's newest ten.
pub fn add_worker_note(run: &mut super::model::Run, task: &str, mut note: WorkerNote) -> bool {
    let Some(i) = run.tasks.iter().position(|t| t.id() == task) else {
        return false;
    };
    run.orch.note_seq += 1;
    note.seq = run.orch.note_seq;
    run.tasks[i].orch.worker_notes.push(note);
    true
}

/// Decision 42e: a refresh waiting for the turn boundary, or its `HandBack` op.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefreshState {
    Due,
    InFlight(OpId),
}

/// Decision 42d: one accepted `message` to a task, kept across sessions for its prompts
/// ([`contract::notes_section`]). Task M9.13a records them in `Task.orch`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskMessage {
    pub at: u64,
    pub source: EditSource,
    pub kind: MessageKind,
    pub text: String,
    pub delivered: bool,
    /// The outbox message carrying it to a live worker (task M9.13a), `None` when it
    /// is only recorded for the next session's prompt.
    #[serde(default)]
    pub outbox: Option<u64>,
}

/// Decision 26: a run `RunService::build_plan` built from an empty plan becomes a
/// planned run, `planning` with its orchestrator (decision 6's resolution, kept for
/// decision 43's records) not yet launched. `yes` applies at submit (decision 27), so
/// the build's own `yes` is false. Milestone 9.3 (D14): a continued goal has no triage
/// and is on the plan path.
pub fn make_planned(
    run: &mut super::model::Run,
    triage: Option<proto::TriageInfo>,
    resolved: launch::Resolved,
    yes: bool,
    installed: BTreeMap<String, bool>,
) {
    run.state = proto::RunState::Planning;
    run.path = Some(triage.as_ref().map_or(proto::RunPath::Plan, |t| t.path));
    run.triage = triage;
    run.orch.yes = yes;
    run.orch.installed = installed;
    let mut record = OrchestratorRecord::new(resolved.route, run.created_at);
    record.routing = roles::RoleSnapshot {
        source: resolved.source,
        candidates: resolved.candidates,
    };
    run.orch.orchestrator = Some(record);
    // Milestone 9.6 decision 4: a design run brainstorms first.
    if run.design_mode == proto::DesignMode::Full {
        crate::run::engine::design::enter(run);
    }
}

impl OrchestratorRecord {
    /// A record for an orchestrator about to be launched: no window yet, session 1.
    /// The driver fills `otlp_token` (decision 14a); the engine the first prompt.
    pub fn new(route: Route, now: u64) -> OrchestratorRecord {
        OrchestratorRecord {
            route,
            window_id: None,
            launch_op: None,
            live: false,
            started_at: now,
            exited_at: None,
            first_prompt: String::new(),
            plan_submitted: false,
            summary: None,
            notes: Vec::new(),
            note_seqs: Vec::new(),
            last_note_seq: 0,
            last_wake_rev: 0,
            wakes: 0,
            otlp_token: String::new(),
            session: 1,
            start_error: None,
            launches: 0,
            routing: roles::RoleSnapshot::default(),
            usage_at_adopt: None,
            first_turn_pending: false,
        }
    }
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

fn is_zero_u64(n: &u64) -> bool {
    *n == 0
}
