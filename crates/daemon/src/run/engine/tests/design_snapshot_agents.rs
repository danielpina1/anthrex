//! Task M9.6.18, ruling T18-1: the snapshot lists the current round's brainstormers and
//! document reviewer (`RunInfo.design_agents`), one entry an agent with its session
//! count, so a relaunched agent (ruling T8-7) is one entry of two sessions; a reviewer
//! names its document and review number. A run without the flow lists none.

use proto::{AgentRole, DesignAgentInfo, DesignAgentStatus, DocKind, Runtime};

use super::design_fixture::{at_spec_gate, brainstormer};
use super::fixture::*;
use crate::run::design::state::DesignAgentState;
use crate::run::snapshot::snapshot;

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
