//! A task's agent rounds as the run view draws them (milestone 8c decisions 13 and 14):
//! a worker session split at its bounces, in start order. Split out of `run_rows.rs`.

use super::DisplayRound;
use proto::{AgentRole, TaskInfo, WindowInfo};
use std::collections::HashSet;

/// A task's agent rounds as the run view draws them, in start order, a worker before a
/// reviewer on a tie (decisions 13 and 14). A worker session becomes one display round
/// per piece between its `sent_back_at` bounces; every other round is one. Bounces are
/// taken in time order and never before the session's start, and a repeated
/// `(role, session, number)` keeps its earliest copy, so no two rounds share a key.
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
            AgentRole::Worker => {
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
            | AgentRole::Decider
            // Milestone 9.5: split at `sent_back_at` by task M9.5.20.
            | AgentRole::Racer
            | AgentRole::TestWriter => {
                rounds.push(single(info.round));
            }
        }
    }
    rounds.sort_by_key(|round| (round.started_at, role_rank(round.info.role)));
    let mut seen = HashSet::new();
    rounds.retain(|round| seen.insert((round.info.role, round.info.session, round.number)));
    rounds
}

fn role_rank(role: AgentRole) -> u8 {
    match role {
        AgentRole::Worker => 0,
        AgentRole::Reviewer => 1,
        AgentRole::Orchestrator => 2,
        AgentRole::Scout => 3,
        AgentRole::Planner => 4,
        AgentRole::Decider => 5,
        // Milestone 9.5: ordered by task M9.5.20.
        AgentRole::Racer => 6,
        AgentRole::TestWriter => 7,
    }
}
