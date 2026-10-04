//! Milestone 9.6 (DF §10): a run's design fields in its snapshot: the mode, the open
//! document gate and every stored version's index entry. Pure, as `snapshot.rs`. A run
//! without the design flow shows `Off`, no gate and no documents, so its `RunInfo` is
//! written as 9.5's.

use proto::{DesignMode, DocGateInfo, DocInfo};

use super::design::state::{DesignState, gate_doc};
use super::model::Run;

/// `RunInfo.{design, doc_gate, docs}`.
pub fn design_fields(run: &Run) -> (DesignMode, Option<DocGateInfo>, Vec<DocInfo>) {
    let Some(design) = &run.orch.design else {
        return (run.design_mode, None, Vec::new());
    };
    (run.design_mode, doc_gate(design), docs(design))
}

/// The open gate, with what its version stored beside it.
fn doc_gate(design: &DesignState) -> Option<DocGateInfo> {
    let gate = design.gate.as_ref()?;
    let version = design.find(gate_doc(gate.kind), Some(gate.version));
    Some(DocGateInfo {
        kind: gate.kind,
        version: gate.version,
        revising: gate.revising.clone(),
        disputed: version.map(|v| v.disputed.clone()).unwrap_or_default(),
        not_reviewed: version.and_then(|v| v.not_reviewed.clone()),
        changes_summary: version.map(|v| v.changes.clone()).unwrap_or_default(),
        same_runtime: version.is_some_and(|v| v.same_runtime),
        report: version.and_then(|v| v.report.clone()),
    })
}

/// Every stored version, in the order stored.
fn docs(design: &DesignState) -> Vec<DocInfo> {
    (design.versions.iter())
        .map(|v| DocInfo {
            kind: v.kind,
            version: v.n,
            author: v.author.clone(),
            reason: v.reason.clone(),
            bytes: v.bytes,
            requirements: v.requirements.clone(),
        })
        .collect()
}
