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

    /// Stage `n`'s round: its record's, or for a stage not yet created its tasks'
    /// lowest, 1 with neither.
    pub fn stage_round(&self, n: u16) -> u32 {
        match self.stage(n) {
            Some(record) => record.round,
            None => self
                .tasks
                .iter()
                .filter(|t| t.stage() == n)
                .map(|t| t.round)
                .min()
                .unwrap_or(1),
        }
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
