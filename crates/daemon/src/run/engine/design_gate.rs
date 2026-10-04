//! Milestone 9.6 (DF §2.1), engine side II: the one gate record of a design run's three
//! gates, opened by a stored version ([`open`]), and the user's six actions on it
//! ([`act`], decision 7) with their wake notes. A plain `run approve` at the brainstorm
//! or spec gate is refused (ruling T1-O1); at the plan gate it is the plan gate's
//! approve, and a plain `run reject` is the gate's reject. Brief ruling BD-2 caps a
//! gate at 6 versions and the brainstorm at 3 rethinks. Pure (design decision 2).
//!
//! The snapshot shows the gate (`snapshot_design.rs`, the Alerts' data) and the
//! orchestrator's digest names it ([`digest`]).

use proto::{DocAuthor, DocGateAction, DocGateKind, DocKind, RunState, safe_text};
use serde_json::{Value, json};

use super::design::{checked_text, start_clock};
use super::requests::log;
use super::{Effect, EngineState, ReplyId, goal_rounds_end, wake};
use crate::run::design::changes;
use crate::run::design::plan_md;
use crate::run::design::state::{DocGate, NewDoc, Revision, gate_doc, not_design, store};
use crate::run::model::Run;

/// Brief ruling BD-2: a gate's versions at most.
pub const MAX_VERSIONS: u32 = 6;
/// Brief ruling BD-2: the brainstorm's rethinks at most.
pub const MAX_RETHINKS: u32 = 3;
/// A user's note is kept to this many bytes.
pub const NOTE_MAX: usize = 8 * 1024;
/// A note's head in a version's reason.
const NOTE_HEAD_CHARS: usize = 60;

/// `RunRequest::DocGate`: [`act`] on run `run_id`, answered.
pub(super) fn request(
    state: &mut EngineState,
    reply: ReplyId,
    (run_id, kind, action): (&str, DocGateKind, DocGateAction),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let result = match state.runs.get_mut(run_id) {
        Some(run) => act(run, kind, action, now, fx),
        None => Err(format!("unknown run {run_id}")),
    };
    fx.push(Effect::Reply { reply, result });
}

/// The gate a design run waits at, if it does.
pub(crate) fn waiting(run: &Run) -> Option<&DocGate> {
    let gate = run.orch.design.as_ref()?.gate.as_ref()?;
    (run.state == RunState::AwaitingApproval).then_some(gate)
}

/// The note the orchestrator is revising the open gate's version with, if it is.
pub(super) fn revising(run: &Run) -> Option<String> {
    waiting(run).and_then(|g| g.revising.clone())
}

/// Ruling T1-O1: a plain `run approve` (or the action menu's) at the brainstorm or
/// spec gate.
pub(crate) fn plain_approve_refusal(run: &Run) -> Option<String> {
    let gate = waiting(run).filter(|g| g.kind != DocGateKind::Plan)?;
    let (id, kind) = (&run.id, gate.kind.label());
    Some(format!(
        "run {id} is waiting at the {kind} gate; approve it with anthrex run approve {id} --gate {kind}"
    ))
}

/// Ruling T7-3: `run approve` at a design run's plan gate while the orchestrator
/// revises it, refused as the plan gate's approve is ([`refusal`]).
pub(crate) fn plan_approve_refusal(run: &Run) -> Option<String> {
    let gate = waiting(run).filter(|g| g.kind == DocGateKind::Plan && g.revising.is_some())?;
    refusal(run, gate.kind, &DocGateAction::Approve)
}

/// Ruling T7-4: the orchestrator changed the plan at the open plan gate (a sub-planner
/// started there, or `edit_plan`'s edits): the run plans again, the gate closes and the
/// planning clock starts, so the next passing submit opens the plan's next version. A
/// gate the orchestrator revises is left as it is: its next submit is that version
/// already. Returns whether the gate closed.
pub(super) fn plan_changed(run: &mut Run, now: u64) -> bool {
    if !waiting(run).is_some_and(|g| g.kind == DocGateKind::Plan && g.revising.is_none()) {
        return false;
    }
    if let Some(design) = run.orch.design.as_mut() {
        design.gate = None;
    }
    run.state = RunState::Planning;
    if let Some(o) = run.orch.orchestrator.as_mut() {
        o.plan_submitted = false;
    }
    start_clock(run, now);
    true
}

