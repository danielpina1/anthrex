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

use proto::{
    Effort, HoldKind, HoldState, IntegrationState, MessageKind, Route, Runtime, Strength,
    TaskNoteKind, TokenUsage,
};
use serde::{Deserialize, Serialize};

use super::model::OpId;
use crate::scout::report::ScoutReportArgs;

pub mod context;
pub mod contract;
pub mod digest;
pub mod extract;
pub(crate) mod json;
pub mod launch;
pub mod result;
pub mod rules;
#[cfg(test)]
pub(crate) mod test_support;
pub mod tools;

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
    /// The digest revision each note of `notes` was added at, one per note: a
    /// `DigestRead` or `OrchestratorWoken` of a revision drops the notes up to it.
    #[serde(default)]
    pub note_revs: Vec<u64>,
    pub last_wake_rev: u64,
    pub wakes: u32,
    pub otlp_token: String,
    /// Decision 43: 1 at launch, +1 per restart.
    pub session: u32,
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
}

/// A sub-planner's phase (decision 32). `Queued` and `Planning` are live.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlannerPhase {
    #[default]
    Queued,
    Planning,
    Finished,
    Failed {
        reason: String,
    },
}

impl PlannerPhase {
    /// Queued or planning: the epic is its sub-planner's (decision 23.5).
    pub fn is_live(&self) -> bool {
        matches!(self, PlannerPhase::Queued | PlannerPhase::Planning)
    }
}

/// One epic and its sub-planner sessions (decisions 31–33, 37).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpicRecord {
    pub epic: String,
    pub title: String,
    pub area: Vec<String>,
    pub brief: String,
    pub scout_refs: Vec<String>,
    pub route: Route,
    pub phase: PlannerPhase,
    /// The live or last session's brief.
    pub request: String,
    pub sessions: Vec<PlannerSession>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub edits_accepted: u32,
    pub edits_rejected: u32,
    pub last_rejection: Option<String>,
    pub replans: Vec<String>,
    pub note: Option<String>,
    pub gate_hold: Option<String>,
    pub base: Option<String>,
    /// `(task id, merge commit)`.
    pub merges: Vec<(String, String)>,
    pub integration_state: IntegrationState,
    pub integration_rounds: u32,
    /// How many of `merges` the latest integration review covers: a round is due only
    /// once a merge came after it (decision 37).
    #[serde(default)]
    pub integration_reviewed: u32,
}

impl EpicRecord {
    /// A new epic as `spawn_subplanner` asks for it (decision 21), its sub-planner
    /// queued; the engine sets its route and start time.
    pub fn requested(
        epic: &str,
        title: &str,
        area: Vec<String>,
        brief: &str,
        scout_refs: Vec<String>,
    ) -> EpicRecord {
        EpicRecord {
            epic: epic.into(),
            title: title.into(),
            area,
            brief: brief.into(),
            scout_refs,
            route: Route {
                runtime: Runtime::Claude,
                model: String::new(),
                strength: Strength::Frontier,
                effort: Effort::High,
            },
            phase: PlannerPhase::Queued,
            request: brief.into(),
            sessions: Vec::new(),
            started_at: 0,
            ended_at: None,
            edits_accepted: 0,
            edits_rejected: 0,
            last_rejection: None,
            replans: Vec::new(),
            note: None,
            gate_hold: None,
            base: None,
            merges: Vec::new(),
            integration_state: IntegrationState::default(),
            integration_rounds: 0,
            integration_reviewed: 0,
        }
    }
}

/// One sub-planner session of an epic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannerSession {
    pub session: u32,
    pub window_id: Option<u32>,
    pub op: Option<OpId>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub usage: TokenUsage,
    /// The session's rejected `submit_epic` batches (decision 22's `max_rejections`).
    #[serde(default)]
    pub rejections: u32,
}

