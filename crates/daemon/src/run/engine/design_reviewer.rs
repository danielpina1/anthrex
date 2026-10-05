//! Milestone 9.6 task M9.6.10 (DF §4.2, §4.3; decisions 9, 10 and 15), part of
//! `design_agents.rs`: the document reviewer. One per review, queued by a spec's review
//! draft ([`queue`]) on the orchestrator's peer runtime, else its own (`same_runtime`),
//! it takes a reader slot as a brainstormer does and starts while its review is asked
//! (specifying, or the spec gate the orchestrator revises; [`start_next`]). Task
//! M9.6.11: the plan's one review too, queued by the first passing plan submit and
//! asked in planning. Its one
//! write, `submit_findings`, is taken only from its live window ([`tool`]); the
//! findings wake the orchestrator with the exact note. Its session runs unsaved (ruling
//! T8-4, keyed on its MCP role) and is never resumed: an end without its findings is
//! relaunched fresh once, a second one fails it (ruling T8-7, [`ended`]). A failure is
//! the review's `not reviewed: <reason>`. A restart relaunches a running reviewer fresh
//! ([`restore`]). Pure (design decision 2).

use proto::{AgentRole, DocFinding, DocKind, DocSeverity, RoleOutcome, RunState, ToolCall};
use proto::{TokenUsage, safe_text};
use serde_json::{Value, json};

use super::super::orch::refuse;
use super::super::requests::log;
use super::super::{
    Effect, OpKind, OpResult, ReplyId, ScoutEnd, design_gate, emit_op, history, next_op, wake,
};
use super::relaunch::{self, Agent};
use crate::decider::DECIDER_CAPS;
use crate::run::design::state::{DesignAgent, DesignAgentState, DesignState, DocReviewRecord};
use crate::run::model::Run;
use crate::run::orch::roles;
use crate::run::orch::roles::lists::review_pick;
use crate::run::orch::tools::{OrchCall, parse_call};
use crate::scout::design_spec::{DesignAgentSpec, reviewer_spec};

/// A reviewer's session that ended with no findings and no failure of its own.
pub const NO_FINDINGS: &str = "the document reviewer ended without accepted findings";
/// A reviewer's record's result once its findings were accepted.
const FINDINGS_ACCEPTED: &str = "findings accepted";

/// Review `k` of `doc`'s reviewer label, its `--agent-label` (ruling T1-O3): `spec-r1`.
pub fn label(doc: DocKind, k: u32) -> String {
    format!("{}-r{k}", doc.label())
}

/// The review the reviewer works on: the last one asked.
fn review(design: &DesignState) -> Option<&DocReviewRecord> {
    design.reviews.last()
}

/// Review `k` of `doc`, asked when the document had `after` gate versions: its record,
/// and its reviewer queued for a reader slot (decision 10's route, [`review_pick`]). A
/// reviewer on the orchestrator's own runtime is said so in the log, with why (fix
/// round 1, m1).
pub(in crate::run::engine) fn queue(run: &mut Run, doc: DocKind, (k, after): (u32, u32), now: u64) {
    let (route, why) = review_pick(run, &DECIDER_CAPS);
    let same_runtime = why.is_some();
    let label = label(doc, k);
    if let Some(why) = why {
        let runtime = route.runtime.label();
        let text = format!(
            "the document reviewer {label}: no peer reviewer: {why}; reviewing on the same runtime, {runtime}"
        );
        log(run, now, text);
    }
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    design.reviews.push(DocReviewRecord {
        doc,
        n: k,
        findings: Vec::new(),
        failed: None,
        after,
        same_runtime,
        dropped: false,
    });
    design.reviewer = Some(DesignAgent {
        label: label.clone(),
        role: AgentRole::DocReviewer,
        route,
        session: 0,
        window_id: None,
        state: DesignAgentState::Queued,
        calls: 0,
        tokens: 0,
        started: None,
        listed: false,
        unsubmitted: false,
    });
    log(run, now, format!("document reviewer {label} queued"));
}

