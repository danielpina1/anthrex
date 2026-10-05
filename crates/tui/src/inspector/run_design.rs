//! Milestone 9.6 ruling T18-1: the run view's inspection of a design agent, a
//! brainstormer or a document reviewer: its state, runtime, the document and review it
//! answers (a reviewer), its session count and its window.

use super::run_format::{clean, kind_glyph, rows, session_text};
use super::{Inspection, field};
use crate::app::App;
use crate::tree::RowKind;
use proto::{AgentRole, DesignAgentInfo, DesignAgentStatus, DocGateKind, RunInfo, WindowInfo};

/// The agent's state as its inspection and the run status name it.
pub(crate) fn design_state_word(state: DesignAgentStatus) -> &'static str {
    match state {
        DesignAgentStatus::Queued => "queued",
        DesignAgentStatus::Running => "running",
        DesignAgentStatus::Submitted => "submitted",
        DesignAgentStatus::Done => "done",
        DesignAgentStatus::Failed => "failed",
    }
}

/// Ruling T18-5 (final fix wave FW-57): the `gate` line at a design gate, the gate's
/// kind and version then the keys its screen offers (the brainstorm and spec gates' document
/// screen, the plan gate's plan review); while the orchestrator revises, only the
/// reject. `None` for a run not at a design gate.
pub(crate) fn design_gate_keys(run: &RunInfo) -> Option<String> {
    let gate = run.doc_gate.as_ref()?;
    let head = format!("{} v{}", gate.kind.label(), gate.version);
    if gate.revising.is_some() {
        return Some(format!("{head} being revised · x reject"));
    }
    let keys = match gate.kind {
        DocGateKind::Brainstorm => {
            "a approve · c changes · e edit · r rethink · x reject · g drafts"
        }
        DocGateKind::Spec => "a approve · c changes · e edit · b back · x reject",
        DocGateKind::Plan => "a approve · c changes · e edit · b back · x reject · d remove",
    };
    Some(format!("{head} · {keys}"))
}

pub(crate) fn design_agent_inspection(
    run: &RunInfo,
    agent: &DesignAgentInfo,
    window: Option<&WindowInfo>,
    app: &App,
) -> Inspection {
    let state = design_state_word(agent.state);
    let mut state_value = state.to_owned();
    if let Some(tool) = window
        .and_then(|window| window.tool.as_deref())
        .filter(|_| agent.state == DesignAgentStatus::Running)
    {
        state_value.push_str(&format!(" · {}", clean(tool)));
    }
    let mut fields = vec![
        field("state", state_value),
        field("runtime", agent.runtime.label()),
    ];
    if let (Some(doc), Some(review)) = (agent.doc, agent.review) {
        fields.push(field(
            "document",
            format!("{}, review {review}", doc.label()),
        ));
    }
    fields.push(field("sessions", agent.sessions.to_string()));
    fields.push(field(
        "session",
        session_text(agent.window_id, "main checkout, read-only", app),
    ));
    let role = match agent.role {
        AgentRole::DocReviewer => "doc reviewer",
        _ => "brainstormer",
    };
    let right = match agent.sessions {
        0 | 1 => state.to_owned(),
        n => format!("{state} · {n} sessions"),
    };
    let glyph = kind_glyph(RowKind::DesignAgent { run, agent, window }, app);
    rows(
        glyph,
        format!("{role} {}", clean(&agent.label)),
        right,
        fields,
    )
}
