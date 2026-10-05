//! Task M9.6.18, ruling T18-1: the snapshot lists the current round's brainstormers and
//! document reviewer (`RunInfo.design_agents`), one entry an agent with its session
//! count, so a relaunched agent (ruling T8-7) is one entry of two sessions; a reviewer
//! names its document and review number. A run without the flow lists none. Ruling
//! T18-6: only the current round's agents, so a later round lists none of round 1's.

use proto::{AgentRole, DesignAgentInfo, DesignAgentStatus, DocKind, RoundDesign, Runtime};

use super::design_fixture::{at_spec_gate, brainstormer, start_brainstorm};
use super::design_rounds::round_task;
use super::design_rounds_fixture::{design_complete, iterate_with, round_planning, round_submit};
use super::fixture::*;
use crate::run::design::state::DesignAgentState;
use crate::run::snapshot::snapshot;
use serde_json::json;

fn agents(fx: &Fixture) -> Vec<DesignAgentInfo> {
    snapshot(&fx.state, fx.now).runs.remove(0).design_agents
}

#[test]
fn the_snapshot_lists_the_design_agents() {
    let mut fx = at_spec_gate(false);
    let design = fx.run_mut().orch.design.as_mut().unwrap();
    let mut codex = brainstormer("codex", Runtime::Codex);
    codex.session = 2;
    codex.window_id = Some(41);
    codex.state = DesignAgentState::Running;
    let mut claude = brainstormer("claude", Runtime::Claude);
    claude.state = DesignAgentState::Failed("ended twice without submitting".into());
    design.brainstormers = vec![claude, codex];
    let mut reviewer = brainstormer("spec-r2", Runtime::Codex);
    reviewer.role = AgentRole::DocReviewer;
    reviewer.state = DesignAgentState::Queued;
    reviewer.session = 0;
    design.reviewer = Some(reviewer);
    let want = |role, label: &str, runtime, state, sessions, window_id| DesignAgentInfo {
        role,
        label: label.into(),
        runtime,
        state,
        sessions,
        window_id,
        doc: None,
        review: None,
    };
    assert_eq!(
        agents(&fx),
        [
            want(
                AgentRole::Brainstormer,
                "claude",
                Runtime::Claude,
                DesignAgentStatus::Failed,
                1,
                None
            ),
            want(
                AgentRole::Brainstormer,
                "codex",
                Runtime::Codex,
                DesignAgentStatus::Running,
                2,
                Some(41)
            ),
            DesignAgentInfo {
                doc: Some(DocKind::Spec),
                review: Some(2),
                ..want(
                    AgentRole::DocReviewer,
                    "spec-r2",
                    Runtime::Codex,
                    DesignAgentStatus::Queued,
                    0,
                    None
                )
            },
        ]
    );
    // Submitted and done are told apart.
    let design = fx.run_mut().orch.design.as_mut().unwrap();
    design.brainstormers[0].state = DesignAgentState::Submitted;
    design.brainstormers[1].state = DesignAgentState::Done;
    design.reviewer = None;
    let states: Vec<_> = agents(&fx).into_iter().map(|a| a.state).collect();
    assert_eq!(
        states,
        [DesignAgentStatus::Submitted, DesignAgentStatus::Done]
    );
}

#[test]
fn a_run_without_the_flow_lists_no_design_agents() {
    let (fx, _, _) = super::race::racing();
    assert!(agents(&fx).is_empty());
}

fn listed(fx: &Fixture) -> Vec<(AgentRole, String)> {
    agents(fx).into_iter().map(|a| (a.role, a.label)).collect()
}

/// Ruling T18-6: an `amend` round 2 runs no brainstorm, so round 1's brainstormers
/// and its last reviewer (`plan-r1`) are not this round's; its own plan reviewer is.
#[test]
fn an_amend_round_lists_none_of_round_ones_agents() {
    let mut fx = design_complete();
    let before = listed(&fx);
    assert!(
        before.iter().any(|(r, _)| *r == AgentRole::Brainstormer)
            && before.iter().any(|(r, _)| *r == AgentRole::DocReviewer),
        "round 1 lists its agents: {before:?}"
    );
    iterate_with(&mut fx, None).unwrap();
    assert_eq!(listed(&fx), [], "round 2 has started no agent");
    let mut fx = round_planning();
    assert_eq!(
        listed(&fx),
        [],
        "the amendment approved, nothing queued yet"
    );
    round_submit(&mut fx, json!([round_task("t2", &["R2", "R3"])])).unwrap();
    assert_eq!(
        listed(&fx),
        [(AgentRole::DocReviewer, "plan-r2".to_string())]
    );
}

/// Ruling T18-6: a `full` round 2 lists its own brainstormers, and round 1's reviewer
/// not at all before round 2's first review.
#[test]
fn a_full_round_lists_its_brainstormers_and_no_earlier_reviewer() {
    let mut fx = design_complete();
    iterate_with(&mut fx, Some(RoundDesign::Full)).unwrap();
    let held = |fx: &Fixture| {
        let design = fx.run().orch.design.as_ref().unwrap();
        design.reviewer.as_ref().is_some_and(|r| r.round == 1)
    };
    assert!(held(&fx), "round 1's reviewer is still held");
    assert_eq!(
        listed(&fx),
        [],
        "no brainstormer yet, and not round 1's reviewer"
    );
    start_brainstorm(&mut fx);
    assert!(held(&fx));
    let design = fx.run().orch.design.as_ref().unwrap();
    let want: Vec<_> = (design.brainstormers.iter())
        .map(|b| (AgentRole::Brainstormer, b.label.clone()))
        .collect();
    assert!(!want.is_empty() && design.brainstormers.iter().all(|b| b.round == 2));
    assert_eq!(listed(&fx), want);
}