/// The review whose reviewer is queued or running, if one is: `(doc, k)`.
pub(in crate::run::engine) fn in_progress(run: &Run) -> Option<(DocKind, u32)> {
    let design = run.orch.design.as_ref()?;
    let agent = design.reviewer.as_ref()?;
    let live = matches!(
        agent.state,
        DesignAgentState::Queued | DesignAgentState::Running
    );
    live.then(|| review(design).map(|r| (r.doc, r.n)))?
}

/// Whether the run is where its review is asked: specifying, or at the spec gate the
/// orchestrator revises; the plan's (task M9.6.11), planning.
fn asked(run: &Run) -> bool {
    let doc = run.orch.design.as_ref().and_then(review).map(|r| r.doc);
    match (doc, run.state) {
        (Some(DocKind::Plan), state) => state == RunState::Planning,
        (_, RunState::Specifying) => true,
        (_, RunState::AwaitingApproval) => design_gate::waiting(run)
            .is_some_and(|g| g.kind == proto::DocGateKind::Spec && g.revising.is_some()),
        _ => false,
    }
}

/// A queued reviewer starts in a free reader slot while its review is asked.
pub(super) fn start_next(run: &mut Run, now: u64, fx: &mut Vec<Effect>) -> bool {
    let queued = (run.orch.design.as_ref())
        .and_then(|d| d.reviewer.as_ref())
        .is_some_and(|a| a.state == DesignAgentState::Queued);
    if !queued || !asked(run) {
        return false;
    }
    let op = next_op(run);
    let asked = (run.orch.design.as_ref()).and_then(review);
    let (doc, k) = asked.map_or((DocKind::Spec, 0), |r| (r.doc, r.n));
    let Some(agent) = run.orch.design.as_mut().and_then(|d| d.reviewer.as_mut()) else {
        return false;
    };
    agent.session += 1;
    agent.state = DesignAgentState::Running;
    agent.window_id = None;
    agent.started = Some(now);
    let agent = agent.clone();
    let spec = reviewer_spec(run, &agent, (doc, k));
    let text = format!(
        "document reviewer {} session {} starting",
        agent.label, agent.session
    );
    log(run, now, text);
    history::open(run, roles::design_agent_record(run, &agent, false, now));
    let kind = OpKind::StartDesignAgent {
        spec: Box::new(spec),
    };
    emit_op(run, op, None, kind, fx);
    true
}

/// The reviewer, when `spec` launched its current session and it still runs.
fn current<'a>(run: &'a mut Run, label: &str, session: u32) -> Option<&'a mut DesignAgent> {
    let agent = run.orch.design.as_mut()?.reviewer.as_mut()?;
    (agent.label == label && agent.session == session).then_some(agent)
}

/// `StartDesignAgent`'s result for a reviewer: its window, or its failure. A window
/// whose reviewer no longer runs is stopped.
pub(super) fn started(
    run: &mut Run,
    spec: &DesignAgentSpec,
    result: OpResult,
    now: u64,
    fx: &mut Vec<Effect>,
) -> Option<u32> {
    let (label, session) = (spec.kind.label(), spec.session);
    let live =
        (current(run, &label, session)).is_some_and(|a| a.state == DesignAgentState::Running);
    match result {
        OpResult::DesignAgentStarted { window_id, .. } if live => {
            if let Some(agent) = current(run, &label, session) {
                agent.window_id = Some(window_id);
            }
            Some(window_id)
        }
        OpResult::DesignAgentStarted { window_id, .. } => {
            let reason = "the document reviewer is no longer running".to_string();
            fx.push(Effect::StopPlanner { window_id, reason });
            None
        }
        OpResult::Failed { message } => {
            let why = format!("the document reviewer could not start: {message}");
            let record = format!("{label}/{session}");
            let failed = (RoleOutcome::Failed, Some(why.clone()));
            history::close_session(run, (AgentRole::DocReviewer, &record), failed, fx);
            if live {
                fail(run, why, now);
            }
            None
        }
        _ => None,
    }
}

