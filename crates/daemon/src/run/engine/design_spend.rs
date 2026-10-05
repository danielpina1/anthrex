//! Milestone 9.6 decision 32 and ruling T13-1 (task M9.6.13), engine side: what each
//! design phase spends, kept on `DesignState.spend` (`run/design/spend.rs`), and the
//! phase's history record, written when its gate is approved.
//!
//! - **The orchestrator's time.** Each stop of a phase's clock ([`stop_clock`]: its
//!   gate opens, its budget or a read-back halts it) adds the clock's run, its paused
//!   time left out as decision 8 counts it.
//! - **The design agents.** Each ended session of a brainstormer or a document
//!   reviewer ([`session_ended`]) adds its tool calls, tokens and active time (start to
//!   end) to its agent's sums in its phase; its outcome is its latest session's.
//! - **The record** ([`approved`]): `<run>/phase/<round>/<phase>/v<n>`, `n` the
//!   version approved, appended as any history line. A phase approved again after a
//!   back is a new record with everything since added (ruling T13-4), so a restart
//!   with its append pending cannot take it for the first; the refit keeps each phase's
//!   highest version.
//!
//! Pure (design decision 2).

use proto::{AgentRole, DocGateKind, DocKind, HistoryLine, RunState, TokenUsage};

use super::{Effect, history};
use crate::run::design::spend::{AgentSpend, OK, OVER_BUDGET};
use crate::run::design::state::DesignAgent;
use crate::run::model::{LogEntry, Run};
use crate::run::refit;

/// The phase whose clock runs: the run's own, or the revised gate's, or a paused
/// run's phase.
fn clock_phase(run: &Run) -> Option<&'static str> {
    let state = match run.state {
        RunState::Paused => run.paused_from?,
        state => state,
    };
    super::design::phase_of(run, state)
}

/// Decision 8's clock stops now; its run, less the time paused since it started, is
/// added to its phase's time.
pub(super) fn stop_clock(run: &mut Run, now: u64) {
    let phase = clock_phase(run);
    let (round, total) = (run.round(), run.paused_total(now));
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    let Some(started) = design.phase_started.take() else {
        return;
    };
    let paused = total.saturating_sub(design.phase_paused_base);
    let secs = now.saturating_sub(started).saturating_sub(paused);
    if let Some(phase) = phase {
        design.add_phase_secs(round, phase, secs);
    }
}

/// Design agent `agent`'s session ended at `now` with `calls` tool calls and `usage`;
/// `failure` is why it delivered nothing, `None` when it did. A failed session over
/// its budget (the scout machine's limits, `scout::machine`) is `over budget`.
pub(super) fn session_ended(
    run: &mut Run,
    agent: &DesignAgent,
    (calls, usage): (u32, &TokenUsage),
    failure: Option<&str>,
    now: u64,
) {
    let round = run.round();
    let Some(design) = run.orch.design.as_mut() else {
        return;
    };
    let (phase, budget) = match agent.role {
        AgentRole::DocReviewer => {
            let doc = design.reviews.last().map(|r| r.doc);
            let phase = match doc {
                Some(DocKind::Plan) => "planning",
                _ => "specifying",
            };
            (phase, run.limits.orch.design.doc_reviewer)
        }
        _ => ("brainstorming", run.limits.orch.design.brainstormer),
    };
    let secs = now.saturating_sub(agent.started.unwrap_or(now));
    let over = u64::from(calls) * 2 >= u64::from(budget.tool_calls) * 3
        || secs >= u64::from(budget.minutes) * 60;
    let outcome = match failure {
        None => OK.to_string(),
        Some(_) if over => OVER_BUDGET.to_string(),
        Some(reason) => format!("failed: {reason}"),
    };
    let session = AgentSpend {
        label: agent.label.clone(),
        role: agent.role,
        route: agent.route.clone(),
        sessions: 0,
        calls,
        tokens: usage.input + usage.output + usage.cache_read + usage.cache_write,
        secs,
        outcome,
    };
    design.add_session(round, phase, session);
}

/// Decision 32: the user approved the `kind` gate at its v`n`, so its phase's record is
/// appended (a run without history writes none).
pub(super) fn approved(run: &mut Run, gate: (DocGateKind, u32), now: u64, fx: &mut Vec<Effect>) {
    if !crate::run::history::enabled(run) {
        return;
    }
    let Some(record) = crate::run::history::phase_record(run, gate, now) else {
        return;
    };
    history::append(run, None, HistoryLine::Phase(record), fx);
}

/// Ruling T13-5 (m5): a run entering the design flow takes the design classes' frozen
/// refit budgets, and its log and REPORT.md's `## Tuning` get their lines after the
/// start's, its `tuning: none (…)` line dropped when there are any.
pub(super) fn tuned(run: &mut Run) {
    let lines = run.limits.take_design_tuning();
    if lines.is_empty() {
        return;
    }
    let at = run.created_at;
    run.tuning_lines.retain(|l| !refit::is_none_line(l));
    run.log
        .retain(|e| !(e.at == at && refit::is_none_line(&e.text)));
    for text in lines {
        run.log.push(LogEntry {
            at,
            text: text.clone(),
        });
        run.tuning_lines.push(text);
    }
}
