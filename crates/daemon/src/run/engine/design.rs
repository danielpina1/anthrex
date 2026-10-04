//! Milestone 9.6 (DF §2), engine side I: a design run's phases. It enters
//! `brainstorming` where 9.5 enters `planning` ([`enter`]); the orchestrator's tools are
//! admitted by phase (ruling T1-O4, [`tool`]); `start_brainstorm` and the orchestrator's
//! `submit_doc` ([`submit_doc`]), whose version opens its gate (`design_gate.rs`); and
//! each orchestrator phase's wall-clock budget ([`tick`], decision 8), whose halt `run
//! resume` returns to its own phase ([`resume_phase`]); and, after a restore, what the
//! driver could not read back ([`checked`]). Pure (design decision 2).
//!
//! A phase's clock runs from when the phase starts (brainstorming: when the drafts are
//! in, [`drafts_in`]) to when its gate opens; a changes request or a back restarts it.
//! It leaves out the run's paused time (9.5's pause bookkeeping, `Run::paused_total`),
//! which a daemon restart's downtime is.

use proto::{DocAuthor, DocGateKind, DocKind, RunState, ToolCall, safe_text};
use serde_json::{Value, json};

use super::orch::refuse;
use super::requests::log;
use super::{Effect, ReplyId, design_gate, wake};
use crate::run::design::requirements;
use crate::run::design::state::{
    DesignAgentState, DesignState, NewDoc, Revision, gate_doc, not_design,
};
use crate::run::design::template::kind_name;
use crate::run::design::template::{self, TemplateCtx};
use crate::run::model::Run;
use crate::run::orch::tools::{OrchCall, SubmitDoc};

/// Decision 6: said once, at the start of a design run started with `--yes`.
pub const YES_LINE: &str = "design flow: --yes does not skip the brainstorm, spec or plan gates";

/// Until task M9.6.10 sends a spec draft to its review, a spec is submitted ready.
pub const SPEC_REVIEW_NOT_YET: &str =
    "a spec review draft (ready = false) is not available yet; submit with ready = true";

/// Decision 4: a design run enters `brainstorming` with an empty design state, where a
/// run without the flow enters `planning` (`orch::make_planned`). Decision 6: `--yes`
/// skips none of its gates, and the run's log says so once.
pub(crate) fn enter(run: &mut Run) {
    run.state = RunState::Brainstorming;
    run.orch.design = Some(DesignState::default());
    if run.orch.yes {
        let at = run.created_at;
        log(run, at, YES_LINE);
    }
}

/// The states sub-planners and run scouts start in (`planners::dispatch`): planning and
/// its two design phases, the gate, and running (decision 4: scouts run in
/// brainstorming as in planning).
pub(super) fn readers_start(state: RunState) -> bool {
    matches!(
        state,
        RunState::Brainstorming
            | RunState::Specifying
            | RunState::Planning
            | RunState::AwaitingApproval
            | RunState::Running
    )
}

/// Where a design run is before its plan may be touched, in ruling T1-O4's words;
/// `None` in planning, at the plan gate, past the design phases, or without the flow.
pub(super) fn before_planning(run: &Run) -> Option<&'static str> {
    let gate = run.orch.design.as_ref()?.gate.as_ref().map(|g| g.kind);
    match (run.state, gate) {
        (RunState::Brainstorming, _) => Some("brainstorming"),
        (RunState::Specifying, _) => Some("specifying"),
        (RunState::AwaitingApproval, Some(DocGateKind::Brainstorm)) => {
            Some("at the brainstorm gate")
        }
        (RunState::AwaitingApproval, Some(DocGateKind::Spec)) => Some("at the spec gate"),
        _ => None,
    }
}

