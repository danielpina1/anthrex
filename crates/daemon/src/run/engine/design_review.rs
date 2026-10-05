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
//!   never from the engine's cache of texts ([`approved`], [`requirements_read`]). A
//!   failed read-back is read again when the run resumes or restores, and a second
//!   failure halts the run (ruling T10-3, [`read_again`]).
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
/// DF §4.2: a review draft of a revision the user asked no review of (ruling T10-6:
/// it ends telling the orchestrator what to submit).
pub const NO_REVIEW_ASKED: &str =
    "the user asked for no review of this revision; submit it with ready: true";
/// DF §4.2: a ready revision the user asked a review of, before that review.
pub const REVIEW_ASKED: &str =
    "the user asked for a review of this revision; submit it with ready = false first";
/// DF §4.2: a second review draft of a revision the user asked one review of.
pub const ASKED_REVIEW_DONE: &str =
    "the spec has had the review the user asked for; submit with ready = true";
/// Rulings T10-4 and T10-6: a review draft at a spec gate revising after a Back or a
/// failed read-back, which is not sent for review.
pub const NOT_SENT: &str = "this revision is not sent for review; submit it with ready: true";
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
        .filter(|r| r.doc == DocKind::Spec && r.after == cycle && !r.dropped)
        .collect();
    // At the spec gate the orchestrator revises: whether the user asked for a review,
    // and why the gate reopened (ruling T10-4).
    let asked = design_gate::waiting(run)
        .filter(|g| g.kind == DocGateKind::Spec && g.revising.is_some())
        .map(|g| (g.review, g.cause));
    let (count, latest) = (reviews.len(), reviews.last().map(|r| (*r).clone()));
    match doc.ready {
        false => {
            match asked {
                Some((false, Revision::Changes)) => return Err(NO_REVIEW_ASKED.into()),
                Some((false, _)) => return Err(NOT_SENT.into()),
                Some((true, _)) if count >= 1 => return Err(ASKED_REVIEW_DONE.into()),
                None if count >= MAX_REVIEWS => return Err(TWO_REVIEWS.into()),
                _ => {}
            }
            draft(run, &doc, cycle, now, fx)
        }
        true if asked.is_some_and(|(review, _)| review) && latest.is_none() => {
            Err(REVIEW_ASKED.into())
        }
        true => ready(run, &doc, reason, (latest, asked.is_none()), now, fx),
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
    let text = checked_text(run, DocKind::Spec, &doc.text, amends(run, doc), false)?;
    let k = (run.orch.design.as_ref()).map_or(1, |d| d.next_review(DocKind::Spec));
    let reason = format!("draft for review {k}");
    let mut new = NewDoc::new(DocKind::Spec, DocAuthor::Orchestrator, &reason, &text);
    new.draft_review = Some(k);
    let (_, write) = store(run, new, now)?;
    fx.push(write);
    log(run, now, format!("the spec draft for review {k} is stored"));
    reviewer::queue(run, DocKind::Spec, (k, cycle), now);
    Ok(json!({"accepted": true, "kind": "spec", "review": k, "awaiting_review": true}))
}

/// Ruling T15-5: in a round that amends the spec, the orchestrator's spec is checked as
/// an amendment whatever its `amend`, as the user's edit at the gate is.
fn amends(run: &Run, doc: &SubmitDoc) -> bool {
    doc.amend || (run.orch.design.as_ref()).is_some_and(|d| d.amending())
}

/// Decision 15: the gate's next version, once every finding of `latest` (the cycle's
/// last review) is answered; decision 16's disputed findings, and the findings file
/// with every answer. A version of a cycle the user did not ask about (`unasked`: not
/// a revision at the spec gate) with no review is marked not reviewed, the first
/// cycle's and every later one's (task 10's carry: after a brainstorm back, and a
/// round's amendment).
fn ready(
    run: &mut Run,
    doc: &SubmitDoc,
    reason: &str,
    (latest, unasked): (Option<DocReviewRecord>, bool),
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
    let text = checked_text(run, DocKind::Spec, &doc.text, amends(run, doc), true)?;
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
        (None, _) if unasked => new.not_reviewed = Some(UNREVIEWED.into()),
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
    unapproved(run);
    if let Some(design) = run.orch.design.as_mut() {
        design.approved_spec = Some(n);
    }
    ask(run, n, fx);
}