/// Whether the run is in a document phase: brainstorming or specifying (or paused
/// there), or at the brainstorm or spec gate. Its plan cannot be approved yet.
pub(crate) fn in_doc_phase(run: &Run) -> bool {
    let doc_state =
        |s: Option<RunState>| matches!(s, Some(RunState::Brainstorming | RunState::Specifying));
    let paused = run.state == RunState::Paused && doc_state(run.paused_from);
    let gate = waiting(run).is_some_and(|g| g.kind != DocGateKind::Plan);
    doc_state(Some(run.state)) || paused || gate
}

/// `ActionKind::ReviewDoc`'s refusal: listed only while a document awaits review at the
/// brainstorm or spec gate (decision 34; the plan gate has its own review screen).
pub(crate) fn review_doc_refusal(run: &Run) -> Option<String> {
    let open = waiting(run).is_some_and(|g| g.kind != DocGateKind::Plan);
    (!open).then(|| format!("run {} has no document waiting for review", run.id))
}

/// Decision 7: the user's `action` at the `kind` gate. Every refusal comes first and
/// changes nothing ([`refusal`]).
pub(crate) fn act(
    run: &mut Run,
    kind: DocGateKind,
    action: DocGateAction,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<String, String> {
    if let Some(text) = refusal(run, kind, &action) {
        return Err(text);
    }
    let n = waiting(run).map_or(0, |g| g.version);
    let what = format!("{} v{n}", kind.label());
    match action {
        DocGateAction::Approve => Ok(approve(run, kind, &what, now)),
        DocGateAction::Changes { note, review } => {
            let note = clean_note(&note);
            let revising = (note.clone(), Revision::Changes);
            set_revising(run, kind, Some(revising), review, now);
            log(
                run,
                now,
                format!("the user asked for changes to the {what}"),
            );
            let text = revision_note(kind, n, Revision::Changes, &note);
            wake::note(run, text);
            Ok(format!(
                "run {}: the orchestrator revises the {what}",
                run.id
            ))
        }
        DocGateAction::Edit { text } => {
            let text = checked_text(run, gate_doc(kind), &text, false)?;
            let doc = NewDoc::new(gate_doc(kind), DocAuthor::User, "edited by you", &text);
            let n = open(run, doc, now, fx)?;
            let note = format!("the user edited the {} (now v{n})", kind.label());
            wake::note(run, note);
            Ok(format!("run {}: the {} is now v{n}", run.id, kind.label()))
        }
        DocGateAction::Rethink { note } => {
            let note = clean_note(&note);
            if let Some(design) = run.orch.design.as_mut() {
                design.rethinks += 1;
                design.gate = None;
                design.phase_started = None;
            }
            run.state = RunState::Brainstorming;
            log(run, now, format!("the user asked to rethink the {what}"));
            let text = format!("the user asked to rethink the brainstorm: {note}");
            wake::note(run, text);
            Ok(format!("run {}: the brainstorm is rethought", run.id))
        }
        DocGateAction::Back { note } => {
            let note = clean_note(&note);
            let prev = previous(kind);
            set_revising(run, prev, Some((note.clone(), Revision::Back)), false, now);
            log(run, now, format!("the user went back from the {what}"));
            let text = revision_note(prev, n, Revision::Back, &note);
            wake::note(run, text);
            Ok(format!("run {}: back to the {}", run.id, prev.label()))
        }
        DocGateAction::Reject => {
            if let Some(design) = run.orch.design.as_mut() {
                design.gate = None;
                design.phase_started = None;
            }
            let note = format!("the user rejected the {}; run discarded", kind.label());
            Ok(super::requests::reject_run(run, &note, now, fx))
        }
    }
}

/// Decision 7's refusals, in order, changing nothing: the run's flow, an accept or
/// discard in flight, the action's own gates, the gate the run waits at, a revision
/// under way (only reject passes it), then brief ruling BD-2's caps.
pub(crate) fn refusal(run: &Run, kind: DocGateKind, action: &DocGateAction) -> Option<String> {
    let design = match run.orch.design.as_ref() {
        Some(design) => design,
        None => return Some(not_design(&run.id)),
    };
    if let Some(how) = super::dispatch::finishing_as(run) {
        return Some(format!("run {} is being {how}", run.id));
    }
    match (action, kind) {
        (DocGateAction::Rethink { .. }, DocGateKind::Spec | DocGateKind::Plan) => {
            return Some("rethink is only for the brainstorm gate".into());
        }
        (DocGateAction::Back { .. }, DocGateKind::Brainstorm) => {
            return Some("back is only for the spec and plan gates".into());
        }
        (DocGateAction::Edit { .. }, DocGateKind::Plan) => {
            return Some("the plan is edited task by task with anthrex run edit".into());
        }
        _ => {}
    }
    let Some(gate) = waiting(run).filter(|g| g.kind == kind) else {
        return Some(format!(
            "run {} is not waiting at the {} gate",
            run.id,
            kind.label()
        ));
    };
    if let Some(_note) = gate
        .revising
        .as_ref()
        .filter(|_| *action != DocGateAction::Reject)
    {
        return Some(format!(
            "the orchestrator is revising {} v{}; wait for it",
            kind.label(),
            gate.version
        ));
    }
    // Ruling T7-6: at the brainstorm gate the way on is a rethink, not a back.
    let full = |kind: DocGateKind| {
        let ways = match kind {
            DocGateKind::Brainstorm => "approve, rethink or reject",
            _ => "approve, go back or reject",
        };
        (design.gate_versions(gate_doc(kind)) >= MAX_VERSIONS).then(|| {
            let kind = kind.label();
            format!("the {kind} has had its {MAX_VERSIONS} versions; {ways}")
        })
    };
    match action {
        DocGateAction::Changes { .. } | DocGateAction::Edit { .. } => full(kind),
        DocGateAction::Back { .. } => full(previous(kind)),
        // A rethink's own cap: the brainstorm's rethinks (ruling T7-6's text offers it).
        DocGateAction::Rethink { .. } if design.rethinks >= MAX_RETHINKS => Some(format!(
            "the brainstorm has been rethought {MAX_RETHINKS} times; approve, change or reject"
        )),
        DocGateAction::Rethink { .. } | DocGateAction::Approve | DocGateAction::Reject => None,
    }
}

/// The gate a back from `kind` reopens.
fn previous(kind: DocGateKind) -> DocGateKind {
    match kind {
        DocGateKind::Plan => DocGateKind::Spec,
        _ => DocGateKind::Brainstorm,
    }
}

/// Decision 7's approve: the next phase starts, its clock with it (decision 8); at the
/// plan gate, the run starts as 9.5's approve starts it.
fn approve(run: &mut Run, kind: DocGateKind, what: &str, now: u64) -> String {
    if let Some(design) = run.orch.design.as_mut() {
        design.gate = None;
        design.phase_started = None;
    }
    wake::note(run, format!("the user approved the {what}"));
    log(run, now, format!("the user approved the {what}"));
    let next = match kind {
        DocGateKind::Brainstorm => RunState::Specifying,
        DocGateKind::Spec => RunState::Planning,
        DocGateKind::Plan => {
            run.state = RunState::Running;
            // Milestone 9.1 decision 46 and 9.3 decision 12, as `requests::approve`.
            goal_rounds_end::approved(run, "user", now);
            log(run, now, "approved by the user");
            return format!("run {} approved", run.id);
        }
    };
    run.state = next;
    start_clock(run, now);
    format!("run {}: the {what} is approved; {}", run.id, next.label())
}

/// The gate of `kind` waits with `revising` (a changes request, a back, or a read-back,
/// with its note), at its latest version; the phase it returns to restarts its clock
/// (decision 8).
pub(super) fn set_revising(
    run: &mut Run,
    kind: DocGateKind,
    revising: Option<(String, Revision)>,
    review: bool,
    now: u64,
) {
    let (note, cause) = revising.map_or((None, Revision::Changes), |(n, c)| (Some(n), c));
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    let version = design.find(gate_doc(kind), None).map_or(0, |v| v.n);
    design.gate = Some(DocGate {
        kind,
        version,
        opened_at: now,
        revising: note,
        review,
        cause,
    });
    run.state = RunState::AwaitingApproval;
    start_clock(run, now);
}

/// Stores `doc` as its gate's next version, with its change summary against the
/// version before it (task 5's carry), and opens the gate at it: the run waits, and the
/// phase's clock stops (gate waits never count, DF §2.2). Returns the version.
pub(super) fn open(
    run: &mut Run,
    mut doc: NewDoc,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<u32, String> {
    let kind = match doc.kind {
        DocKind::Brainstorm => DocGateKind::Brainstorm,
        DocKind::Spec => DocGateKind::Spec,
        DocKind::Plan => DocGateKind::Plan,
        DocKind::BrainstormDraft => return Err("a brainstorm draft opens no gate".into()),
    };
    let design = run
        .orch
        .design
        .as_ref()
        .ok_or_else(|| not_design(&run.id))?;
    if let Some((_, before)) = design.text_of(doc.kind) {
        doc.changes = changes::summary(before, &doc.text);
    }
    let text = doc.text.clone();
    let (version, write) = store(run, doc, now)?;
    fx.push(write);
    if let Some(design) = run.orch.design.as_mut() {
        design.keep_text(version.kind, version.n, text);
        design.gate = Some(DocGate {
            kind,
            version: version.n,
            opened_at: now,
            revising: None,
            review: false,
            cause: Revision::Changes,
        });
        design.phase_started = None;
    }
    run.state = RunState::AwaitingApproval;
    let text = format!("the {} v{} awaits the user", kind.label(), version.n);
    log(run, now, text);
    Ok(version.n)
}

/// A plan submitted in a design run (`edit_plan`'s or `run edit --submit`'s, by
/// `author`): the plan gate opens at `plan.md`'s next version (decision 21), the first
/// time and after each changes request; a submit at the open gate otherwise stores
/// nothing. `None`: no submit.
pub(super) fn plan_submitted(
    run: &mut Run,
    author: Option<DocAuthor>,
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let (Some(author), Some(design)) = (author, run.orch.design.as_ref()) else {
        return;
    };
    if run.state != RunState::AwaitingApproval {
        return;
    }
    let reason = match &design.gate {
        None => "submitted".to_string(),
        Some(g) if g.kind == DocGateKind::Plan => match &g.revising {
            Some(note) => format!("revised: {}", note_head(note)),
            None => return,
        },
        Some(_) => return,
    };
    let text = plan_md::render(run, &design.requirements);
    let doc = NewDoc::new(DocKind::Plan, author, &reason, &text);
    if let Err(error) = open(run, doc, now, fx) {
        log(run, now, format!("the plan gate did not open: {error}"));
    }
}

/// Review focus 1: a fresh orchestrator session (a lost window's handoff, a fresh
/// restart) gets the note of the revision it owes again, since its predecessor may have
/// read it, in the words it was first told (fix round 1, m3).
pub(super) fn renote(run: &mut Run) {
    let Some(gate) = waiting(run).cloned() else {
        return;
    };
    let Some(note) = gate.revising else {
        return;
    };
    let text = revision_note(gate.kind, gate.version, gate.cause, &note);
    wake::unnote(run, &text);
    wake::note(run, text);
}

/// The wake note of a revision of the `kind` gate's v`n`, by its cause.
pub(super) fn revision_note(kind: DocGateKind, n: u32, cause: Revision, note: &str) -> String {
    let what = format!("{} v{n}", kind.label());
    match cause {
        Revision::Changes => format!("the user asked for changes to the {what}: {note}"),
        Revision::Back => format!("the user went back to the {}: {note}", kind.label()),
        Revision::ReadBack => {
            format!("the {what} could not be read back after a restart; submit it again")
        }
    }
}

/// Review m6: the document of the phase a run is in (or paused in) before its plan:
/// brainstorming's brainstorm or specifying's spec.
pub(super) fn phase_doc(run: &Run) -> Option<DocGateKind> {
    let state = match run.state {
        RunState::Paused => run.paused_from?,
        state => state,
    };
    match state {
        RunState::Brainstorming => Some(DocGateKind::Brainstorm),
        RunState::Specifying => Some(DocGateKind::Spec),
        _ => None,
    }
}

/// The digest's document gate (`orch/digest.rs::gate`): its kind, version, the user's
/// note while the orchestrator revises, and the version's disputed findings.
pub(crate) fn digest(run: &Run) -> Option<Value> {
    let design = run.orch.design.as_ref()?;
    let gate = waiting(run)?;
    let version = design.find(gate_doc(gate.kind), Some(gate.version));
    let disputed: Vec<Value> = (version.into_iter())
        .flat_map(|v| &v.disputed)
        .map(|f| json!({"id": f.id, "text": f.text}))
        .collect();
    Some(json!({
        "kind": gate.kind.label(),
        "version": gate.version,
        "revising": gate.revising,
        "disputed": disputed,
    }))
}

/// A user's note: cleaned (control characters out, decision 4) and kept to
/// [`NOTE_MAX`] bytes.
fn clean_note(note: &str) -> String {
    let mut note = safe_text::multi_line(note.trim());
    if note.len() > NOTE_MAX {
        let mut cut = NOTE_MAX;
        while !note.is_char_boundary(cut) {
            cut -= 1;
        }
        note.truncate(cut);
    }
    note
}

/// A note's first line, cut to [`NOTE_HEAD_CHARS`] characters.
pub(super) fn note_head(note: &str) -> String {
    let line = note.lines().next().unwrap_or_default();
    line.chars().take(NOTE_HEAD_CHARS).collect()
}
