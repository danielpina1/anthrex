//! Milestone 9.6 task M9.6.10 (DF §4, decisions 14 to 16, ruling T5-1), part of
//! `design.rs`: the orchestrator's spec and its review.
//!
//! - **A review draft** (`ready: false`) is checked as the spec is (its Open questions
//!   may still be open), stored as `spec-draft-r<k>.md` with `n = 0` (never a gate
//!   version), and sent to review `k`: the document reviewer is queued on the
//!   orchestrator's peer runtime (`design_reviewer.rs`). At most two reviews before the
//!   spec's first gate version; after the user's changes only when they asked for one
//!   (`Changes { review: true }`), and then exactly one.
//! - **A ready spec** answers every finding of the latest review of its cycle (the
//!   reviews asked since the last gate version), or is refused naming the missing ids;
//!   it opens the gate with the findings answered `kept: <reason>` as disputed, the
//!   failed review's `not reviewed: <reason>`, and the reviewer's `same_runtime`. The
//!   version's findings file keeps every answer.
//! - **Approval.** The approved version's text is read back by the driver, checked
//!   against its index entry, and its requirements and Goal section are stored from it,
//!   never from the engine's cache of texts ([`approved`], [`requirements_read`]).
//!
//! Pure (design decision 2).

use proto::{DocAuthor, DocFinding, DocGateKind, DocKind, RunState};
use serde_json::{Value, json};

use super::super::design_agents::reviewer;
use super::super::requests::log;
use super::super::{Effect, design_gate, wake};
use super::{DocChecked, checked_text};
use crate::run::design::requirements;
use crate::run::design::state::{
    DocReviewRecord, NewDoc, Revision, design_dir, store, store_findings,
};
use crate::run::model::Run;
use crate::run::orch::tools::SubmitDoc;

/// Decision 15: a third review draft before the gate.
pub const TWO_REVIEWS: &str = "the spec has had its two reviews; submit with ready = true";
/// DF §4.2: a review draft of a revision the user asked no review of.
pub const NO_REVIEW_ASKED: &str =
    "the user asked for no review of this revision; submit with ready = true";
/// DF §4.2: a ready revision the user asked a review of, before that review.
pub const REVIEW_ASKED: &str =
    "the user asked for a review of this revision; submit it with ready = false first";
/// DF §4.2: a second review draft of a revision the user asked one review of.
pub const ASKED_REVIEW_DONE: &str =
    "the spec has had the review the user asked for; submit with ready = true";
/// A spec's first gate version submitted without any review: its `not reviewed` line.
pub const UNREVIEWED: &str = "the orchestrator submitted it without a review";
/// Decision 15: reviews before the spec's first gate version, at most.
const MAX_REVIEWS: usize = 2;

/// The orchestrator's spec, admitted in its phase or at its revising gate: a review
/// draft, or the gate's next version. `reason` is the version's (`submitted`,
/// `revised: <note>`).
pub(in crate::run::engine) fn submit_spec(
    run: &mut Run,
    doc: SubmitDoc,
    reason: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<Value, String> {
    if let Some((kind, k)) = reviewer::in_progress(run) {
        let kind = kind.label();
        return Err(format!(
            "the {kind} review {k} is still running; wait for its findings"
        ));
    }
    let Some(design) = run.orch.design.as_ref() else {
        return Err(crate::run::design::state::not_design(&run.id));
    };
    let cycle = design.gate_versions(DocKind::Spec);
    let reviews: Vec<&DocReviewRecord> = (design.reviews.iter())
        .filter(|r| r.doc == DocKind::Spec && r.after == cycle)
        .collect();
    // At the spec gate the orchestrator revises: whether the user asked for a review.
    let asked = design_gate::waiting(run)
        .filter(|g| g.kind == DocGateKind::Spec && g.revising.is_some())
        .map(|g| g.review);
    let (count, latest) = (reviews.len(), reviews.last().map(|r| (*r).clone()));
    match doc.ready {
        false => {
            match asked {
                Some(false) => return Err(NO_REVIEW_ASKED.into()),
                Some(true) if count >= 1 => return Err(ASKED_REVIEW_DONE.into()),
                None if count >= MAX_REVIEWS => return Err(TWO_REVIEWS.into()),
                _ => {}
            }
            draft(run, &doc, cycle, now, fx)
        }
        true if asked == Some(true) && latest.is_none() => Err(REVIEW_ASKED.into()),
        true => ready(run, &doc, reason, (cycle, latest), now, fx),
    }
}

/// Ruling T5-1: the review draft for review `k`, stored with `n = 0`, and its reviewer
/// queued.
fn draft(
    run: &mut Run,
    doc: &SubmitDoc,
    cycle: u32,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<Value, String> {
    let text = checked_text(run, DocKind::Spec, &doc.text, doc.amend, false)?;
    let reviews = (run.orch.design.iter()).flat_map(|d| &d.reviews);
    let k = reviews.filter(|r| r.doc == DocKind::Spec).count() as u32 + 1;
    let reason = format!("draft for review {k}");
    let mut new = NewDoc::new(DocKind::Spec, DocAuthor::Orchestrator, &reason, &text);
    new.draft_review = Some(k);
    let (_, write) = store(run, new, now)?;
    fx.push(write);
    log(run, now, format!("the spec draft for review {k} is stored"));
    reviewer::queue(run, DocKind::Spec, (k, cycle), now);
    Ok(json!({"accepted": true, "kind": "spec", "review": k, "awaiting_review": true}))
}

/// Decision 15: the gate's next version, once every finding of `latest` (the cycle's
/// last review) is answered; decision 16's disputed findings, and the findings file
/// with every answer.
fn ready(
    run: &mut Run,
    doc: &SubmitDoc,
    reason: &str,
    (cycle, latest): (u32, Option<DocReviewRecord>),
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<Value, String> {
    let answer = |id: &str| (doc.responses.iter()).find(|a| a.id == id);
    let reviewed = latest.as_ref().filter(|r| r.failed.is_none());
    if let Some(review) = reviewed {
        let missing: Vec<&str> = (review.findings.iter())
            .filter(|f| answer(&f.id).is_none())
            .map(|f| f.id.as_str())
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "answer every finding of review {}; missing: {}",
                review.n,
                missing.join(", ")
            ));
        }
    }
    let text = checked_text(run, DocKind::Spec, &doc.text, doc.amend, true)?;
    let mut new = NewDoc::new(DocKind::Spec, DocAuthor::Orchestrator, reason, &text);
    let mut answered: Vec<(DocFinding, Option<String>)> = Vec::new();
    match (&latest, reviewed) {
        (_, Some(review)) => {
            for f in &review.findings {
                let given = answer(&f.id).map(|a| a.answer.clone());
                if given.as_deref().is_some_and(|a| a.starts_with("kept: ")) {
                    new.disputed.push(f.clone());
                }
                answered.push((f.clone(), given));
            }
            new.same_runtime = review.same_runtime;
        }
        (Some(failed), None) => new.not_reviewed = failed.failed.clone(),
        (None, _) if cycle == 0 => new.not_reviewed = Some(UNREVIEWED.into()),
        (None, _) => {}
    }
    let n = design_gate::open(run, new, now, fx)?;
    if !answered.is_empty() {
        match store_findings(run, DocKind::Spec, n, &answered) {
            Ok(write) => fx.push(write),
            Err(error) => log(run, now, format!("the spec v{n}'s findings: {error}")),
        }
    }
    Ok(json!({"accepted": true, "kind": "spec", "version": n, "awaiting_approval": true}))
}

