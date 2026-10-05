//! Milestone 9.6 task M9.6.11 (DF §5.1; decisions 18 to 21), part of `design.rs`: a
//! design run's plan.
//!
//! - **The checks.** A plan submit that would take a version (in planning, or at the
//!   plan gate the orchestrator revises) waits for the approved spec's read-back (ruling
//!   T10-2), then passes `coverage::check` against the approved requirements only
//!   (Review focus 2): coverage, dangling `covers`, brief shape ([`refusal`]). Approval
//!   re-checks the gate's plan ([`approve_refusal`]).
//! - **The plan review.** The orchestrator's first passing submit stores the rendered
//!   `plan.md` as the review's draft (`plan-draft-r1.md`, so the reviewer's `get_doc`
//!   reads it) and queues the document reviewer; the run stays in planning. The plan has
//!   one review. The next submit answers every finding, and opens the gate with the kept
//!   ones disputed and the findings file ([`submit`]).
//! - **Changes at the open gate.** A user's `run edit` that changes `plan.md` is its
//!   next version, `edited by you` (ruling T7-7, [`user_edited`]); an engine change
//!   (a decider's size or route) is one too, `updated by anthrex: sizes and routes`. A
//!   revising gate a sub-planner left for planning is cleared, its note kept for the
//!   next version ([`pass`]).
//!
//! Pure (design decision 2).

use proto::{DocAuthor, DocFinding, DocGateKind, DocKind, FindingAnswer, RunState, TaskState};
use serde_json::{Value, json};

use super::super::design_agents::reviewer;
use super::super::orch::{submit_plan, submit_refusal};
use super::super::requests::log;
use super::super::{Effect, design_gate};
use crate::run::design::coverage;
use crate::run::design::plan_md;
use crate::run::design::state::{NewDoc, sha256_hex, store, store_findings};
use crate::run::model::{Run, Task};

/// Ruling T10-2: a plan submit before the approved spec's requirements are stored.
pub const READ_BACK_PENDING: &str =
    "the approved spec is still being read back; submit the plan again in a moment";
/// Task 6's review (m5): `edit_plan`'s responses in a run without the flow.
pub const RESPONSES_ONLY: &str = "responses are only for a design run's plan review";
/// The addendum's engine update: the reason of the version it stores.
pub const ENGINE_UPDATE: &str = "updated by anthrex: sizes and routes";
/// Ruling T7-7: the reason of a user's edit's version.
const USER_EDIT: &str = "edited by you";
/// Ruling T11-3 (m2): the `not reviewed` line of a v1 the user submitted.
pub const USER_SUBMITTED: &str = "you submitted it yourself";
/// Ruling T11-1: a design run's spawn without the requirement ids its epic owns.
pub const NEEDS_COVERS: &str =
    "in a design run, spawn_subplanner needs covers: the requirement ids this epic owns";
/// Ruling T11-3 (m4): a spawn before the approved spec's requirements are stored.
pub const SPAWN_PENDING: &str =
    "the approved spec is still being read back; spawn the sub-planner again in a moment";

/// What the plan's first gate version shows of its review.
#[derive(Default)]
struct Opening {
    disputed: Vec<DocFinding>,
    not_reviewed: Option<String>,
    same_runtime: bool,
    findings: Vec<(DocFinding, Option<String>)>,
}

/// Whether a submit now takes a plan version: in planning, or at the plan gate the
/// orchestrator revises.
fn takes_version(run: &Run) -> bool {
    match run.state {
        RunState::Planning => true,
        RunState::AwaitingApproval => design_gate::waiting(run)
            .is_some_and(|g| g.kind == DocGateKind::Plan && g.revising.is_some()),
        _ => false,
    }
}

/// Decision 18 for `orch::submit_refusal`: a design run's plan, where a submit takes a
/// version, is refused while the approved spec is read back (ruling T10-2), and then by
/// the first failing coverage check.
pub(in crate::run::engine) fn refusal(run: &Run) -> Option<String> {
    takes_version(run).then(|| checks(run))?
}

