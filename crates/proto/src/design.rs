//! The design flow's wire types (milestone 9.6, DF §10): the run's mode, its documents
//! and their gate, review findings, the document view `run show` returns, and the
//! history record of one design phase.
//!
//! Every time is unix seconds (`u64`), as everywhere in the engine. No float appears,
//! so the daemon's design state can derive `Eq` (brief decision 2).

use serde::{Deserialize, Serialize};

use crate::run::{AgentRole, Route};

/// Whether a run uses the design flow (decision 3). Frozen at the start; a run from
/// before milestone 9.6 reads `Off`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignMode {
    Full,
    #[default]
    Off,
}

impl DesignMode {
    /// For `skip_serializing_if`: a run without the design flow writes no `design`.
    pub fn is_off(&self) -> bool {
        *self == DesignMode::Off
    }
}

/// What a new round of a design run does with its documents (decision 28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundDesign {
    /// Amend the approved spec, then plan the round.
    Amend,
    /// Brainstorm, specify and plan the round from the start.
    Full,
    /// Plan the round as 9.3 does.
    Off,
}

/// A document the design flow writes (decision 12).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocKind {
    /// One brainstormer's draft.
    BrainstormDraft,
    /// The merged brainstorm report.
    Brainstorm,
    Spec,
    Plan,
}

impl DocKind {
    pub fn label(self) -> &'static str {
        match self {
            DocKind::BrainstormDraft => "brainstorm_draft",
            DocKind::Brainstorm => "brainstorm",
            DocKind::Spec => "spec",
            DocKind::Plan => "plan",
        }
    }
}

/// One of the three user gates (decision 5). Not `GateKind`, a task's bounce gate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocGateKind {
    Brainstorm,
    Spec,
    Plan,
}

impl DocGateKind {
    pub fn label(self) -> &'static str {
        match self {
            DocGateKind::Brainstorm => "brainstorm",
            DocGateKind::Spec => "spec",
            DocGateKind::Plan => "plan",
        }
    }
}

/// Who wrote a document version.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocAuthor {
    Orchestrator,
    /// The user's own edit at a gate (`DocGateAction::Edit`).
    User,
    Brainstormer {
        label: String,
    },
    /// Written by anthrex itself, such as the plan's `plan.md`.
    Engine,
}

/// How serious a document reviewer's finding is. Not `Severity`, a code review's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocSeverity {
    Blocking,
    Minor,
}

/// One finding of a document review (`submit_findings`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocFinding {
    pub id: String,
    pub severity: DocSeverity,
    /// Where in the document: a heading, a requirement id or a task id.
    pub place: String,
    pub text: String,
}

/// The orchestrator's answer to one finding: `"fixed"` or `"kept: <reason>"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FindingAnswer {
    pub id: String,
    pub answer: String,
}

/// What the user does at a gate (decision 7). Rethink is the brainstorm gate's only;
/// Back is the spec and plan gates' only.
///
/// Ruling T17-1 appended `Approve`'s `version` under protocol 17: an approve of no
/// particular version is still written as the bare `"approve"`, and the bare name an
/// earlier build wrote still reads as one (the `Serialize` and `Deserialize` impls
/// below wrap the derived ones).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(remote = "Self", rename_all = "snake_case")]
pub enum DocGateAction {
    /// `version`: the gate version the user reviewed (ruling T17-1).
    Approve {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        version: Option<u32>,
    },
    /// `review`: send the revision to the document reviewer again.
    Changes {
        note: String,
        review: bool,
    },
    /// The user's own text becomes the next version.
    Edit {
        text: String,
    },
    Rethink {
        note: String,
    },
    Back {
        note: String,
    },
    Reject,
}

impl DocGateAction {
    /// An approve of whatever version is open (a plain `run approve --gate`).
    pub const APPROVE: DocGateAction = DocGateAction::Approve { version: None };
}

impl Serialize for DocGateAction {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            DocGateAction::Approve { version: None } => {
                serializer.serialize_unit_variant("DocGateAction", 0, "approve")
            }
            _ => DocGateAction::serialize(self, serializer),
        }
    }
}

impl<'de> Deserialize<'de> for DocGateAction {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        /// The bare name, as an approve of no version was always written.
        #[derive(Deserialize)]
        #[serde(rename_all = "snake_case")]
        enum Bare {
            Approve,
        }
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Wire {
            Bare(Bare),
            Full(#[serde(deserialize_with = "DocGateAction::deserialize")] DocGateAction),
        }
        Ok(match Wire::deserialize(deserializer)? {
            Wire::Bare(Bare::Approve) => DocGateAction::APPROVE,
            Wire::Full(action) => action,
        })
    }
}

