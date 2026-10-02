//! Milestone 9.3 task 4b, fix round 1: a reject or cancel takes the round's path only
//! for an open round (I1); a cancelled round's request and plan state (m1); a widened
//! run's reject (m5); a late summary of the previous round (m2); an orchestrator
//! round's holds under `--yes`; and `max_tasks` counts the run's current round (m3).

use proto::{HoldState, RoundOrigin, RoundOutcome, RunState};
use serde_json::json;

use super::delivery_open::pr_mode;
use super::fixture::*;
use super::gate_holds::{hold_state, in_epic};
use super::goal_rounds_end::{create_stages, submit_round};
use super::goal_rounds_start::{complete, iterate, reply, round_lines, started};
use super::kinds_cancel::{cancel, settle_all};
use super::orch::{answer, edit_plan};
use super::orch_restore::restart;
use super::planners::spawn;
use crate::run::engine::{Effect, EventKind};
use crate::run::model::StageLayout;
use crate::run::orch::PlannerPhase;

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

/// m2: a summary written while round 2 is still open is the previous round's (its
/// completion asked for it); once round 2 has ended, a summary is round 2's.
#[test]
fn a_late_summary_goes_to_the_previous_round() {
    let mut fx = complete();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    let summary = |fx: &mut Fixture, text: &str| {
        let effects = edit_plan(fx, json!({"edits": [], "summary": text}));
        assert!(answer(&effects).0, "{effects:#?}");
    };
    summary(&mut fx, "round one, late");
    let run = fx.run();
    assert_eq!(run.rounds[0].summary.as_deref(), Some("round one, late"));
    assert_eq!(run.rounds[1].summary, None);
    submit_round(&mut fx);
    let effects = reject(&mut fx);
    assert!(reply(&effects).is_ok());
    summary(&mut fx, "round two");
    let run = fx.run();
    assert_eq!(run.rounds[0].summary.as_deref(), Some("round one, late"));
    assert_eq!(run.rounds[1].summary.as_deref(), Some("round two"));
}

/// Decision 12 for holds: an epic the orchestrator adds inside a round it started
/// waits for the user even with `--yes`; inside the user's round `--yes` approves it
/// as before.
#[test]
fn an_orchestrator_rounds_holds_await_the_user_under_yes() {
    for origin in [RoundOrigin::Orchestrator, RoundOrigin::User] {
        let mut fx = complete();
        fx.run_mut().orch.yes = true;
        match origin {
            RoundOrigin::User => assert_eq!(reply(&iterate(&mut fx, "more")), started(2)),
            RoundOrigin::Orchestrator => {
                assert!(answer(&edit_plan(&mut fx, json!({"iterate": "more"}))).0)
            }
        }
        submit_round(&mut fx);
        if fx.run().state == RunState::AwaitingApproval {
            fx.approve();
        }
        assert_eq!(fx.run().state, RunState::Running, "{origin:?}");
        assert_eq!(spawn(&mut fx, "mail")["hold"], "epic:mail", "{origin:?}");
        fx.run_mut().orch.epics[0].phase = PlannerPhase::Finished;
        let mut task = in_epic("t3", "mail", "mail");
        task["task"]["stage"] = json!(2);
        let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [task]})));
        assert!(ok, "{value}");
        let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [], "submit": true})));
        assert!(ok, "{value}");
        let want = match origin {
            RoundOrigin::Orchestrator => HoldState::Awaiting,
            RoundOrigin::User => HoldState::Approved,
        };
        assert_eq!(hold_state(&fx, "epic:mail"), want, "{origin:?}");
    }
}
