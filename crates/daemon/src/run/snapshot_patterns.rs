//! Milestone 9.5 decision 1 (ruling T20-1, task 20b): what the snapshot shows of a race
//! and a pair. A racing task's lanes, winner and adoption (`TaskInfo.race`), a paired
//! task's test writer and red commit (`TaskInfo.pair`), and the lane a racer's or a
//! lane reviewer's round and review belong to (`AgentRoundInfo.lane`,
//! `ReviewInfo.lane`). `run/snapshot.rs` calls these. Pure (design decision 1): no
//! `std::fs`, `std::process`, `std::thread`, `tokio` or `SystemTime`.

use proto::{AgentRole, LaneInfo, LaneState, PairInfo, RaceInfo, RaceLane};

use super::model::{AgentRound, Lane, ReviewRecord, Task};

/// A racing task's lanes as a client sees them, in the order the task keeps them (lane
/// a, then lane b); `None` for a task that never raced. A race that ended with both
/// lanes out (`Race.ended`) is still shown: its lanes keep their salvage.
pub fn race_info(task: &Task) -> Option<RaceInfo> {
    let race = task.race.as_ref()?;
    Some(RaceInfo {
        lanes: race.lanes.iter().map(lane_info).collect(),
        winner: race.winner,
        adopted: race.adopted,
    })
}

fn lane_info(lane: &Lane) -> LaneInfo {
    LaneInfo {
        lane: lane.lane,
        route: lane.route.clone(),
        state: lane.state,
        checkout: lane.checkout.clone(),
        head: lane.head.clone(),
        reason: lane.reason.clone(),
        salvage_ref: lane.salvage_ref.clone(),
        kept: lane.kept,
    }
}

/// A paired task's test writer and red commit as a client sees them; `None` for a task
/// that is not paired.
pub fn pair_info(task: &Task) -> Option<PairInfo> {
    let pair = task.pair.as_ref()?;
    Some(PairInfo {
        phase: pair.phase,
        writer_route: pair.writer_route.clone(),
        // Whole-branch review D, M-1: before the red check, the test the spec names.
        test: (pair.test.clone()).or_else(|| task.spec.test_to_write.clone()),
        red: pair.red.clone(),
        red_checked: pair.red_checked,
        // The final fix wave (review B's M7): the counter REPORT.md and history read.
        writer_failures: super::history::writer_failures(task),
    })
}

/// The lane a round belongs to: a racer's, or a lane reviewer's (rounds made in a
/// lane's view carry it, `engine/race_view.rs`). A worker, a test writer and every
/// other role belong to no lane.
pub fn round_lane(round: &AgentRound) -> Option<RaceLane> {
    match round.role {
        AgentRole::Racer | AgentRole::Reviewer => round.lane,
        _ => None,
    }
}

/// Task 20b's carry: whether a gate record of task `task` made in `lane` (`None`: the
/// task's own) is one the task shows. While its race has no winner and a lane is still
/// in it, every lane's; once it has a winner, the winner's and the task's own; once
/// the race is over with no winner (both lanes out), the task's own only.
pub fn counts(task: &Task, lane: Option<RaceLane>) -> bool {
    let Some(race) = task.race.as_ref() else {
        return true;
    };
    match (race.winner, lane) {
        (_, None) => true,
        (Some(winner), Some(lane)) => lane == winner,
        (None, Some(_)) => {
            (race.lanes.iter()).any(|l| !matches!(l.state, LaneState::Lost | LaneState::Out))
        }
    }
}

/// The lane a review belongs to: the lane reviewer's that made it.
pub fn review_lane(review: &ReviewRecord) -> Option<RaceLane> {
    review.lane
}

#[cfg(test)]
#[path = "snapshot_patterns_tests.rs"]
mod tests;