/// The reviewer's session ended: its usage and calls are counted and its record
/// finished; a reviewer still running fails, or, ended unnudged without its findings
/// (ruling T8-7), is relaunched fresh once.
pub(super) fn ended(
    run: &mut Run,
    (label, session): (&str, u32),
    (outcome, usage, calls): (ScoutEnd, TokenUsage, u32),
    now: u64,
    fx: &mut Vec<Effect>,
) {
    let Some(agent) = current(run, label, session) else {
        return;
    };
    // Ruling T13-1: its calls are summed over its sessions, as its tokens are.
    agent.tokens += usage.input + usage.output + usage.cache_read + usage.cache_write;
    agent.calls += calls;
    let (done, running) = (
        agent.state == DesignAgentState::Done,
        agent.state == DesignAgentState::Running,
    );
    let ended = agent.clone();
    let (reason, unsubmitted) = match outcome {
        ScoutEnd::Failed { reason } => (reason, false),
        ScoutEnd::Unsubmitted { reason } => (reason, true),
        ScoutEnd::Reported => (NO_FINDINGS.to_string(), false),
    };
    let failure = (!done).then_some(reason.as_str());
    super::super::design_spend::session_ended(run, &ended, (calls, &usage), failure, now);
    let closed = match done {
        true => (RoleOutcome::Completed, Some(FINDINGS_ACCEPTED.to_string())),
        false => (RoleOutcome::Failed, Some(reason.clone())),
    };
    let record = format!("{label}/{session}");
    history::close_session(run, (AgentRole::DocReviewer, &record), closed, fx);
    // Ruling T8-7: one fresh relaunch, with the same draft and the resubmit line.
    if running && unsubmitted {
        return relaunch::once(run, Agent::Reviewer, now, fx);
    }
    if running {
        fail(run, reason, now);
    }
}

/// DF §4.3: the reviewer fails with `reason`, its review records it, and the
/// orchestrator is woken: its next `ready` submit opens the gate not reviewed.
pub(super) fn fail(run: &mut Run, reason: String, now: u64) {
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    let Some(agent) = design.reviewer.as_mut() else {
        return;
    };
    agent.state = DesignAgentState::Failed(reason.clone());
    let label = agent.label.clone();
    let Some(record) = design.reviews.last_mut() else {
        return;
    };
    record.failed = Some(reason.clone());
    let (kind, doc, k) = (record.doc, record.doc.label(), record.n);
    log(
        run,
        now,
        format!("document reviewer {label} failed: {reason}"),
    );
    let note = match kind {
        // Task M9.6.11: the plan is submitted with edit_plan.
        DocKind::Plan => format!(
            "plan review {k} failed: {reason}; submit the plan again with edit_plan, and its gate opens not reviewed"
        ),
        _ => format!(
            "{doc} review {k} failed: {reason}; submit the {doc} with ready = true, and its gate opens not reviewed"
        ),
    };
    wake::note(run, note);
}

/// The reviewer's tool, after `orch::tool`'s run-state gate: from its live window only,
/// its one write, `submit_findings`. A run being ended takes none.
pub(super) fn tool(
    run: &mut Run,
    reply: ReplyId,
    call: &ToolCall,
    (ending, now): (Option<String>, u64),
    fx: &mut Vec<Effect>,
) {
    if let Some(text) = ending {
        return refuse(fx, reply, text);
    }
    let parsed = match parse_call(call.role, &call.tool, &call.args) {
        Ok(parsed) => parsed,
        Err(text) => return refuse(fx, reply, text),
    };
    let design = run.orch.design.as_ref();
    let live = design.and_then(|d| d.live_agent(call.role, call.window_id));
    if live.is_none() {
        let text = format!("this window is not the document reviewer of run {}", run.id);
        return refuse(fx, reply, text);
    }
    match parsed {
        OrchCall::SubmitFindings { findings } => {
            let value = findings_in(run, findings, now);
            fx.push(Effect::Reply {
                reply,
                result: Ok(value.to_string()),
            });
            fx.push(Effect::PlannerAccepted {
                window_id: call.window_id,
            });
        }
        _ => refuse(fx, reply, "a document reviewer submits its findings"),
    }
}