#[cfg(test)]
impl EpicRecord {
    /// A test epic `epic` in `phase`, with no sessions.
    pub fn new(epic: &str, phase: PlannerPhase) -> EpicRecord {
        EpicRecord {
            epic: epic.into(),
            title: format!("Epic {epic}"),
            area: vec![format!("crates/{epic}/**")],
            brief: String::new(),
            scout_refs: Vec::new(),
            route: Route {
                runtime: Runtime::Claude,
                model: "claude-opus-5".into(),
                strength: Strength::Frontier,
                effort: Effort::High,
            },
            phase,
            request: String::new(),
            sessions: Vec::new(),
            started_at: 0,
            ended_at: None,
            edits_accepted: 0,
            edits_rejected: 0,
            last_rejection: None,
            replans: Vec::new(),
            note: None,
            gate_hold: None,
            base: None,
            merges: Vec::new(),
            integration_state: IntegrationState::default(),
            integration_rounds: 0,
            integration_reviewed: 0,
        }
    }
}

/// `[orchestrator]`'s milestone 9 keys, as the run was built with them. Absent from a
/// run recorded before milestone 9: the config defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct OrchLimits {
    pub planner_task_cap: u32,
    pub max_scouts: u32,
    pub wake_orchestrator: bool,
    pub wake_quiet_secs: u64,
    /// Zero turns messages off.
    pub message_max_per_turn: u32,
    /// Zero turns notes off.
    pub note_max_per_task: u32,
    pub planners: PlannerLimits,
    /// `[orchestrator.agent]`, frozen too (task M9.7): `run promote` and a promotion
    /// recorded before milestone 9 resolve the orchestrator's route from the run alone.
    pub agent: AgentLimits,
}

/// `[orchestrator.agent]`: `runtime` `None` means `default_runtime`; an empty `model`
/// means decision 6's resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AgentLimits {
    pub runtime: Option<Runtime>,
    pub model: String,
    pub effort: Effort,
}

impl AgentLimits {
    /// The `config::AgentConfig` these limits were frozen from.
    pub fn config(&self) -> config::AgentConfig {
        config::AgentConfig {
            runtime: self.runtime,
            model: self.model.clone(),
            effort: self.effort,
        }
    }
}

/// `[orchestrator.planners]`. `runtime` `None` means the orchestrator's runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlannerLimits {
    pub runtime: Option<Runtime>,
    pub strength: Strength,
    pub effort: Effort,
    pub max_tool_calls: u32,
    pub timeout_secs: u64,
    pub max_rejections: u32,
}

impl OrchLimits {
    /// The limits `[orchestrator]` gives a run built now.
    pub fn from_config(config: &config::Orchestrator) -> OrchLimits {
        let a = &config.agent;
        let p = &a.planners;
        OrchLimits {
            planner_task_cap: a.planner_task_cap,
            max_scouts: a.max_scouts,
            wake_orchestrator: a.wake_orchestrator,
            wake_quiet_secs: a.wake_quiet_secs,
            message_max_per_turn: a.message_max_per_turn,
            note_max_per_task: a.note_max_per_task,
            planners: PlannerLimits {
                runtime: p.runtime,
                strength: p.strength,
                effort: p.effort,
                max_tool_calls: p.max_tool_calls,
                timeout_secs: p.timeout_secs,
                max_rejections: p.max_rejections,
            },
            agent: AgentLimits {
                runtime: a.agent.runtime,
                model: a.agent.model.clone(),
                effort: a.agent.effort,
            },
        }
    }
}

impl Default for OrchLimits {
    fn default() -> Self {
        OrchLimits::from_config(&config::Orchestrator::default())
    }
}

impl Default for AgentLimits {
    fn default() -> Self {
        OrchLimits::default().agent
    }
}

impl Default for PlannerLimits {
    fn default() -> Self {
        OrchLimits::default().planners
    }
}

/// Decision 26: a run `RunService::build_plan` built from an empty plan becomes a
/// planned run, `planning` with its orchestrator (the route of decision 6) not yet
/// launched. `yes` applies at submit (decision 27), so the build's own `yes` is false.
pub fn make_planned(
    run: &mut super::model::Run,
    triage: proto::TriageInfo,
    route: Route,
    yes: bool,
    installed: BTreeMap<String, bool>,
) {
    run.state = proto::RunState::Planning;
    run.path = Some(triage.path);
    run.triage = Some(triage);
    run.orch.yes = yes;
    run.orch.installed = installed;
    run.orch.orchestrator = Some(OrchestratorRecord::new(route, run.created_at));
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
            note_revs: Vec::new(),
            last_wake_rev: 0,
            wakes: 0,
            otlp_token: String::new(),
            session: 1,
        }
    }
}
