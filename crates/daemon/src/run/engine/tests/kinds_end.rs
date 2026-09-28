//! M9.9 review fixes, C1 and I4: the user can always end a run. After `run cancel` or
//! the `finish` edit, a run with an orchestrator completes without its plan's submit,
//! its holds' verdicts or a `changes` verdict's fix; and the user's submit (decision
//! 13) is accepted on a promoted running run whose orchestrator never submitted. Each
//! run then completes and takes `run accept` or `run discard`.

use proto::{FinishAction, HoldState, PlanEdit, RunState, TaskState};

use super::dispatch::{edit, replies};
use super::fixture::*;
use super::kinds::changes;
use super::kinds_integration::{C1, merge_real, reviewed};
use super::merge::{pending, pending_one};
use super::promote::{hold_verdict, promoted};
use crate::run::engine::{Effect, EventKind, OpResult};
use crate::run::validate::EditScope;

fn cancel(fx: &mut Fixture) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Cancel {
        reply,
        run_id: RUN_ID.into(),
    })
}

fn finish(fx: &mut Fixture, action: FinishAction) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Finish {
        reply,
        run_id: RUN_ID.into(),
        action,
    })
}

fn user_submit(fx: &mut Fixture) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Edit {
        reply,
        run_id: RUN_ID.into(),
        edits: Vec::new(),
        scope: EditScope::Run,
        refusals: Vec::new(),
        submit: true,
    })
}

/// Answers the ref guard and a final check, if one runs, until the run completes.
fn to_complete(fx: &mut Fixture) {
    for _ in 0..4 {
        if fx.run().state == RunState::Complete {
            return;
        }
        fx.tick();
        if let Some((op, _)) = pending(fx, "VerifyRefs", None).first().cloned() {
            fx.done(op, OpResult::RefsOk);
        } else if let Some((op, _)) = pending(fx, "Check", None).first().cloned() {
            fx.done(
                op,
                OpResult::Check {
                    ok: true,
                    code: Some(0),
                    timed_out: false,
                    tail: String::new(),
                    secs: 1,
                },
            );
        }
    }
    assert_eq!(fx.run().state, RunState::Complete, "{:#?}", fx.run().log);
}

/// The promoted run whose `t1` merged, stuck before the fix: its orchestrator never
/// submitted, so decision 38 held it running.
fn promoted_and_merged() -> Fixture {
    let mut fx = promoted();
    merge_real(&mut fx, "t1", C1);
    fx.tick();
    assert_eq!(fx.task("t1").state, TaskState::Merged);
    assert_eq!(fx.run().state, RunState::Running);
    assert!(pending(&fx, "VerifyRefs", None).is_empty());
    fx
}

fn accepted(fx: &mut Fixture) {
    finish(fx, FinishAction::Accept);
    pending_one(fx, "Accept", None);
}

fn discarded(fx: &mut Fixture) {
    finish(fx, FinishAction::Discard);
    pending_one(fx, "Discard", None);
}

#[test]
fn a_promoted_run_the_user_cancels_completes() {
    let mut fx = promoted_and_merged();
    let effects = cancel(&mut fx);
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    to_complete(&mut fx);
    accepted(&mut fx);
}

#[test]
fn a_promoted_run_the_user_finishes_completes() {
    let mut fx = promoted_and_merged();
    let effects = edit(&mut fx, vec![PlanEdit::Finish]);
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    to_complete(&mut fx);
    discarded(&mut fx);
}

#[test]
fn the_users_submit_ends_a_promoted_runs_promotion() {
    let mut fx = promoted_and_merged();
    let effects = user_submit(&mut fx);
    assert_eq!(
        replies(&effects),
        vec![Ok(format!(
            "the plan of run {RUN_ID} was submitted: hold promotion awaits approval"
        ))]
    );
    let o = fx.run().orch.orchestrator.clone().unwrap();
    assert!(o.plan_submitted);
    let hold = fx
        .run()
        .orch
        .gate_holds
        .iter()
        .find(|h| h.id == "promotion");
    assert_eq!(hold.map(|h| h.state), Some(HoldState::Awaiting));
    // The user's verdict on the promotion, then the run completes as any does.
    assert_eq!(fx.run().state, RunState::Running);
    hold_verdict(&mut fx, "promotion", true);
    to_complete(&mut fx);
    accepted(&mut fx);
}

#[test]
fn a_planned_run_whose_epic_asked_for_changes_completes_on_cancel() {
    let mut fx = reviewed(changes());
    fx.tick();
    assert_eq!(fx.run().state, RunState::Running);
    assert!(pending(&fx, "VerifyRefs", None).is_empty());
    let effects = cancel(&mut fx);
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    to_complete(&mut fx);
    // No integration review is made for the cancelled run.
    assert!(fx.run().task("mail-int2").is_none());
    discarded(&mut fx);
}
