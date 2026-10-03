//! Milestone 9.1 task M9.1.15: a probe the executor could not run is never red (ruling
//! C-18): it is issued again after a backoff, and the bisect ends without a culprit
//! after the third failure or a setup failure; the `finish` edit ends a bisect.

use proto::{PlanEdit, RunState};

use super::*;
use crate::run::engine::EventKind;

const LOCK: &str = "git: could not lock the index";

fn failed() -> OpResult {
    OpResult::Failed {
        message: format!("{LOCK}\nsecond line"),
    }
}

#[test]
fn a_failed_probe_is_retried_after_a_backoff_and_is_never_red() {
    let mut fx = merged(&["t1", "t2"], "");
    with_orchestrator(&mut fx);
    red_full(&mut fx);
    let (op, spec) = probe(&fx);
    fx.done(op, failed());
    let bisect = fx
        .run()
        .stage(1)
        .unwrap()
        .bisect
        .clone()
        .expect("still bisecting");
    assert_eq!((bisect.probes, bisect.infra, bisect.lo), (0, 1, 0));
    assert_eq!(fx.run().full_op, None);
    assert!(notes(&fx).is_empty(), "{:?}", notes(&fx));
    let effects = fx.send(fx.now + 29, EventKind::Tick);
    assert!(ops_in(&effects, "TestAt").is_empty(), "{effects:#?}");
    assert!(ops_in(&effects, "VerifyRefs").is_empty(), "{effects:#?}");
    fx.send(fx.now + 1, EventKind::Tick);
    let (op, again) = probe(&fx);
    assert_eq!(again.commit, spec.commit, "the same probe");
    fx.done(op, failed());
    assert!(pending(&fx, "TestAt", None).is_empty());
    fx.send(fx.now + 120, EventKind::Tick);
    let (op, _) = probe(&fx);
    fx.done(op, failed());
    // The third failure ends the bisect without a culprit; the stage is red from its
    // tier 3, never from a probe.
    assert!(pending(&fx, "TestAt", None).is_empty());
    let stage = fx.run().stage(1).unwrap();
    assert_eq!(stage.bisect, None);
    assert_eq!(stage.full.red_at, Some(commit(2)));
    let line = format!(
        "tier 3 red, no single culprit: a::works (stage 1: could not probe {}: {LOCK})",
        sha7(BASE)
    );
    assert!(attention(&fx).contains(&line), "{:?}", attention(&fx));
    assert_eq!(
        notes(&fx),
        ["stage 1 tier 3 red, no single culprit: a::works; plan a fix"]
    );
    assert!(fx.run().task("fix1").is_none());

    // A probe that cannot set up ends at once.
    let mut fx = merged(&["t1", "t2"], "");
    red_full(&mut fx);
    let (op, _) = probe(&fx);
    fx.done(
        op,
        OpResult::SetupFailed {
            output: "\n  npm ERR! missing script\nmore".into(),
        },
    );
    let line = format!(
        "tier 3 red, no single culprit: a::works (stage 1: could not probe {}: setup failed: npm ERR! missing script)",
        sha7(BASE)
    );
    assert!(attention(&fx).contains(&line), "{:?}", attention(&fx));
    assert_eq!(fx.run().stage(1).unwrap().bisect, None);
}

#[test]
fn a_finish_edit_ends_the_bisect_and_the_run_completes() {
    let mut fx = merged(&["t1", "t2"], "");
    red_full(&mut fx);
    let (op, spec) = probe(&fx);
    super::super::dispatch::edit(&mut fx, vec![PlanEdit::Finish]);
    fx.done(op, probe_result(false, &spec.commit));
    assert_eq!(fx.run().stage(1).unwrap().bisect, None);
    assert!(pending(&fx, "TestAt", None).is_empty());
    let effects = verify_ok(&mut fx);
    assert!(ops_in(&effects, "Tier").is_empty(), "{effects:#?}");
    assert_eq!(fx.run().state, RunState::Complete);
    assert!(fx.run().final_check_failed);
    assert!(
        attention(&fx).contains(&"tier 3 red on stage 1: a::works".to_string()),
        "{:?}",
        attention(&fx)
    );

    // With no probe in flight (one waits out a backoff), the next pass ends it.
    let mut fx = merged(&["t1", "t2"], "");
    red_full(&mut fx);
    let (op, _) = probe(&fx);
    fx.done(op, failed());
    super::super::dispatch::edit(&mut fx, vec![PlanEdit::Finish]);
    assert_eq!(fx.run().stage(1).unwrap().bisect, None);
    verify_ok(&mut fx);
    assert_eq!(fx.run().state, RunState::Complete);
}

/// W1 fix round 2 (item 4): a round's cancel is not the run's finish. With round 2
/// being cancelled (its stages from 2 on), stage 1's bisect, round 1's, keeps going:
/// the probe's answer moves it on rather than ending it.
#[test]
fn a_round_cancel_leaves_an_earlier_stages_bisect_going() {
    let mut fx = merged(&["t1", "t2"], "");
    red_full(&mut fx);
    let (op, spec) = probe(&fx);
    // Round 2 open and being cancelled (set in place: the cancel itself is
    // `goal_rounds_cancel`'s; only the scope of its finish matters here).
    let run = fx.run_mut();
    let mut round = run.rounds.last().cloned().expect("round 1's record");
    run.rounds[0].ended_at = Some(1);
    round.n = 2;
    round.first_stage = 2;
    round.outcome = Some(proto::RoundOutcome::Cancelled);
    round.ended_at = None;
    round.summary = None;
    run.rounds.push(round);
    run.finish_edit = true;
    run.round_finish = true;
    fx.done(op, probe_result(false, &spec.commit));
    let ended = |fx: &Fixture| {
        (fx.run().log.iter()).any(|e| e.text.contains("the run ended before the bisect did"))
    };
    assert!(!ended(&fx), "{:#?}", fx.run().log);
    let going = fx.run().stage(1).unwrap().bisect.is_some();
    assert!(
        going || fx.run().task("fix1").is_some(),
        "{:#?}",
        fx.run().log
    );
}