/// Ruling T10-3: when the run resumes, an approved spec whose requirements are not
/// stored yet (its read-back failed, or never came back) is read back again.
pub(in crate::run::engine) fn read_again(run: &mut Run, fx: &mut Vec<Effect>) {
    let due = (run.orch.design.as_ref())
        .filter(|d| d.requirements.is_empty())
        .and_then(|d| d.approved_spec);
    if let Some(n) = due {
        ask(run, n, fx);
    }
}

/// `Effect::ReadBack` of spec v`n`, against its index entry.
fn ask(run: &Run, n: u32, fx: &mut Vec<Effect>) {
    let Some(design) = run.orch.design.as_ref() else {
        return;
    };
    let Some(version) = design.find(DocKind::Spec, Some(n)).cloned() else {
        return;
    };
    let path = design_dir(run).join(design.file_name(&version));
    fx.push(Effect::ReadBack {
        run_id: run.id.clone(),
        docs: vec![(version, path)],
    });
}

/// The approved spec's text as the driver read it back (after its approval, or after a
/// restore): its requirements and Goal section are stored. A read that failed is
/// logged, here only (fix round 1, m3: the version it returns); in planning, the spec
/// gate then reopens for the orchestrator to submit it again, as a restore's read-back
/// does, since nothing may be planned against it.
pub(in crate::run::engine) fn requirements_read(
    run: &mut Run,
    checked: &[DocChecked],
    now: u64,
) -> Option<u32> {
    let due = (run.orch.design.as_ref())
        .filter(|d| d.requirements.is_empty())
        .and_then(|d| d.approved_spec);
    let n = due?;
    let doc = (checked.iter()).find(|c| c.kind == DocKind::Spec && c.n == n)?;
    match &doc.read {
        Ok(Some(text)) => {
            let found = requirements::scan(text);
            let mut ids: Vec<String> = Vec::new();
            if let Some(design) = run.orch.design.as_mut() {
                // Task M9.6.15 (ruling T4-4): a round's amendment merges into its base.
                design.store_requirements(found);
                ids = design.requirements.iter().map(|r| r.id.clone()).collect();
                design.goal_section = requirements::goal_section(text);
                design.interfaces_section = requirements::interfaces_section(text);
                design.spec_unread = false;
            }
            let text = format!(
                "the spec v{n}'s requirements are stored: {}",
                ids.join(", ")
            );
            log(run, now, text);
            None
        }
        Ok(None) => None,
        Err(reason) => {
            // Task 10's review (m9): the log line is ruling T10-3's halt text.
            let text = format!("design flow: the approved spec could not be read back: {reason}");
            // Ruling T10-3: read again on resume or restore; a second failure halts.
            if run.orch.design.as_ref().is_some_and(|d| d.spec_unread) {
                unread_halt(run, text, now);
                return Some(n);
            }
            log(run, now, text);
            if run.state != RunState::Planning {
                if let Some(design) = run.orch.design.as_mut() {
                    design.spec_unread = true;
                }
                return Some(n);
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
            Some(n)
        }
    }
}

/// Ruling T10-5: a Back reopens the spec or the brainstorm, so the spec is no longer
/// approved: its approved version, requirements and Goal section are cleared.
pub(in crate::run::engine) fn unapproved(run: &mut Run) {
    if let Some(design) = run.orch.design.as_mut() {
        design.approved_spec = None;
        design.requirements.clear();
        design.goal_section.clear();
        design.interfaces_section.clear();
        design.spec_unread = false;
    }
}

/// Ruling T10-3: the approved spec's second failed read-back halts the run, retryably;
/// its plain `run resume` returns it to the phase it left and reads the spec again.
fn unread_halt(run: &mut Run, text: String, now: u64) {
    // Ruling T11-2: a run already halted (a restore's read-back failed again) keeps the
    // phase it left, its reason and its retry; the failure is only logged.
    if run.state == RunState::Halted {
        run.halt_retryable = true;
        return log(run, now, text);
    }
    super::super::design_spend::stop_clock(run, now);
    let from = match run.state {
        RunState::Paused => run.paused_from.take(),
        state => Some(state),
    };
    if let Some(design) = run.orch.design.as_mut() {
        design.halted_from = from;
    }
    super::super::merge::halt(run, text, now);
    run.halt_retryable = true;
}

/// The orchestrator's digest (`run_status`): the spec review its next `ready` submit
/// answers, while the spec is being written: running, its findings, or its failure.
pub(crate) fn digest(run: &Run) -> Option<Value> {
    let design = run.orch.design.as_ref()?;
    let cycle = design.gate_versions(DocKind::Spec);
    let review = (design.reviews.iter())
        .rev()
        .find(|r| r.doc == DocKind::Spec && r.after == cycle && !r.dropped)?;
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