/// Ruling T1-O4 and task 6's carry (m4): the orchestrator's call, admitted by phase,
/// before `orch::tool` routes it. A design tool on a run without the flow is refused;
/// a plan tool before planning is refused; `start_brainstorm` and `submit_doc` are
/// answered here. Anything else comes back to be routed as before.
pub(super) fn tool(
    run: &mut Run,
    reply: ReplyId,
    call: &ToolCall,
    parsed: OrchCall,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Option<OrchCall> {
    let design_tool = matches!(
        parsed,
        OrchCall::StartBrainstorm { .. }
            | OrchCall::SubmitDoc(_)
            | OrchCall::GetDoc { .. }
            | OrchCall::SubmitFindings { .. }
    );
    if run.orch.design.is_none() {
        if design_tool {
            refuse(fx, reply, not_design(&run.id));
            return None;
        }
        return Some(parsed);
    }
    let plan_tool = matches!(
        parsed,
        OrchCall::EditPlan { .. } | OrchCall::SpawnSubplanner { .. } | OrchCall::SubmitEpic { .. }
    );
    if let Some(at) = before_planning(run).filter(|_| plan_tool) {
        let text = format!("{} is for the planning phase; this run is {at}", call.tool);
        refuse(fx, reply, text);
        return None;
    }
    let result = match parsed {
        OrchCall::StartBrainstorm { answers } => start_brainstorm(run, &answers, now),
        OrchCall::SubmitDoc(doc) => submit_doc(run, doc, now, fx),
        other => return Some(other),
    };
    match result {
        Ok(value) => fx.push(Effect::Reply {
            reply,
            result: Ok(value.to_string()),
        }),
        Err(text) => refuse(fx, reply, text),
    }
    None
}

/// DF §2: the user's answers, and the brainstormers' launch (task M9.6.8), once, in
/// brainstorming only.
fn start_brainstorm(run: &mut Run, answers: &str, now: u64) -> Result<Value, String> {
    if run.state != RunState::Brainstorming {
        return Err("start_brainstorm is only for the brainstorming phase".into());
    }
    let id = run.id.clone();
    let design = run.orch.design.as_mut().ok_or_else(|| not_design(&id))?;
    let live = (design.brainstormers.iter()).any(|a| {
        matches!(
            a.state,
            DesignAgentState::Queued | DesignAgentState::Running | DesignAgentState::Submitted
        )
    });
    if design.answers.is_some() || live {
        return Err("the brainstormers are already running".into());
    }
    design.answers = Some(safe_text::multi_line(answers));
    log(run, now, "the orchestrator started the brainstorm");
    super::design_agents::queue_brainstormers(run, now);
    Ok(json!({"accepted": true}))
}

/// The orchestrator's `submit_doc`: the merged brainstorm report or the spec, admitted
/// in its phase or at its gate while the orchestrator revises (ruling T1-O4), checked
/// against its template (capped, then cleaned, task 6's carry f), and stored as the
/// gate's next version, which opens the gate.
pub(super) fn submit_doc(
    run: &mut Run,
    doc: SubmitDoc,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Result<Value, String> {
    let kind = match doc.kind {
        DocKind::Brainstorm => DocGateKind::Brainstorm,
        DocKind::Spec => DocGateKind::Spec,
        _ => return Err("the orchestrator submits the brainstorm report and the spec".into()),
    };
    admitted(run, kind)?;
    if kind == DocGateKind::Spec && !doc.ready {
        return Err(SPEC_REVIEW_NOT_YET.into());
    }
    let text = checked_text(run, doc.kind, &doc.text, doc.amend)?;
    let reason = match design_gate::revising(run) {
        Some(note) => format!("revised: {}", design_gate::note_head(&note)),
        None => "submitted".to_string(),
    };
    let doc = NewDoc::new(doc.kind, DocAuthor::Orchestrator, &reason, &text);
    let n = design_gate::open(run, doc, now, fx)?;
    Ok(json!({"accepted": true, "kind": kind.label(), "version": n, "awaiting_approval": true}))
}

/// `raw` as `kind`'s template admits it (capped on its raw bytes, then cleaned and
/// checked), and a spec's requirements numbered (decision 14): the text to store.
pub(super) fn checked_text(
    run: &Run,
    kind: DocKind,
    raw: &str,
    amend: bool,
) -> Result<String, String> {
    let text = template::admit(kind, raw, &template_ctx(run, true))?;
    if kind == DocKind::Spec {
        let approved = run.orch.design.as_ref().map(|d| &d.requirements[..]);
        match (amend, approved) {
            (true, Some(approved)) => requirements::parse_amendment(&text, approved)?,
            _ => requirements::parse(&text)?,
        };
    }
    Ok(text)
}

/// What the template check needs: the brainstormers' labels (from their drafts when
/// none is recorded), the one that failed, and `ready`.
fn template_ctx(run: &Run, ready: bool) -> TemplateCtx {
    let Some(design) = run.orch.design.as_ref() else {
        return TemplateCtx::default();
    };
    let mut labels: Vec<String> = design
        .brainstormers
        .iter()
        .map(|a| a.label.clone())
        .collect();
    if labels.is_empty() {
        let drafts = design.versions.iter().filter_map(|v| v.label());
        for label in drafts {
            if !labels.iter().any(|l| l == label) {
                labels.push(label.to_string());
            }
        }
    }
    let failed = design.brainstormers.iter().find_map(|a| match &a.state {
        DesignAgentState::Failed(reason) => Some((a.label.clone(), reason.clone())),
        _ => None,
    });
    TemplateCtx {
        labels,
        failed,
        ready,
    }
}

/// Ruling T1-O4: a document is submitted in its phase, or at its gate while the
/// orchestrator revises it.
fn admitted(run: &Run, kind: DocGateKind) -> Result<(), String> {
    let phase = match kind {
        DocGateKind::Brainstorm => RunState::Brainstorming,
        DocGateKind::Spec => RunState::Specifying,
        DocGateKind::Plan => RunState::Planning,
    };
    let gate = run.orch.design.as_ref().and_then(|d| d.gate.as_ref());
    let revising = gate.is_some_and(|g| g.kind == kind && g.revising.is_some());
    if run.state == phase || (run.state == RunState::AwaitingApproval && revising) {
        return Ok(());
    }
    let at = match gate.filter(|_| run.state == RunState::AwaitingApproval) {
        Some(g) => format!("at the {} gate", g.kind.label()),
        None => run.state.label().to_string(),
    };
    Err(format!(
        "submit_doc kind \"{}\" is for the {} phase; this run is {at}",
        gate_doc(kind).label(),
        phase.label()
    ))
}

/// DF §3.4, for task M9.6.8: both drafts are in, or one draft and one failure,
/// `(label, reason)`. The brainstorming clock starts now, and the orchestrator is woken
/// to merge them.
pub(super) fn drafts_in(run: &mut Run, failed: Option<(&str, &str)>, now: u64) {
    start_clock(run, now);
    let note = match failed {
        None => {
            "both brainstorm drafts are in; read them with get_doc and submit the merged report"
                .to_string()
        }
        Some((label, reason)) => format!(
            "one brainstormer failed ({label}: {reason}); read the other draft with get_doc and submit the merged report"
        ),
    };
    log(run, now, "the brainstorm drafts are in");
    wake::note(run, note);
}

/// Decision 8: the current phase's clock starts (or restarts) now.
pub(super) fn start_clock(run: &mut Run, now: u64) {
    let base = run.paused_total(now);
    if let Some(design) = run.orch.design.as_mut() {
        design.phase_started = Some(now);
        design.phase_paused_base = base;
    }
}

/// The phase whose clock runs, as the halt names it: the run's phase, or the phase of
/// the gate the orchestrator is revising.
fn phase_name(run: &Run) -> Option<&'static str> {
    phase_of(run, run.state)
}

/// The design phase of a run in `state`: its own, or at a revising gate its gate's.
fn phase_of(run: &Run, state: RunState) -> Option<&'static str> {
    let design = run.orch.design.as_ref()?;
    match state {
        RunState::Brainstorming => Some("brainstorming"),
        RunState::Specifying => Some("specifying"),
        RunState::Planning => Some("planning"),
        RunState::AwaitingApproval => {
            let gate = design.gate.as_ref().filter(|g| g.revising.is_some())?;
            Some(match gate.kind {
                DocGateKind::Brainstorm => "brainstorming",
                DocGateKind::Spec => "specifying",
                DocGateKind::Plan => "planning",
            })
        }
        _ => None,
    }
}