/// Decision 15: the review's findings, cleaned (Review focus 4), stored with its record;
/// the reviewer is done, and the orchestrator is woken with the exact note.
fn findings_in(run: &mut Run, findings: Vec<DocFinding>, now: u64) -> Value {
    let findings: Vec<DocFinding> = (findings.into_iter())
        .map(|f| DocFinding {
            place: safe_text::one_line(&f.place),
            text: safe_text::multi_line(&f.text),
            ..f
        })
        .collect();
    let n = findings.len();
    let b = (findings.iter())
        .filter(|f| f.severity == DocSeverity::Blocking)
        .count();
    let Some(design) = run.orch.design.as_mut() else {
        return json!({"accepted": false});
    };
    let Some(agent) = design.reviewer.as_mut() else {
        return json!({"accepted": false});
    };
    agent.state = DesignAgentState::Done;
    let record = format!("{}/{}", agent.label, agent.session);
    let Some(review) = design.reviews.last_mut() else {
        return json!({"accepted": false});
    };
    review.findings = findings;
    let (kind, doc, k) = (review.doc, review.doc.label(), review.n);
    history::note_result(run, (AgentRole::DocReviewer, &record), FINDINGS_ACCEPTED);
    log(
        run,
        now,
        format!("{doc} review {k} is in: {n} findings ({b} blocking)"),
    );
    let note = match kind {
        // Decision 20 (task M9.6.11): the plan's exact note.
        DocKind::Plan => format!(
            "plan review is in: {n} findings ({b} blocking); answer each in edit_plan's responses when you submit"
        ),
        _ => format!(
            "{doc} review {k} is in: {n} findings ({b} blocking); answer each in your next submit_doc"
        ),
    };
    wake::note(run, note);
    json!({"accepted": true, "findings": n})
}

/// Decision 9 after a daemon restart (DF §8.4): a running reviewer is queued again and
/// relaunched fresh, as a new session on the same draft (its one relaunch, if it was
/// that, stays its one relaunch).
pub(super) fn restore(run: &mut Run, now: u64) {
    let agent = run.orch.design.as_mut().and_then(|d| d.reviewer.as_mut());
    let Some(agent) = agent.filter(|a| a.state == DesignAgentState::Running) else {
        return;
    };
    agent.state = DesignAgentState::Queued;
    agent.window_id = None;
    let text = format!(
        "document reviewer {} relaunches after a daemon restart",
        agent.label
    );
    log(run, now, text);
}

/// `planners::halt_all`, and a rejected run: a live reviewer is stopped, a queued one
/// never starts; it fails with `reason`, and its review with it.
pub(super) fn halt_all(run: &mut Run, reason: &str, fx: &mut Vec<Effect>) {
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    let Some(agent) = design.reviewer.as_mut() else {
        return;
    };
    let was = agent.state.clone();
    if !matches!(was, DesignAgentState::Running | DesignAgentState::Queued) {
        return;
    }
    agent.state = DesignAgentState::Failed(reason.to_string());
    let (window, session) = (
        agent.window_id,
        format!("{}/{}", agent.label, agent.session),
    );
    if let Some(review) = design.reviews.last_mut() {
        review.failed.get_or_insert_with(|| reason.to_string());
    }
    if was != DesignAgentState::Running {
        return;
    }
    if let Some(window_id) = window {
        let reason = reason.to_string();
        fx.push(Effect::StopPlanner { window_id, reason });
    }
    history::session_stopped(run, (AgentRole::DocReviewer, &session), fx);
}
