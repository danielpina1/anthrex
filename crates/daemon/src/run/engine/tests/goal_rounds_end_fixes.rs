//! Milestone 9.3 task 4b, fix round 1: a reject or cancel takes the round's path only
//! for an open round (I1); a cancelled round's request and plan state (m1); a widened
//! run's reject (m5); a late summary of the previous round (m2); an orchestrator
//! round's holds under `--yes`; and `max_tasks` counts the run's current round (m3).

use proto::{RoundOutcome, RunState};

use super::delivery_open::pr_mode;
use super::fixture::*;
use super::goal_rounds_end::{create_stages, submit_round};
use super::goal_rounds_start::{complete, iterate, reply, round_lines, started};
use super::kinds_cancel::{cancel, settle_all};
use super::orch_restore::restart;
use crate::run::engine::{Effect, EventKind};
use crate::run::model::StageLayout;

fn reject(fx: &mut Fixture) -> Vec<Effect> {
    let reply_id = fx.reply();
    fx.next(EventKind::Reject {
        reply: reply_id,
        run_id: RUN_ID.into(),
    })
}

/// Round 2's `round` lines among the whole log, by outcome.
fn round_two_lines(fx: &Fixture) -> Vec<RoundOutcome> {
    round_lines(&fx.log)
        .into_iter()
        .filter(|l| l.round == 2)
        .map(|l| l.outcome)
        .collect()
}

/// I1: once round 2 ended (rejected, the `pr` run back to delivering), `run cancel`
/// cancels the run, not the ended round.
#[test]
fn a_cancel_after_a_rejected_pr_round_cancels_the_run() {
    let mut fx = complete();
    pr_mode(fx.run_mut());
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    submit_round(&mut fx);
    let effects = reject(&mut fx);
    assert_eq!(
        reply(&effects),
        Ok("run 3f9a round 2 rejected; the earlier rounds are unchanged".into())
    );
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(round_two_lines(&fx), vec![RoundOutcome::Rejected]);
    let effects = cancel(&mut fx);
    assert_eq!(
        reply(&effects),
        Ok(format!(
            "run {RUN_ID} cancelled; it completes once its sessions have ended"
        ))
    );
    let run = fx.run();
    assert!(run.cancelled);
    assert!(!run.finish_edit);
    assert!(!run.delivery.watching);
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Rejected));
    assert_eq!(round_two_lines(&fx), vec![RoundOutcome::Rejected]);
}

/// m1: a round cancelled while it is still being planned (paused there by a restart)
/// drops its request wake and its plan state; the run completes and no round wake is
/// ever pasted into it.
#[test]
fn cancelling_a_round_paused_in_planning_drops_its_request() {
    let mut fx = complete();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    restart(&mut fx);
    assert_eq!(fx.run().state, RunState::Paused);
    assert_eq!(fx.run().paused_from, Some(RunState::Planning));
    assert!(fx.run().orch.request_wake.is_some());
    let logged = fx.log.len();
    let effects = cancel(&mut fx);
    assert_eq!(
        reply(&effects),
        Ok("run 3f9a round 2 cancelled; it ends once its sessions have ended".into())
    );
    let run = fx.run();
    assert_eq!(run.orch.request_wake, None);
    let o = run.orch.orchestrator.as_ref().unwrap();
    assert!(o.plan_submitted, "the earlier rounds' plan stays submitted");
    // The widened run's stage 1 is created from the run head, as for any round.
    create_stages(&mut fx);
    settle_all(&mut fx);
    let run = fx.run();
    assert_eq!(run.state, RunState::Complete);
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Cancelled));
    let round_wakes = fx.log[logged..].iter().any(|e| {
        matches!(e, Effect::WakeOrchestrator { text, request, .. }
            if request.is_some() || text.contains("the user asks for round"))
    });
    assert!(!round_wakes, "{:#?}", &fx.log[logged..]);
}

/// m5: rejecting round 2 of a widened run (`Single` in round 1) leaves no stage record
/// and the run `Multi`, with round 1 as it was.
#[test]
fn rejecting_a_widened_runs_round_keeps_round_one() {
    let mut fx = complete();
    assert_eq!(fx.run().stage_layout, StageLayout::Single);
    let t1 = fx.task("t1").clone();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    submit_round(&mut fx);
    assert!(reply(&reject(&mut fx)).is_ok());
    fx.tick();
    let run = fx.run();
    assert_eq!(run.state, RunState::Complete);
    assert!(run.stages.is_empty(), "{:?}", run.stages);
    assert_eq!(run.stage_layout, StageLayout::Multi);
    assert_eq!(fx.task("t1"), &t1);
    assert_eq!(run.rounds[0].outcome, Some(RoundOutcome::Completed));
    assert_eq!(run.rounds[1].outcome, Some(RoundOutcome::Rejected));
}
