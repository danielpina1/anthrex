//! The design flow's state on a run, `RunOrch.design` (brief decision 2), and the index
//! of its documents (decision 12). Pure: the engine records a version here and asks the
//! driver to write its file with [`Effect::WriteDoc`]; it never touches a file itself.
//!
//! Versions are immutable (DF §5.3, §12): [`store`] always takes the next number of its
//! kind, and the driver refuses to write a file that exists.

use std::path::PathBuf;

use proto::{AgentRole, DocFinding, DocGateKind, DocKind, Route, RunState};
use serde::{Deserialize, Serialize};

pub use super::pack::FrozenPack;
pub use super::requirements::Requirement;
pub use super::round::{ApprovedSpec, DesignRound};
pub use super::spend::{AgentSpend, PhaseSpend};
pub use super::versions::{
    DocVersion, LABELS, NewDoc, findings_name, index_text, sha256_hex, store, store_findings,
};
use crate::run::model::Run;

/// The design documents' folder, `<data>/runs/<id>/design/`.
pub const DESIGN_DIR: &str = "design";
/// The index the driver mirrors from [`DesignState::versions`].
pub const VERSIONS_FILE: &str = "versions.json";

/// A design run's state. Times are unix seconds, as everywhere in the engine.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DesignState {
    /// When the current phase's clock started (decision 8); `None` while it is stopped.
    pub phase_started: Option<u64>,
    /// `Run::paused_total` when the clock started, as `TaskPaused.base`.
    pub phase_paused_base: u64,
    /// The user's answers, given to the brainstormers (`start_brainstorm`).
    pub answers: Option<String>,
    pub brainstormers: Vec<DesignAgent>,
    pub reviewer: Option<DesignAgent>,
    pub reviews: Vec<DocReviewRecord>,
    /// Every stored version, in the order stored: the index `versions.json` mirrors.
    pub versions: Vec<DocVersion>,
    pub gate: Option<DocGate>,
    /// The approved spec's requirements only (decision 12).
    pub requirements: Vec<Requirement>,
    /// The approved spec's Goal section, at most 8 KiB.
    pub goal_section: String,
    pub plan_review_done: bool,
    /// The documents commit is due (decision 23); worktrees wait while it is.
    pub commit_due: bool,
    /// The documents commit's sha, once made.
    pub committed: Option<String>,
    /// The committed spec's path in the repository: a stage PR names it (decision 31).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spec_path: Option<String>,
    /// The state a phase-budget halt left (decision 8): `run resume` returns to it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub halted_from: Option<RunState>,
    /// The brainstorm's rethinks so far (brief ruling BD-2: at most 3).
    #[serde(skip_serializing_if = "is_zero")]
    pub rethinks: u32,
    /// The latest text of each gate document, `(kind, n, text)`: what the next version's
    /// change summary compares against; and of each brainstormer's latest draft, which
    /// the merged report's appendix attaches (task M9.6.9). In memory only; a restore
    /// refills it from the files (`driver/design_restore.rs`), and without it a summary
    /// is empty and an appendix names the draft as unread.
    #[serde(skip)]
    pub texts: Vec<(DocKind, u32, String)>,
    /// Ruling T8-1: each brainstormer's accepted draft, `(label, text)`, held until
    /// both brainstormers have ended, then stored and written (`design_agents::settle`).
    /// In memory only, so no draft's file exists, and `get_doc` finds none, while the
    /// other brainstormer runs; a restore relaunches its brainstormer instead.
    #[serde(skip)]
    pub held: Vec<(String, String)>,
    /// Ruling WB-A-W1: the document reviewer's findings, taken while the run was halted
    /// or paused in a design phase, applied when it resumes. In memory only, as
    /// [`DesignState::held`]: a restore relaunches the reviewer instead.
    #[serde(skip)]
    pub held_findings: Option<Vec<proto::DocFinding>>,
    /// Ruling T9-1a: brainstorm drafts a restore could not read back, `(n, reason)`.
    /// Such a draft is attached as unread; its brainstormer's outcome is unchanged. In
    /// memory only, as [`DesignState::texts`]: the next restore reads it again.
    #[serde(skip)]
    pub unread: Vec<(u32, String)>,
    /// Ruling T9-2(c): a merged report was refused while a restore's read-back of the
    /// drafts was pending; the orchestrator is woken once it lands. In memory only.
    #[serde(skip)]
    pub read_back_owed: bool,
    /// Ruling T8-5: this brainstorm round's drafts are in (`design::drafts_in` ran), so
    /// the brainstorm never settles again until brainstormers are queued anew.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub drafts_settled: bool,
    /// Ruling T8-2: the brainstormers' pack inputs, frozen when they were queued.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pack: Option<FrozenPack>,
    /// The spec version the user approved (task M9.6.10): its requirements and Goal
    /// section are stored once the driver has read its text back.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub approved_spec: Option<u32>,
    /// The approved spec's Interfaces section, at most 8 KiB: a sub-planner's first turn
    /// carries it with the Goal (decision 22, task M9.6.11).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub interfaces_section: String,
    /// Ruling T10-3: the approved spec's read-back failed once; it is read again when the
    /// run resumes or restores, and a second failure halts the run.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub spec_unread: bool,
    /// The note of a plan revision whose gate a sub-planner left for planning (task
    /// M9.6.11): the stale gate is cleared, and the next plan version keeps the note.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_revision: Option<String>,
    /// Ruling T11-3 (m1): each live task's size, route and test mode when the open plan
    /// gate's engine pass last rendered `plan.md`. In memory only.
    #[serde(skip)]
    pub plan_fingerprint: Option<String>,
    /// Decision 32 and ruling T13-1 (task M9.6.13): each phase's spend, per round, for
    /// its history record and REPORT.md's spend line.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub spend: Vec<PhaseSpend>,
    /// Ruling T13-1: the brainstormers whose next session starts a rethink's round,
    /// routed with trigger `rethink`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub rethink_starts: Vec<String>,
    /// Task M9.6.15 (decision 28): the current round's design, from round 2 on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub round: Option<DesignRound>,
    /// Task M9.6.15: the last round whose documents were committed (0 with `committed`
    /// set: round 1, from before this field).
    #[serde(skip_serializing_if = "is_zero")]
    pub committed_round: u32,
    /// Ruling T15-1: the specs approved before the current round, round 1's first, then
    /// each round's amendment (`DesignState::approved_specs`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub approved_before: Vec<ApprovedSpec>,
}

