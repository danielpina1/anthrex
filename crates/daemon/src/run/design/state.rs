//! The design flow's state on a run, `RunOrch.design` (brief decision 2), and the index
//! of its documents (decision 12). Pure: the engine records a version here and asks the
//! driver to write its file with [`Effect::WriteDoc`]; it never touches a file itself.
//!
//! Versions are immutable (DF §5.3, §12): [`store`] always takes the next number of its
//! kind, and the driver refuses to write a file that exists.

use std::path::PathBuf;

use proto::{
    AgentRole, DocAuthor, DocFinding, DocGateKind, DocKind, ReportSummary, Route, RunState,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub use super::pack::FrozenPack;
pub use super::requirements::Requirement;
use super::requirements::scan;
use super::template::kind_name;
use crate::run::engine::Effect;
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
}

/// One stored version: its index entry. The text is the file, never `run.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocVersion {
    pub kind: DocKind,
    /// Numbered per kind from 1, never reused.
    pub n: u32,
    pub author: DocAuthor,
    pub reason: String,
    pub bytes: u64,
    /// The written text's SHA-256, lower-case hex.
    pub sha256: String,
    pub at: u64,
    /// A spec version's requirement ids (`DocInfo.requirements`, DF §5.3).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requirements: Vec<String>,
    /// The findings answered `kept: <reason>`, stored with the version (decision 16).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub disputed: Vec<DocFinding>,
    /// Why the version went unreviewed, when its reviewer failed (decision 15).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_reviewed: Option<String>,
    /// What changed since the previous version (`changes::summary`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<String>,
    /// Its reviewer ran on the orchestrator's own runtime (no peer installed).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub same_runtime: bool,
    /// A merged brainstorm report as the engine read it (`report::summary`, task
    /// M9.6.9): the gate's Review panel shows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub report: Option<ReportSummary>,
    /// Ruling T5-1: a spec's review draft, sent to review `k`: stored with `n = 0`,
    /// never a gate version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_review: Option<u32>,
}

impl DocVersion {
    /// A brainstorm draft's label (`claude`, `codex`, `A`, `B`).
    pub fn label(&self) -> Option<&str> {
        match &self.author {
            DocAuthor::Brainstormer { label } => Some(label),
            _ => None,
        }
    }
}

impl DesignState {
    /// Version `n` of `kind`, or its latest when `n` is `None`. Ruling T5-1's seam: a
    /// spec's review draft is stored with `n = 0` (task M9.6.10), so the latest is the
    /// highest gate version, or, before any, the last draft stored (`max_by_key` keeps
    /// the last of equal keys). `get_doc { kind: "spec" }` reads it so.
    pub fn find(&self, kind: DocKind, n: Option<u32>) -> Option<&DocVersion> {
        let mut of_kind = self.versions.iter().filter(|v| v.kind == kind);
        match n {
            Some(n) => of_kind.find(|v| v.n == n && v.draft_review.is_none()),
            None => of_kind.max_by_key(|v| v.n),
        }
    }

    /// Ruling T5-1: `kind`'s review draft for review `k` (`get_doc`'s `draft`).
    pub fn draft(&self, kind: DocKind, k: u32) -> Option<&DocVersion> {
        (self.versions.iter()).find(|v| v.kind == kind && v.draft_review == Some(k))
    }

    /// The live design agent of `role` in `window`: a brainstormer, or the document
    /// reviewer, whose session runs there (the driver's caller check, task M9.6.6).
    pub fn live_agent(&self, role: AgentRole, window: u32) -> Option<&DesignAgent> {
        (self.brainstormers.iter().chain(&self.reviewer)).find(|a| {
            a.role == role && a.window_id == Some(window) && a.state == DesignAgentState::Running
        })
    }

    /// The latest brainstorm draft from `label` (`get_doc`'s `from`).
    pub fn draft_from(&self, label: &str) -> Option<&DocVersion> {
        (self.versions.iter())
            .filter(|v| v.kind == DocKind::BrainstormDraft && v.label() == Some(label))
            .max_by_key(|v| v.n)
    }

    /// The version a diff of `v` compares against: the latest earlier version of its
    /// kind (for a draft, from the same brainstormer). `None` for a first version.
    /// A review draft's is the draft before it (ruling T5-1), and a gate version never
    /// compares against a draft.
    pub fn previous(&self, v: &DocVersion) -> Option<&DocVersion> {
        if let Some(k) = v.draft_review {
            let earlier = self.versions.iter().filter(|p| p.kind == v.kind);
            return earlier
                .filter(|p| p.draft_review.is_some_and(|j| j < k))
                .max_by_key(|p| p.draft_review);
        }
        (self.versions.iter())
            .filter(|p| p.kind == v.kind && p.n < v.n && p.label() == v.label())
            .filter(|p| p.draft_review.is_none())
            .max_by_key(|p| p.n)
    }

    /// The number the next version of `kind` takes: one past every number it had.
    pub fn next_n(&self, kind: DocKind) -> u32 {
        let last = self.versions.iter().filter(|v| v.kind == kind).map(|v| v.n);
        last.max().unwrap_or(0) + 1
    }

