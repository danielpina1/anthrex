//! Milestone 9.5 tasks 20 and 20b: Enter on a racer or a lane reviewer finds its own
//! round by its lane (ruling T20-1 (c)), and names it with its lane.

use super::*;
use crate::tree::run_fixtures::lane_reviews_fixture;
use proto::{AgentRole, RaceLane};

use super::super::runs::app_with_runs;

fn round_key(role: AgentRole, lane: Option<RaceLane>, session: u32) -> NodeKey {
    NodeKey::AgentRound {
        run: "r1".into(),
        task: "t2".into(),
        role,
        lane,
        session,
        round: 1,
    }
}

fn conversation_of(window_id: u32) -> Vec<Effect> {
    vec![Effect::Send(ClientMsg::SubscribeConversation {
        window_id,
        agent_id: None,
        from_rev: None,
    })]
}

/// Both lanes' first reviewers are `Reviewer` session 1, round 1: Enter on each opens
/// its own, lane a's live window, lane b's ended one.
#[test]
fn enter_on_a_lane_reviewer_opens_its_own_lanes_round() {
    let (snapshot, windows) = lane_reviews_fixture();
    let mut app = app_with_runs(windows, snapshot);
    let a = app.activate_run_node(round_key(AgentRole::Reviewer, Some(RaceLane::A), 1));
    assert_eq!(a, conversation_of(12));

    let (snapshot, windows) = lane_reviews_fixture();
    let mut app = app_with_runs(windows, snapshot);
    let b = app.activate_run_node(round_key(AgentRole::Reviewer, Some(RaceLane::B), 1));
    assert!(b.is_empty(), "{b:?}");
    assert_eq!(
        app.toast_text(),
        Some("review b#1 has finished and its window is gone")
    );
}