fn is_zero(n: &u32) -> bool {
    *n == 0
}

/// The open document gate (decision 5).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocGate {
    pub kind: DocGateKind,
    pub version: u32,
    pub opened_at: u64,
    /// The note the orchestrator writes the next version to: the user's, or the
    /// engine's after a read-back ([`Revision::ReadBack`]).
    #[serde(default)]
    pub revising: Option<String>,
    /// The user's `Changes { review: true }`: the revision is reviewed again (decision
    /// 15, task M9.6.10).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub review: bool,
    /// What `revising` came from, so a fresh session is told it again in its own words
    /// (task M9.6.7 fix round 1, m3).
    #[serde(default, skip_serializing_if = "Revision::is_changes")]
    pub cause: Revision,
}

/// Why the orchestrator revises a gate's version.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Revision {
    /// The user asked for changes.
    #[default]
    Changes,
    /// The user went back to this gate from the next one.
    Back,
    /// The restore could not read the version back (`design::checked`).
    ReadBack,
}

impl Revision {
    fn is_changes(&self) -> bool {
        *self == Revision::Changes
    }
}

/// A brainstormer or a document reviewer: a headless, read-only session (decision 9).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesignAgent {
    pub label: String,
    pub role: AgentRole,
    pub route: Route,
    pub session: u32,
    /// Bound by the start op's result, as `PlannerSession.window_id`.
    #[serde(default)]
    pub window_id: Option<u32>,
    #[serde(default)]
    pub state: DesignAgentState,
    #[serde(default)]
    pub calls: u32,
    #[serde(default)]
    pub tokens: u64,
    #[serde(default)]
    pub started: Option<u64>,
    /// Its route came from the run's `brainstorm` list (`BrainstormPick.listed`), as
    /// its routing record's source says (fix round 1, m3).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub listed: bool,
    /// Ruling T8-7: its previous attempt ended without submitting (a Codex design agent,
    /// never nudged), so this one is its one relaunch, told so in its first turn.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub unsubmitted: bool,
    /// Ruling T18-6: the goal round it was queued in, so the snapshot lists only the
    /// current round's agents. A state saved before it reads as round 1.
    #[serde(default = "first_round")]
    pub round: u32,
}