/// Ruling T7-1: the phase a halted design run returns to on its plain `run resume`
/// (`halted_from`), which `rules::resume` names when it refuses `--rebaseline`.
pub(crate) fn halted_phase(run: &Run) -> Option<&'static str> {
    if run.state != RunState::Halted {
        return None;
    }
    let from = run.orch.design.as_ref()?.halted_from?;
    phase_of(run, from)
}

/// Decision 8, on every tick: a phase past its `phase_minutes` of unpaused wall-clock
/// time halts the run, retryably; `run resume` returns it to the state it left.
pub(super) fn tick(run: &mut Run, now: u64) {
    let Some(started) = run.orch.design.as_ref().and_then(|d| d.phase_started) else {
        return;
    };
    let Some(phase) = phase_name(run) else {
        return;
    };
    let base = run.orch.design.as_ref().map_or(0, |d| d.phase_paused_base);
    let paused = run.paused_total(now).saturating_sub(base);
    let spent = now.saturating_sub(started).saturating_sub(paused);
    let minutes = run.limits.orch.design.phase_minutes;
    if spent <= u64::from(minutes) * 60 {
        return;
    }
    let text = format!("design flow: the {phase} phase passed its {minutes} min budget");
    if let Some(design) = run.orch.design.as_mut() {
        design.halted_from = Some(run.state);
        design.phase_started = None;
    }
    // Fix round 1 (m1): halted as every halt is (its log line and wake note).
    super::merge::halt(run, text, now);
    run.halt_retryable = true;
}

