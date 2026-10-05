//! Ruling T13-1: an agent's sessions in a phase are summed; phases and rounds are kept
//! apart.

use proto::{AgentRole, Effort, Route, Runtime, Strength};

use super::*;

fn route(model: &str) -> Route {
    Route {
        runtime: Runtime::Codex,
        model: model.into(),
        strength: Strength::Frontier,
        effort: Effort::High,
    }
}

fn session(label: &str, (calls, tokens, secs): (u32, u64, u64), outcome: &str) -> AgentSpend {
    AgentSpend {
        label: label.into(),
        role: AgentRole::Brainstormer,
        route: route("m"),
        sessions: 0,
        calls,
        tokens,
        secs,
        outcome: outcome.into(),
    }
}

#[test]
fn an_agents_sessions_are_summed_and_its_latest_outcome_kept() {
    let mut design = DesignState::default();
    design.add_session(
        1,
        "brainstorming",
        session("codex", (7, 120, 30), "failed: quiet"),
    );
    design.add_session(1, "brainstorming", session("claude", (5, 50, 20), OK));
    design.add_session(1, "brainstorming", session("codex", (9, 300, 40), OK));
    design.add_session(1, "specifying", session("spec-r1", (3, 10, 5), OK));
    design.add_session(2, "brainstorming", session("codex", (1, 1, 1), OK));
    design.add_phase_secs(1, "brainstorming", 100);
    design.add_phase_secs(1, "brainstorming", 20);
    let phase = design.phase_spend(1, "brainstorming").unwrap();
    assert_eq!(phase.secs, 120);
    let codex = &phase.agents[0];
    assert_eq!(
        (
            codex.label.as_str(),
            codex.sessions,
            codex.calls,
            codex.tokens,
            codex.secs
        ),
        ("codex", 2, 16, 420, 70)
    );
    assert_eq!(codex.outcome, OK);
    assert_eq!(phase.agents[1].label, "claude");
    assert_eq!(
        design.phase_spend(2, "brainstorming").unwrap().agents[0].calls,
        1
    );
    assert_eq!(design.phase_spend(1, "planning"), None);
    assert_eq!(
        design.spend_totals(),
        Totals {
            calls: 7 + 5 + 9 + 3 + 1,
            tokens: 120 + 50 + 300 + 10 + 1,
            secs: 120,
        }
    );
    let agent = codex.phase_agent();
    assert_eq!((agent.calls, agent.tokens, agent.secs), (16, 420, 70));
}