/// Decision 18's re-check at approval (`rules::approve`, the gate's own approve): the
/// plan at its gate still passes (a removal at the gate can uncover a requirement).
pub(crate) fn approve_refusal(run: &Run) -> Option<String> {
    let at_plan = design_gate::waiting(run).is_some_and(|g| g.kind == DocGateKind::Plan);
    at_plan.then(|| checks(run))?
}

fn checks(run: &Run) -> Option<String> {
    let design = run.orch.design.as_ref()?;
    if design.approved_spec.is_some() && design.requirements.is_empty() {
        return Some(READ_BACK_PENDING.into());
    }
    coverage::check(run, &design.requirements)
}

/// A plan submit by `author` (`None`: no submit): the orchestrator's `edit_plan`, with
/// its `responses`, or the user's `run edit --submit`. A run without the flow submits
/// as in 9.5. In a design run the orchestrator's first passing submit goes to the plan
/// review instead (the run stays in planning); otherwise the plan is submitted and the
/// gate opens at `plan.md`'s next version.
pub(in crate::run::engine) fn submit(
    run: &mut Run,
    author: Option<DocAuthor>,
    responses: &[FindingAnswer],
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<(), String> {
    let Some(author) = author else {
        return Ok(());
    };
    let who = match author {
        DocAuthor::User => "the user",
        _ => "the orchestrator",
    };
    if run.orch.design.is_none() {
        return submit_plan(run, who, now);
    }
    if let Some((DocKind::Plan, k)) = reviewer::in_progress(run) {
        return Err(format!(
            "the plan review {k} is still running; wait for its findings"
        ));
    }
    submit_refusal(run).map_or(Ok(()), Err)?;
    if review_due(run, &author) {
        return start_review(run, now, fx);
    }
    let opening = answered(run, &author, responses)?;
    submit_plan(run, who, now)?;
    opened(run, author, opening, now, fx);
    Ok(())
}

/// Decision 20: the orchestrator's first passing submit in planning, before the plan's
/// first gate version, is reviewed; the plan has one review.
fn review_due(run: &Run, author: &DocAuthor) -> bool {
    let Some(design) = run.orch.design.as_ref() else {
        return false;
    };
    *author == DocAuthor::Orchestrator
        && run.state == RunState::Planning
        && !design.plan_review_done
        && design.gate_versions(DocKind::Plan) == 0
}

/// Task 6's carry (e): the rendered `plan.md` stored as review `k`'s draft before its
/// reviewer starts, so the reviewer's `get_doc { kind: "plan" }` reads it (the latest
/// plan before any gate version, ruling T5-1's seam).
fn start_review(run: &mut Run, now: u64, fx: &mut Vec<Effect>) -> Result<(), String> {
    let Some(design) = run.orch.design.as_ref() else {
        return Ok(());
    };
    let text = plan_md::render(run, &design.requirements);
    let reviews = (design.reviews.iter()).filter(|r| r.doc == DocKind::Plan);
    let k = reviews.count() as u32 + 1;
    let reason = format!("draft for review {k}");
    let mut draft = NewDoc::new(DocKind::Plan, DocAuthor::Orchestrator, &reason, &text);
    draft.draft_review = Some(k);
    let (_, write) = store(run, draft, now)?;
    fx.push(write);
    if let Some(design) = run.orch.design.as_mut() {
        design.plan_review_done = true;
    }
    log(run, now, format!("the plan draft for review {k} is stored"));
    reviewer::queue(run, DocKind::Plan, (k, 0), now);
    Ok(())
}

/// Decision 20: the plan's review belongs to its first gate version. The
/// orchestrator's submit answers every finding (a failed review asks none, and marks
/// the version not reviewed); the findings answered `kept: <reason>` are disputed.
fn answered(run: &Run, author: &DocAuthor, responses: &[FindingAnswer]) -> Result<Opening, String> {
    let mut opening = Opening::default();
    let Some(design) = run.orch.design.as_ref() else {
        return Ok(opening);
    };
    // Ruling T11-3 (m2): a v1 the user submitted never had the plan review.
    if *author == DocAuthor::User && design.gate_versions(DocKind::Plan) == 0 {
        opening.not_reviewed = Some(USER_SUBMITTED.into());
    }
    let review = (design.reviews.iter().rev()).find(|r| r.doc == DocKind::Plan);
    let Some(review) = review.filter(|_| design.gate_versions(DocKind::Plan) == 0) else {
        return Ok(opening);
    };
    if let Some(reason) = &review.failed {
        opening.not_reviewed.get_or_insert_with(|| reason.clone());
        return Ok(opening);
    }
    let answer = |id: &str| responses.iter().find(|a| a.id == id);
    let missing: Vec<&str> = (review.findings.iter())
        .filter(|f| answer(&f.id).is_none())
        .map(|f| f.id.as_str())
        .collect();
    if *author == DocAuthor::Orchestrator && !missing.is_empty() {
        return Err(format!(
            "answer every finding of review {}; missing: {}",
            review.n,
            missing.join(", ")
        ));
    }
    opening.same_runtime = review.same_runtime;
    for f in &review.findings {
        let given = answer(&f.id).map(|a| a.answer.clone());
        if given.as_deref().is_some_and(|a| a.starts_with("kept: ")) {
            opening.disputed.push(f.clone());
        }
        opening.findings.push((f.clone(), given));
    }
    Ok(opening)
}

/// A plan submitted in a design run: the plan gate opens at `plan.md`'s next version
/// (decision 21), the first time and after each changes request (with the user's note,
/// also when a sub-planner left the gate for planning); a submit at the open gate
/// otherwise stores nothing. Moved from `design_gate.rs::plan_submitted`.
fn opened(run: &mut Run, author: DocAuthor, opening: Opening, now: u64, fx: &mut Vec<Effect>) {
    let Some(design) = run.orch.design.as_ref() else {
        return;
    };
    if run.state != RunState::AwaitingApproval {
        return;
    }
    let note = match &design.gate {
        None => design.plan_revision.clone(),
        Some(g) if g.kind == DocGateKind::Plan && g.revising.is_some() => g.revising.clone(),
        Some(_) => return,
    };
    let reason = match note {
        Some(note) => format!("revised: {}", design_gate::note_head(&note)),
        None => "submitted".to_string(),
    };
    let text = plan_md::render(run, &design.requirements);
    let mut doc = NewDoc::new(DocKind::Plan, author, &reason, &text);
    doc.disputed = opening.disputed;
    doc.not_reviewed = opening.not_reviewed;
    doc.same_runtime = opening.same_runtime;
    let n = match design_gate::open(run, doc, now, fx) {
        Ok(n) => n,
        Err(error) => return log(run, now, format!("the plan gate did not open: {error}")),
    };
    if let Some(design) = run.orch.design.as_mut() {
        design.plan_revision = None;
    }
    if opening.findings.is_empty() {
        return;
    }
    match store_findings(run, DocKind::Plan, n, &opening.findings) {
        Ok(write) => fx.push(write),
        Err(error) => log(run, now, format!("the plan v{n}'s findings: {error}")),
    }
}

/// The open plan gate the orchestrator does not revise: where the user's edits and the
/// engine's changes are versions of their own.
pub(in crate::run::engine) fn open_gate(run: &Run) -> bool {
    design_gate::waiting(run).is_some_and(|g| g.kind == DocGateKind::Plan && g.revising.is_none())
}

/// Ruling T7-7 and DF §5.1: the user's `run edit` at the open plan gate, whose tasks
/// were `before` (`None`: not at that gate). When it changed `plan.md`, that is the next
/// version, authored by the user; the gate stays open at it. The reply's `text` then
/// names the version, and approval's wait while the plan fails its checks.
pub(in crate::run::engine) fn user_edited(
    run: &mut Run,
    before: Option<&[Task]>,
    text: String,
    now: u64,
    fx: &mut Vec<Effect>,
) -> String {
    if before.is_none_or(|tasks| tasks == run.tasks) {
        return text;
    }
    let Some(n) = new_version(run, DocAuthor::User, USER_EDIT, now, fx) else {
        return text;
    };
    let mut text = format!("{text}; the plan is now v{n}");
    if let Some(why) = approve_refusal(run) {
        text = format!("{text}; approve waits: {why}");
    }
    text
}

/// Every step, for a design run (`engine::step`): a plan gate a sub-planner left for
/// planning is cleared, its note kept for the next version (ruling T7-4's carry); and a
/// change the engine made at the open plan gate is `plan.md`'s next version, authored by
/// the engine. It is compared with the gate version's text, so a run whose text a
/// restore has not read back yet is left as it is.
pub(in crate::run::engine) fn pass(run: &mut Run, now: u64, fx: &mut Vec<Effect>) {
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    if run.state == RunState::Planning
        && let Some(gate) = design.gate.take_if(|g| g.kind == DocGateKind::Plan)
    {
        design.plan_revision = gate.revising.or(design.plan_revision.take());
    }
    if !open_gate(run) {
        return;
    }
    // Ruling T11-3 (m1): `plan.md` is rendered again only when a size, a route or a
    // test mode changed since the last pass.
    let print = fingerprint(run);
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    if design.plan_fingerprint.as_ref() == Some(&print) {
        return;
    }
    design.plan_fingerprint = Some(print);
    new_version(run, DocAuthor::Engine, ENGINE_UPDATE, now, fx);
}

/// Each live task's id, size, route and test mode: what the engine can change at the
/// gate (ruling T11-3, m1).
fn fingerprint(run: &Run) -> String {
    let live = run.tasks.iter().filter(|t| t.state != TaskState::Cancelled);
    let lines = live.map(|t| format!("{} {:?} {:?} {:?}\n", t.id(), t.size, t.route, t.test_mode));
    lines.collect()
}

/// Ruling T11-1 and T11-3 (m4): a design run's `spawn_subplanner` waits for the
/// approved spec's read-back, and then names the requirement ids its epic owns, each
/// one the spec has.
pub(in crate::run::engine) fn spawn_refusal(
    run: &Run,
    epic: &str,
    covers: &[String],
) -> Option<String> {
    let design = run.orch.design.as_ref()?;
    if design.approved_spec.is_some() && design.requirements.is_empty() {
        return Some(SPAWN_PENDING.into());
    }
    if design.requirements.is_empty() {
        return None;
    }
    if covers.is_empty() {
        return Some(NEEDS_COVERS.into());
    }
    let unknown = (covers.iter()).find(|c| !design.requirements.iter().any(|r| &r.id == *c))?;
    Some(format!(
        "epic {epic} covers {unknown}, which the spec does not have"
    ))
}

/// `plan.md`'s next version at the open gate, when its text differs from the gate
/// version's: its number.
fn new_version(
    run: &mut Run,
    author: DocAuthor,
    reason: &str,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Option<u32> {
    let design = run.orch.design.as_ref()?;
    let version = design_gate::waiting(run)?.version;
    let text = plan_md::render(run, &design.requirements);
    // Ruling T11-3 (m5): compared with the stored version's SHA-256, so a restore that
    // has not read the gate's text back yet stores no identical version.
    let stored = design.find(DocKind::Plan, Some(version));
    if stored.is_some_and(|v| v.sha256 == sha256_hex(text.as_bytes())) {
        return None;
    }
    let doc = NewDoc::new(DocKind::Plan, author, reason, &text);
    match design_gate::open(run, doc, now, fx) {
        Ok(n) => Some(n),
        Err(error) => {
            log(run, now, format!("the plan's next version: {error}"));
            None
        }
    }
}

/// Whether the plan review the orchestrator's submit started is still under way (the
/// `edit_plan` reply's `awaiting_review`, decision 20).
pub(in crate::run::engine) fn awaiting_review(run: &Run) -> bool {
    run.state == RunState::Planning
        && reviewer::in_progress(run).is_some_and(|(doc, _)| doc == DocKind::Plan)
}

/// The orchestrator's digest (`run_status`): the plan review its next submit answers,
/// while the plan has no gate version yet: running, its findings, or its failure.
pub(crate) fn digest(run: &Run) -> Option<Value> {
    let design = run.orch.design.as_ref()?;
    if run.state != RunState::Planning || design.gate_versions(DocKind::Plan) > 0 {
        return None;
    }
    let review = (design.reviews.iter().rev()).find(|r| r.doc == DocKind::Plan)?;
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