/// The user approved spec v`n`: its requirements wait for its text, read back by the
/// driver against its index entry (`Effect::ReadBack`, then [`requirements_read`]).
pub(in crate::run::engine) fn approved(run: &mut Run, n: u32, fx: &mut Vec<Effect>) {
    let dir = design_dir(run);
    let run_id = run.id.clone();
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    design.approved_spec = Some(n);
    design.requirements.clear();
    design.goal_section.clear();
    let Some(version) = design.find(DocKind::Spec, Some(n)).cloned() else {
        return;
    };
    let path = dir.join(design.file_name(&version));
    fx.push(Effect::ReadBack {
        run_id,
        docs: vec![(version, path)],
    });
}

/// The approved spec's text as the driver read it back (after its approval, or after a
/// restore): its requirements and Goal section are stored. A read that failed is
/// logged; in planning, the spec gate then reopens for the orchestrator to submit it
/// again, as a restore's read-back does, since nothing may be planned against it.
pub(in crate::run::engine) fn requirements_read(run: &mut Run, checked: &[DocChecked], now: u64) {
    let due = (run.orch.design.as_ref())
        .filter(|d| d.requirements.is_empty())
        .and_then(|d| d.approved_spec);
    let Some(n) = due else {
        return;
    };
    let Some(doc) = (checked.iter()).find(|c| c.kind == DocKind::Spec && c.n == n) else {
        return;
    };
    match &doc.read {
        Ok(Some(text)) => {
            let found = requirements::scan(text);
            let ids: Vec<String> = found.iter().map(|r| r.id.clone()).collect();
            if let Some(design) = run.orch.design.as_mut() {
                design.requirements = found;
                design.goal_section = requirements::goal_section(text);
            }
            let text = format!(
                "the spec v{n}'s requirements are stored: {}",
                ids.join(", ")
            );
            log(run, now, text);
        }
        Ok(None) => {}
        Err(reason) => {
            let text =
                format!("design flow: the approved spec v{n} could not be read back: {reason}");
            log(run, now, text);
            if run.state != RunState::Planning {
                return;
            }
            if let Some(design) = run.orch.design.as_mut() {
                design.approved_spec = None;
            }
            let note =
                format!("anthrex could not read back the approved spec v{n}; submit it again");
            let revising = Some((note, Revision::ReadBack));
            design_gate::set_revising(run, DocGateKind::Spec, revising, false, now);
            let text = format!("the approved spec v{n} could not be read back; submit it again");
            wake::note(run, text);
        }
    }
}

/// The orchestrator's digest (`run_status`): the spec review its next `ready` submit
/// answers, while the spec is being written: running, its findings, or its failure.
pub(crate) fn digest(run: &Run) -> Option<Value> {
    let design = run.orch.design.as_ref()?;
    let cycle = design.gate_versions(DocKind::Spec);
    let review = (design.reviews.iter())
        .rev()
        .find(|r| r.doc == DocKind::Spec && r.after == cycle)?;
    let writing = matches!(run.state, RunState::Specifying)
        || design_gate::waiting(run).is_some_and(|g| g.kind == DocGateKind::Spec);
    if !writing {
        return None;
    }
    let running = reviewer::in_progress(run).is_some_and(|(_, k)| k == review.n);
    let findings: Vec<Value> = (review.findings.iter())
        .map(|f| json!({"id": f.id, "severity": f.severity, "place": f.place, "text": f.text}))
        .collect();
    Some(json!({
        "review": review.n,
        "running": running,
        "findings": findings,
        "failed": review.failed,
    }))
}
