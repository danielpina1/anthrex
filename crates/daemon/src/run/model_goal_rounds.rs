//! Milestone 9.3's rounds (KG §2.1, decision 18): one request a run works on. Round 1
//! is the goal the run started with; each iterate adds one. Pure (design decision 2).
//!
//! The name `rounds` is taken by `model_rounds.rs` (agent session rounds), so this file
//! is mounted from `model.rs` as `goal_rounds`.

use proto::{ROUND_HEAD_CHARS, RoundInfo, RoundOrigin, RoundOutcome, safe_text};
use serde::{Deserialize, Serialize};

use super::Run;

/// One round of a run (KG §2.1's struct, plus D5's two counters).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Round {
    pub n: u32,
    /// The request, `safe_text::multi_line`, at most `proto::GOAL_MAX_CHARS`; round 1's
    /// is the run's goal.
    pub goal: String,
    pub origin: RoundOrigin,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    pub outcome: Option<RoundOutcome>,
    pub summary: Option<String>,
    /// The first stage the round's tasks may go in.
    pub first_stage: u16,
    /// D5: `Run.windows_created` when the round started; `max_windows` counts from it.
    #[serde(default)]
    pub windows_before: u32,
    /// D5: the run scouts started before the round; `max_scouts` counts from it.
    #[serde(default)]
    pub scouts_before: u32,
    /// Milestone 9.5 decision 14 (ruling RE-1): when the round's plan was approved;
    /// round 1's is `Run.approved_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approved_at: Option<u64>,
    /// Decision 15: the run's paused seconds at the round's approval.
    #[serde(default, skip_serializing_if = "super::is_zero")]
    pub paused_before: u64,
    /// Milestone 9.6 ruling T15-15: the stage this round's documents commit created, set
    /// when the commit's reply lands (a design run's round 2 on).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub committed_stage: Option<u16>,
    /// Ruling T15-17: the round was dropped (rejected, or cancelled before anything of
    /// its documents landed); its approval does not stand (`approved_at` is cleared).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub dropped: bool,
}

impl Round {
    /// Decision 18's round 1 of `run`: its goal, `user`, from its creation, in stage 1,
    /// not ended.
    pub fn first(run: &Run) -> Round {
        Round {
            n: 1,
            goal: run.goal.clone(),
            origin: RoundOrigin::User,
            started_at: run.created_at,
            ended_at: None,
            outcome: None,
            summary: None,
            first_stage: 1,
            windows_before: 0,
            scouts_before: 0,
            approved_at: None,
            paused_before: 0,
            committed_stage: None,
            dropped: false,
        }
    }

    /// The round as a client sees it (KG §5): its goal and summary on one line, cut to
    /// [`ROUND_HEAD_CHARS`].
    pub fn info(&self) -> RoundInfo {
        RoundInfo {
            n: self.n,
            goal_head: head(&self.goal),
            origin: self.origin,
            outcome: self.outcome,
            summary_head: self.summary.as_deref().map(head),
            ended: self.ended_at.is_some(),
        }
    }
}

/// `text` cleaned to one line and cut to [`ROUND_HEAD_CHARS`] characters.
fn head(text: &str) -> String {
    safe_text::one_line(text)
        .chars()
        .take(ROUND_HEAD_CHARS)
        .collect()
}

impl Run {
    /// The current round's number: the last round's, 1 with none recorded.
    pub fn round(&self) -> u32 {
        self.rounds.last().map_or(1, |r| r.n)
    }

    pub fn current_round(&self) -> Option<&Round> {
        self.rounds.last()
    }

    /// The round whose stages hold stage `n` (decision 13): the last round starting at
    /// or below it, 1 with none. A stage a round creates carries it.
    pub fn round_of_stage(&self, n: u16) -> u32 {
        self.rounds
            .iter()
            .rev()
            .find(|r| r.first_stage <= n)
            .map_or(1, |r| r.n)
    }

    /// Stage `n`'s round while it has no record: its tasks' lowest, else
    /// [`Run::round_of_stage`]. A created stage's round is its record's, which the
    /// snapshot reads from the record `created` filtered, never from the placeholder
    /// stage 1 of a plan not yet approved (task 3 review m1).
    pub fn planned_stage_round(&self, n: u16) -> u32 {
        self.tasks
            .iter()
            .filter(|t| t.stage() == n)
            .map(|t| t.round)
            .min()
            .unwrap_or_else(|| self.round_of_stage(n))
    }

    /// `RunInfo.rounds`: one entry a round once the run has more than one, else none,
    /// so a one-round run's snapshot is as before rounds.
    pub fn round_infos(&self) -> Vec<RoundInfo> {
        if self.rounds.len() < 2 {
            return Vec::new();
        }
        self.rounds.iter().map(Round::info).collect()
    }
}