/// One gate version's index entry, as a client sees it (`RunInfo.docs`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocInfo {
    pub kind: DocKind,
    pub version: u32,
    pub author: DocAuthor,
    pub reason: String,
    pub bytes: u64,
    /// The requirement ids a spec version has (`R1`, `R2`, …); empty for the others.
    #[serde(default)]
    pub requirements: Vec<String>,
}

/// The open document gate (`RunInfo.doc_gate`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocGateInfo {
    pub kind: DocGateKind,
    pub version: u32,
    /// The user's note while the orchestrator writes the next version.
    #[serde(default)]
    pub revising: Option<String>,
    /// The findings the orchestrator answered `kept: <reason>` (decision 16).
    #[serde(default)]
    pub disputed: Vec<DocFinding>,
    /// Why the version went unreviewed, when its reviewer failed.
    #[serde(default)]
    pub not_reviewed: Option<String>,
    /// What changed since the previous version (`"+ R4a, R4b"`, `"- Risks"`).
    #[serde(default)]
    pub changes_summary: Vec<String>,
    /// The reviewer ran on the orchestrator's own runtime (no peer installed).
    #[serde(default)]
    pub same_runtime: bool,
    /// At the brainstorm gate, the merged report as the engine read it when it stored
    /// the version (DF §6.1's Review panel; task M9.6.9, appended).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<ReportSummary>,
    /// Why the orchestrator revises the version (task M9.6.17, appended): the user's
    /// changes (the default, never written), the user's back from the next gate, or a
    /// version anthrex could not read back, whose `revising` note is anthrex's own.
    #[serde(default, skip_serializing_if = "RevisingCause::is_changes")]
    pub revising_cause: RevisingCause,
}

/// What a revising gate revises for (`DocGateInfo.revising_cause`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RevisingCause {
    /// The user asked for changes; `revising` is their note.
    #[default]
    Changes,
    /// The user went back to this gate from the next one; `revising` is their note.
    Back,
    /// anthrex could not read the stored version back; `revising` is anthrex's note.
    ReadBack,
}

impl RevisingCause {
    /// For `skip_serializing_if`: the default cause is not written.
    pub fn is_changes(&self) -> bool {
        *self == RevisingCause::Changes
    }

    /// Whether `revising` is the user's own note (changes or back).
    pub fn is_users(&self) -> bool {
        *self != RevisingCause::ReadBack
    }
}

/// The merged brainstorm report, parsed: how many points its two comparison sections
/// make, and each approach with its tag (DF §3.4, §6.1).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReportSummary {
    /// The points under `## Where they agree`.
    pub agree: u32,
    /// The points under `## Where they disagree`.
    pub disagree: u32,
    #[serde(default)]
    pub approaches: Vec<ApproachTag>,
}

/// One approach of a merged report: its name, and the label its heading is tagged with
/// (`claude`, `codex`, `A`, `B` or `both`), without the brackets.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApproachTag {
    pub name: String,
    pub tag: String,
}

/// One document version, answered to `RunRequest::ShowDoc` (`RunReply::Doc`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocView {
    pub run: String,
    pub kind: DocKind,
    pub version: u32,
    pub text: String,
    /// A line diff against the previous version, when asked for.
    #[serde(default)]
    pub diff: Option<String>,
    /// The version's review findings, each with the orchestrator's answer.
    #[serde(default)]
    pub findings: Vec<(DocFinding, Option<String>)>,
}

/// One design phase of one round (`"type": "phase"`, decision 32), keyed by
/// `record_id` like every history record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseRecord {
    pub v: u32,
    pub record_id: String,
    pub at: u64,
    pub run_id: String,
    pub round: u32,
    /// `brainstorming`, `specifying` or `planning`.
    pub phase: String,
    pub secs: u64,
    #[serde(default)]
    pub agents: Vec<PhaseAgent>,
    pub gate_versions: u32,
    pub disputed: u32,
}

/// One design agent's spend in a phase, a refit sample when `outcome` is `"ok"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseAgent {
    pub role: AgentRole,
    pub route: Route,
    pub calls: u32,
    pub tokens: u64,
    /// `"ok"`, `"failed: <reason>"` or `"over budget"`.
    pub outcome: String,
    /// Ruling T13-1 (task M9.6.13): its active seconds, summed with its calls and tokens
    /// over all its sessions in the phase; the refit's minutes. Absent from a line
    /// written before it: 0.
    #[serde(default)]
    pub secs: u64,
    /// Ruling T13-3 (task M9.6.13 fix round 1): how many sessions its sums cover, so the
    /// refit can take per-session values. Absent from a line written before it: 1.
    #[serde(default = "one_session")]
    pub sessions: u32,
}

fn one_session() -> u32 {
    1
}

#[cfg(test)]
#[path = "design_tests.rs"]
mod tests;
