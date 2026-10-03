//! Milestone 9.5 decision 34 (FU-F37): a round after the first that never completes
//! ends `cancelled` when its run becomes terminal (discarded or failed), its `round`
//! line written before the run's own line, so `run stats` counts every round.

use proto::{FinishAction, HistoryLine, RoundOutcome, RunState};

use super::chains::finished;
use super::fixture::*;
use super::goal_rounds_end::{halted_in_round_two, settle_ops};
use super::goal_rounds_start::complete;
use super::kinds_cancel::cancel;
use crate::run::engine::OpKind;

/// Every history line the run appended, in order: `round <n> <outcome>` or
/// `run <outcome>`; other lines are left out.
fn history(fx: &Fixture) -> Vec<String> {
    ops_in(&fx.log, "AppendHistory")
        .into_iter()
        .filter_map(|(_, kind)| match kind {
            OpKind::AppendHistory { line, .. } => match *line {
                HistoryLine::Round(r) => Some(format!("round {} {:?}", r.round, r.outcome)),
                HistoryLine::Run(r) => Some(format!("run {}", r.outcome)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The run's log entries that start with `start`.
fn logged(fx: &Fixture, start: &str) -> Vec<String> {
    let entries = fx.run().log.iter().map(|e| e.text.clone());
    entries.filter(|t| t.starts_with(start)).collect()
}

/// Round 2 of a halted run, then `run discard`: round 2's line, `cancelled`, comes
/// before the run's line.
#[test]
fn a_discarded_later_round_writes_its_round_line() {
    let mut fx = halted_in_round_two();
    assert_eq!(fx.run().state, RunState::Halted);
    // The round's cancel ends its tasks and leaves the run halted and the round open.
    // No request makes that run discardable today (`rules::finish` refuses a halted run
    // that is not `cancelled`; Implementation notes, Task M9.5.4a): set in place.
    cancel(&mut fx);
    settle_ops(&mut fx);
    assert_eq!(fx.run().state, RunState::Halted);
    assert_eq!(fx.run().rounds[1].ended_at, None);
    fx.run_mut().cancelled = true;
    finished(&mut fx, FinishAction::Discard);
    settle_ops(&mut fx);
    fx.tick();
    settle_ops(&mut fx);
    let run = fx.run();
    assert_eq!(run.state, RunState::Discarded);
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Cancelled));
    assert!(run.rounds[1].ended_at.is_some());
    assert_eq!(
        history(&fx),
        ["round 1 Completed", "round 2 Cancelled", "run discarded"]
    );
    let ended = "round 2 ended cancelled: the run is discarded";
    assert_eq!(logged(&fx, "round 2 ended"), [ended]);

    // A run that fails in round 2 likewise (only a run's start fails it today: set in
    // place), its open round's outcome unset until then.
    let mut fx = halted_in_round_two();
    assert_eq!(fx.run().rounds[1].outcome, None);
    fx.run_mut().state = RunState::Failed;
    fx.tick();
    settle_ops(&mut fx);
    fx.tick();
    settle_ops(&mut fx);
    assert_eq!(fx.run().rounds[1].outcome, Some(RoundOutcome::Cancelled));
    assert_eq!(
        history(&fx),
        ["round 1 Completed", "round 2 Cancelled", "run failed"]
    );
    let ended = "round 2 ended cancelled: the run is failed";
    assert_eq!(logged(&fx, "round 2 ended"), [ended]);

    // Round 1 alone writes no round line (unchanged).
    let mut fx = complete();
    finished(&mut fx, FinishAction::Discard);
    settle_ops(&mut fx);
    fx.tick();
    settle_ops(&mut fx);
    assert_eq!(fx.run().state, RunState::Discarded);
    assert_eq!(history(&fx), ["run discarded"]);
    assert!(logged(&fx, "round").is_empty());
}