/// Ruling T7-2, after a restore: a daemon restart's downtime never counts toward a
/// phase's budget. A run the restore left stopped (paused, or halted) has it as paused
/// time already (ruling T12-1, `Run::paused_total`); any other whose clock runs (a
/// revising gate, which the restore does not pause) has its clock shifted by it, from
/// its last change as T12-1 counts it.
pub(super) fn restored(run: &mut Run, now: u64) {
    if matches!(run.state, RunState::Paused | RunState::Halted) || run.last_step_at == 0 {
        return;
    }
    let down = now.saturating_sub(run.last_step_at);
    let started = run
        .orch
        .design
        .as_mut()
        .and_then(|d| d.phase_started.as_mut());
    if let Some(started) = started {
        *started = started.saturating_add(down).min(now);
    }
}

/// F-3 (task M9.6.1): a plain `run resume` of a run a phase budget halted returns it to
/// the state it left, and the phase's clock restarts. `None`: not such a run.
pub(super) fn resume_phase(run: &mut Run, now: u64) -> Option<String> {
    if run.state != RunState::Halted {
        return None;
    }
    let from = run.orch.design.as_mut()?.halted_from.take()?;
    run.state = from;
    run.halted_reason = None;
    run.halt_retryable = false;
    // DF §3.5: both brainstormers failed; they relaunch, and the clock waits for them.
    let text = match super::design_agents::relaunch_failed(run, now) {
        true => "resumed; the brainstormers are relaunched",
        false => {
            start_clock(run, now);
            "resumed; the phase's clock restarts"
        }
    };
    log(run, now, text);
    Some(format!("run {} resumed", run.id))
}

/// One indexed version as the driver read it back at a restore (task 5's carry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocChecked {
    pub kind: DocKind,
    pub n: u32,
    /// Its text when it is the latest version of a gate's document (what the next
    /// version's change summary compares against), `None` for any other; `Err` when
    /// its file is missing or differs from the index.
    pub read: Result<Option<String>, String>,
}

/// After a restore: every version whose file is missing or changed is logged, once;
/// the open gate's version among them reopens the gate as revising, and the
/// orchestrator is woken to submit it again, so that version is never shown. The
/// latest texts refill the change summaries' cache.
pub(super) fn checked(run: &mut Run, checked: Vec<DocChecked>, now: u64) {
    let gate = design_gate::waiting(run).cloned();
    let mut lost = false;
    for doc in checked {
        match doc.read {
            Ok(Some(text)) => {
                if let Some(design) = run.orch.design.as_mut() {
                    design.keep_text(doc.kind, doc.n, text);
                }
            }
            Ok(None) => {}
            Err(reason) => {
                let what = format!("{} v{}", kind_name(doc.kind), doc.n);
                log(
                    run,
                    now,
                    format!("design flow: the {what} could not be read back: {reason}"),
                );
                lost |= gate
                    .as_ref()
                    .is_some_and(|g| gate_doc(g.kind) == doc.kind && g.version == doc.n);
            }
        }
    }
    let Some(gate) = gate.filter(|g| lost && g.revising.is_none()) else {
        return;
    };
    // Fix round 1 (m3): the engine's note, never the user's.
    let what = format!("{} v{}", gate.kind.label(), gate.version);
    let note = format!("anthrex could not read back the stored {what}; submit it again");
    let revising = Some((note.clone(), Revision::ReadBack));
    design_gate::set_revising(run, gate.kind, revising, false, now);
    let cause = Revision::ReadBack;
    wake::note(
        run,
        design_gate::revision_note(gate.kind, gate.version, cause, &note),
    );
}
