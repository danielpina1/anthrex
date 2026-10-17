//! Ruling T13-1: an agent's sessions in a phase are summed; phases and rounds are kept
//! apart.

use proto::{AgentRole, Effort, Route, Runtime, TokenUsage};

use super::*;

fn route(model: &str) -> Route {
    Route {
        runtime: Runtime::Codex,
        model: model.into(),
        effort: Effort::HIGH,
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
        usage: TokenUsage {
            input: tokens,
            cache_read: 1_000,
            ..TokenUsage::default()
        },
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

/// Milestone 9.19 decision 2: an agent's usage is summed raw and its tokens are that
/// usage's billed tokens; an agent saved without `usage` (format 6, its tokens counting
/// cache reads) carries no tokens to the history line or the totals (ruling R10).
#[test]
fn tokens_are_the_summed_usages_billed_tokens() {
    let mut design = DesignState::default();
    design.add_session(1, "brainstorming", session("codex", (1, 100, 1), OK));
    design.add_session(1, "brainstorming", session("codex", (1, 200, 1), OK));
    let codex = &design.phase_spend(1, "brainstorming").unwrap().agents[0];
    assert_eq!(codex.usage.cache_read, 2_000);
    assert_eq!((codex.tokens, codex.phase_agent().tokens), (300, 300));

    let legacy: AgentSpend = serde_json::from_value(serde_json::json!({
        "label": "claude", "role": "brainstormer",
        "route": {"runtime": "claude", "model": "m", "effort": "high"},
        "sessions": 1, "calls": 12, "tokens": 209_000, "secs": 300, "outcome": "ok"
    }))
    .unwrap();
    assert_eq!(
        (legacy.tokens, legacy.usage),
        (209_000, TokenUsage::default())
    );
    assert_eq!(legacy.phase_agent().tokens, 0);
    let mut old = DesignState::default();
    old.spend.push(PhaseSpend {
        round: 1,
        phase: "brainstorming".into(),
        secs: 0,
        agents: vec![legacy],
    });
    assert_eq!(old.spend_totals().tokens, 0);
}