fn first_round() -> u32 {
    1
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignAgentState {
    #[default]
    Queued,
    Running,
    /// Its draft is accepted and held (ruling T8-1), in memory only, until the
    /// brainstorm settles; a restore queues it again.
    Submitted,
    /// Its draft is stored.
    Done,
    Failed(String),
}

/// One document review (`submit_findings`), or why it failed: review `n` of `doc`,
/// numbered from 1 (task M9.6.10).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocReviewRecord {
    pub doc: DocKind,
    pub n: u32,
    #[serde(default)]
    pub findings: Vec<DocFinding>,
    #[serde(default)]
    pub failed: Option<String>,
    /// The document's gate versions when the review was asked: the review belongs to the
    /// next gate version, whose `ready` submit answers it.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub after: u32,
    /// Its reviewer ran on the orchestrator's own runtime (no peer installed).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub same_runtime: bool,
    /// Ruling T15-9: asked in a round since dropped (rejected, or cancelled before its
    /// plan's approval); kept, so no review number is reused, and skipped by every
    /// cycle's reader.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dropped: bool,
}

impl DesignState {
    /// The live design agent of `role` in `window`: a brainstormer, or the document
    /// reviewer, whose session runs there (the driver's caller check, task M9.6.6).
    pub fn live_agent(&self, role: AgentRole, window: u32) -> Option<&DesignAgent> {
        (self.brainstormers.iter().chain(&self.reviewer)).find(|a| {
            a.role == role && a.window_id == Some(window) && a.state == DesignAgentState::Running
        })
    }

    /// The cached latest text of `kind`, with its version number (a gate document's;
    /// for drafts, [`DesignState::draft_text`]).
    pub fn text_of(&self, kind: DocKind) -> Option<(u32, &str)> {
        let found = self.texts.iter().find(|(k, _, _)| *k == kind);
        found.map(|(_, n, text)| (*n, text.as_str()))
    }

    /// The cached text of brainstorm draft `n`, when it is cached.
    pub fn draft_text(&self, n: u32) -> Option<&str> {
        let found = (self.texts.iter()).find(|(k, m, _)| *k == DocKind::BrainstormDraft && *m == n);
        found.map(|(_, _, text)| text.as_str())
    }

    /// Why brainstorm draft `n` could not be read back, when a restore found so.
    pub fn unread_reason(&self, n: u32) -> Option<&str> {
        let found = self.unread.iter().find(|(m, _)| *m == n);
        found.map(|(_, reason)| reason.as_str())
    }

    /// Ruling T9-1a: brainstorm draft `n` could not be read back, for `reason`.
    pub fn mark_unread(&mut self, n: u32, reason: String) {
        self.unread.retain(|(m, _)| *m != n);
        self.unread.push((n, reason));
    }

    /// Caches `text` as `kind`'s latest, version `n`: for a draft, its brainstormer's
    /// latest.
    pub fn keep_text(&mut self, kind: DocKind, n: u32, text: String) {
        if kind == DocKind::BrainstormDraft {
            self.unread.retain(|(m, _)| *m != n);
        }
        let label = |n: u32| {
            self.find(kind, Some(n))
                .and_then(|v| v.label().map(String::from))
        };
        let mine = label(n);
        let stale: Vec<u32> = (self.texts.iter())
            .filter(|(k, m, _)| *k == kind && label(*m) == mine)
            .map(|(_, m, _)| *m)
            .collect();
        self.texts
            .retain(|(k, m, _)| *k != kind || !stale.contains(m));
        self.texts.push((kind, n, text));
    }
}

/// The run's design folder.
pub fn design_dir(run: &Run) -> PathBuf {
    run.data_dir.join(DESIGN_DIR)
}

/// The document a gate shows.
pub fn gate_doc(kind: DocGateKind) -> DocKind {
    match kind {
        DocGateKind::Brainstorm => DocKind::Brainstorm,
        DocGateKind::Spec => DocKind::Spec,
        DocGateKind::Plan => DocKind::Plan,
    }
}

/// The refusal for a run without the design flow.
pub fn not_design(run_id: &str) -> String {
    format!("run {run_id} does not use the design flow")
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
