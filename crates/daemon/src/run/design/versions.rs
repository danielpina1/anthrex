//! The design documents' version store (task M9.6.5, moved out of `state.rs` by task
//! M9.6.11, ruling T7-5): each version's index entry, its number and file name, and the
//! write `store` asks the driver for. Pure, as `state.rs`.

use proto::{DocAuthor, DocFinding, DocKind, ReportSummary};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::requirements::scan;
use super::state::{DesignState, VERSIONS_FILE, design_dir, not_design};
use super::template::kind_name;
use crate::run::engine::Effect;
use crate::run::model::Run;

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
    /// never a gate version. Task M9.6.11: the plan review's draft too.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft_review: Option<u32>,
    /// Ruling T7-10: opened by a read-back resubmit or an engine update, so brief ruling
    /// BD-2's cap does not count it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub uncapped: bool,
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
        // Ruling T15-9: the latest entry with that number.
        (self.versions.iter()).rfind(|v| v.kind == kind && v.draft_review == Some(k))
    }

    /// Ruling T15-9: the number of `kind`'s next review: one past the highest review
    /// draft stored, so a dropped round's number is never taken again.
    pub fn next_review(&self, kind: DocKind) -> u32 {
        let drafts = (self.versions.iter()).filter(|v| v.kind == kind);
        drafts.filter_map(|v| v.draft_review).max().unwrap_or(0) + 1
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
    /// Ruling T7-10: not counted by brief ruling BD-2's cap.
    pub uncapped: bool,
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
            uncapped: false,
        }
    }
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
    // Task M9.6.11: the plan review's draft too.
    if doc.draft_review.is_some() && !matches!(doc.kind, DocKind::Spec | DocKind::Plan) {
        return Err("only a spec or a plan has review drafts".into());
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
        uncapped: doc.uncapped,
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
