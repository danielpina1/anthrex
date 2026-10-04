//! A task's agent rounds as the run view draws them (milestone 8c decisions 13 and 14):
//! a worker session split at its bounces, in start order. Split out of `run_rows.rs`.
//! Milestone 9.5 decision 29: a racer and a test writer split as a worker does.

use super::DisplayRound;
use proto::{AgentRole, AgentRoundInfo, RaceLane, TaskInfo, WindowInfo};
use std::collections::HashSet;

/// A task's agent rounds as the run view draws them, in start order, ties broken as
/// [`rank`] says (decisions 13 and 14; milestone 9.5 decision 29). A worker, racer or
/// test-writer session becomes one display round per piece between its `sent_back_at`
/// bounces; every other round is one. Bounces are
/// taken in time order and never before the session's start, and a repeated
/// `(role, lane, session, number)` keeps its earliest copy, so no two rounds share a
/// key (both lanes number their reviewers from 1; ruling T20-1).
pub fn display_rounds<'a>(task: &'a TaskInfo, windows: &'a [WindowInfo]) -> Vec<DisplayRound<'a>> {
    rounds_with(task, |id| windows.iter().find(|window| window.id == id))
}

/// [`display_rounds`] with the window lookup given, so `run_rows` looks each id up once
/// in a map rather than scanning the listed windows per round.
pub(super) fn rounds_with<'a>(
    task: &'a TaskInfo,
    lookup: impl Fn(u32) -> Option<&'a WindowInfo>,
) -> Vec<DisplayRound<'a>> {
    let mut rounds = Vec::new();
    for info in &task.rounds {
        let window = info.window_id.and_then(&lookup);
        let single = |number| DisplayRound {
            info,
            number,
            started_at: info.started_at,
            ended_at: info.ended_at,
            last: true,
            window,
        };
        match info.role {
            AgentRole::Worker | AgentRole::Racer | AgentRole::TestWriter => {
                let mut bounces = info.sent_back_at.clone();
                bounces.sort_unstable();
                let mut starts = vec![info.started_at];
                for at in bounces {
                    let floor = starts.last().copied().unwrap_or(info.started_at);
                    starts.push(at.max(floor));
                }
                for (index, started_at) in starts.iter().copied().enumerate() {
                    let next = starts.get(index + 1).copied();
                    rounds.push(DisplayRound {
                        number: u32::try_from(index + 1).unwrap_or(u32::MAX),
                        started_at,
                        ended_at: next.or(info.ended_at),
                        last: next.is_none(),
                        ..single(1)
                    });
                }
            }
            AgentRole::Reviewer
            | AgentRole::Orchestrator
            | AgentRole::Scout
            | AgentRole::Planner
            | AgentRole::Decider => {
                rounds.push(single(info.round));
            }
        }
    }
    rounds.sort_by_key(|round| (round.started_at, rank(round.info)));
    let mut seen = HashSet::new();
    rounds.retain(|round| {
        let info = round.info;
        seen.insert((info.role, info.lane, info.session, round.number))
    });
    rounds
}

/// The order of rounds that start together: test writer, worker, racer a, racer b,
/// then reviewers, lane a's before lane b's (milestone 9.5 decision 29); then the
/// roles a task rarely carries.
fn rank(info: &AgentRoundInfo) -> u8 {
    let lane_b = u8::from(info.lane == Some(RaceLane::B));
    match info.role {
        AgentRole::TestWriter => 0,
        AgentRole::Worker => 1,
        AgentRole::Racer => 2 + lane_b,
        AgentRole::Reviewer => 4 + lane_b,
        AgentRole::Orchestrator => 6,
        AgentRole::Scout => 7,
        AgentRole::Planner => 8,
        AgentRole::Decider => 9,
    }
}