    /// `v`'s file, relative to the design folder (decision 12): `spec-v<n>.md`, and a
    /// draft's `brainstorm/draft-<label>.md`. A brainstormer's later draft (after a
    /// rethink) is `brainstorm/draft-<label>-v<n>.md`, so no draft is overwritten.
    /// A spec's review draft is `spec-draft-r<k>.md` (ruling T5-1).
    pub fn file_name(&self, v: &DocVersion) -> String {
        let n = v.n;
        if let Some(k) = v.draft_review {
            return format!("{}-draft-r{k}.md", v.kind.label());
        }
        match (v.kind, v.label()) {
            (DocKind::BrainstormDraft, Some(label)) => match self.earlier_draft(v) {
                false => format!("brainstorm/draft-{label}.md"),
                true => format!("brainstorm/draft-{label}-v{n}.md"),
            },
            (kind, _) => format!("{}-v{n}.md", kind.label()),
        }
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

    /// Caches `text` as `kind`'s latest, version `n`: for a draft, its brainstormer's
    /// latest.
    pub fn keep_text(&mut self, kind: DocKind, n: u32, text: String) {
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

    /// How many gate versions `kind` has had (a spec's review drafts, `n = 0`, are not).
    pub fn gate_versions(&self, kind: DocKind) -> u32 {
        let of_kind = self.versions.iter().filter(|v| v.kind == kind && v.n > 0);
        of_kind.count() as u32
    }

    fn earlier_draft(&self, v: &DocVersion) -> bool {
        (self.versions.iter()).any(|p| p.kind == v.kind && p.n < v.n && p.label() == v.label())
    }
}

/// `findings-<kind>-v<n>.json`: a version's review findings with each answer.
pub fn findings_name(kind: DocKind, n: u32) -> String {
    format!("findings-{}-v{n}.json", kind.label())
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

/// A version to store, with what the gate shows beside it (each defaults to none).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewDoc {
    pub kind: DocKind,
    pub author: DocAuthor,
    pub reason: String,
    /// The text to write, already admitted (cleaned and checked) by the caller.
    pub text: String,
    pub disputed: Vec<DocFinding>,
    pub not_reviewed: Option<String>,
    pub changes: Vec<String>,
    pub same_runtime: bool,
    pub report: Option<ReportSummary>,
    /// Ruling T5-1: a spec's review draft for review `k`, stored with `n = 0`.
    pub draft_review: Option<u32>,
}

impl NewDoc {
    pub fn new(kind: DocKind, author: DocAuthor, reason: &str, text: &str) -> Self {
        NewDoc {
            kind,
            author,
            reason: reason.to_string(),
            text: text.to_string(),
            disputed: Vec::new(),
            not_reviewed: None,
            changes: Vec::new(),
            same_runtime: false,
            report: None,
            draft_review: None,
        }
    }
}

/// The refusal for a run without the design flow.
pub fn not_design(run_id: &str) -> String {
    format!("run {run_id} does not use the design flow")
}

/// Records `doc` as the next version of its kind and returns it with the write the
/// driver does: the file, then `versions.json`. The number is always one past the
/// kind's last, so no stored version is ever rewritten.
pub fn store(run: &mut Run, doc: NewDoc, now: u64) -> Result<(DocVersion, Effect), String> {
    let dir = design_dir(run);
    let id = run.id.clone();
    let design = run.orch.design.as_mut().ok_or_else(|| not_design(&id))?;
    match (doc.kind, &doc.author) {
        (DocKind::BrainstormDraft, DocAuthor::Brainstormer { label })
            if LABELS.contains(&label.as_str()) => {}
        (DocKind::BrainstormDraft, _) => {
            return Err("a brainstorm draft needs its brainstormer's label".into());
        }
        _ => {}
    }
    if doc.draft_review.is_some() && doc.kind != DocKind::Spec {
        return Err("only a spec has review drafts".into());
    }
    let requirements = match doc.kind {
        DocKind::Spec => scan(&doc.text).into_iter().map(|r| r.id).collect(),
        _ => Vec::new(),
    };
    let version = DocVersion {
        kind: doc.kind,
        // Ruling T5-1: a review draft is never numbered among the gate versions.
        n: match doc.draft_review {
            Some(_) => 0,
            None => design.next_n(doc.kind),
        },
        author: doc.author,
        reason: doc.reason,
        bytes: doc.text.len() as u64,
        sha256: sha256_hex(doc.text.as_bytes()),
        at: now,
        requirements,
        disputed: doc.disputed,
        not_reviewed: doc.not_reviewed,
        changes: doc.changes,
        same_runtime: doc.same_runtime,
        report: doc.report,
        draft_review: doc.draft_review,
    };
    design.versions.push(version.clone());
    let effect = Effect::WriteDoc {
        path: dir.join(design.file_name(&version)),
        text: doc.text,
        index: Some((dir.join(VERSIONS_FILE), index_text(design))),
    };
    Ok((version, effect))
}

/// The write of version `n` of `kind`'s findings, each with the orchestrator's answer
/// (`None` while unanswered). Like a version, the file is never rewritten.
pub fn store_findings(
    run: &Run,
    kind: DocKind,
    n: u32,
    findings: &[(DocFinding, Option<String>)],
) -> Result<Effect, String> {
    let design = run
        .orch
        .design
        .as_ref()
        .ok_or_else(|| not_design(&run.id))?;
    if design.find(kind, Some(n)).is_none() {
        return Err(format!("run {} has no {} v{n}", run.id, kind_name(kind)));
    }
    let text = serde_json::to_string_pretty(findings).map_err(|e| e.to_string())?;
    Ok(Effect::WriteDoc {
        path: design_dir(run).join(findings_name(kind, n)),
        text,
        index: None,
    })
}

/// `versions.json`'s text: the index as JSON.
pub fn index_text(design: &DesignState) -> String {
    serde_json::to_string_pretty(&design.versions).unwrap_or_else(|_| "[]".into())
}

/// `bytes`' SHA-256, lower-case hex, as `DocVersion.sha256` records it.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// The labels a brainstormer can have (decision 10): the runtimes' names with two
/// runtimes, `A` and `B` with one. A draft's file is named by it, so nothing else is
/// stored (fix round 1, m6).
pub const LABELS: [&str; 4] = ["claude", "codex", "A", "B"];

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
