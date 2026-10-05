//! Milestone 9.6 decision 32 and ruling T13-1: what each design phase spent, kept on
//! `DesignState.spend` per round and phase. The orchestrator's time is the phase clock's
//! (decision 8: gate waits, pauses and downtime left out), added each time the clock
//! stops; each design agent's tokens, tool calls and active time are summed over every
//! session it ran in the phase, its relaunches and rethink rounds included, and its
//! outcome is its latest session's. A phase's history record (`run/history.rs`) and
//! REPORT.md's spend line (`report_design.rs`) read it. Pure.

use proto::{AgentRole, DocGateKind, PhaseAgent, Route};
use serde::{Deserialize, Serialize};

use super::state::DesignState;

/// A session that delivered its draft or findings.
pub const OK: &str = "ok";
/// A session its budget stopped.
pub const OVER_BUDGET: &str = "over budget";

/// The phase whose document the `kind` gate shows: `brainstorming`, `specifying` or
/// `planning`.
pub fn phase_name(kind: DocGateKind) -> &'static str {
    match kind {
        DocGateKind::Brainstorm => "brainstorming",
        DocGateKind::Spec => "specifying",
        DocGateKind::Plan => "planning",
    }
}

/// One design phase of one round.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhaseSpend {
    pub round: u32,
    /// `brainstorming`, `specifying` or `planning`.
    pub phase: String,
    /// The phase clock's seconds, summed over its runs.
    #[serde(default)]
    pub secs: u64,
    #[serde(default)]
    pub agents: Vec<AgentSpend>,
}

/// One design agent's sessions in a phase, summed: a brainstormer by its label, a
/// document reviewer by its review's (`spec-r1`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSpend {
    pub label: String,
    pub role: AgentRole,
    pub route: Route,
    #[serde(default)]
    pub sessions: u32,
    #[serde(default)]
    pub calls: u32,
    #[serde(default)]
    pub tokens: u64,
    #[serde(default)]
    pub secs: u64,
    /// [`OK`], [`OVER_BUDGET`] or `failed: <reason>`: its latest session's.
    pub outcome: String,
}

impl AgentSpend {
    /// As the history line carries it.
    pub fn phase_agent(&self) -> PhaseAgent {
        PhaseAgent {
            role: self.role,
            route: self.route.clone(),
            calls: self.calls,
            tokens: self.tokens,
            outcome: self.outcome.clone(),
            secs: self.secs,
            sessions: self.sessions,
        }
    }
}

/// What every design phase spent, for REPORT.md's spend line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub calls: u64,
    pub tokens: u64,
    pub secs: u64,
}

impl DesignState {
    /// Round `round`'s `phase`, recorded or not.
    pub fn phase_spend(&self, round: u32, phase: &str) -> Option<&PhaseSpend> {
        (self.spend.iter()).find(|p| p.round == round && p.phase == phase)
    }

    fn phase_spend_mut(&mut self, round: u32, phase: &str) -> &mut PhaseSpend {
        let at = (self.spend.iter()).position(|p| p.round == round && p.phase == phase);
        let at = at.unwrap_or_else(|| {
            self.spend.push(PhaseSpend {
                round,
                phase: phase.to_string(),
                secs: 0,
                agents: Vec::new(),
            });
            self.spend.len() - 1
        });
        &mut self.spend[at]
    }

    /// The phase clock ran `secs` in round `round`'s `phase`.
    pub fn add_phase_secs(&mut self, round: u32, phase: &str, secs: u64) {
        let spend = self.phase_spend_mut(round, phase);
        spend.secs = spend.secs.saturating_add(secs);
    }

    /// One session of `session.label` ended in round `round`'s `phase`: added to that
    /// agent's sums (`sessions` counts it), its route and outcome replacing the earlier.
    pub fn add_session(&mut self, round: u32, phase: &str, session: AgentSpend) {
        let spend = self.phase_spend_mut(round, phase);
        let found = (spend.agents.iter_mut()).find(|a| a.label == session.label);
        let Some(agent) = found else {
            spend.agents.push(AgentSpend {
                sessions: 1,
                ..session
            });
            return;
        };
        agent.sessions += 1;
        agent.calls = agent.calls.saturating_add(session.calls);
        agent.tokens = agent.tokens.saturating_add(session.tokens);
        agent.secs = agent.secs.saturating_add(session.secs);
        agent.route = session.route;
        agent.outcome = session.outcome;
    }

    /// Every design phase's spend, summed.
    pub fn spend_totals(&self) -> Totals {
        let mut totals = Totals::default();
        for phase in &self.spend {
            totals.secs = totals.secs.saturating_add(phase.secs);
            for agent in &phase.agents {
                totals.calls = totals.calls.saturating_add(u64::from(agent.calls));
                totals.tokens = totals.tokens.saturating_add(agent.tokens);
            }
        }
        totals
    }
}

#[cfg(test)]
#[path = "spend_tests.rs"]
mod tests;
