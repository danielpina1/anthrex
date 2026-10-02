//! Milestone 9.2 task M9.2.9, decision 27 step 6: after `ci_fix_max` CI fix tasks with
//! the same key on one stage, the next red with that key goes to the user (an attention
//! line and a wake note) and adds no task; a red with another key still gets one; `0`
//! sends every red to the user.

use proto::CiCategory;

use super::bisect::with_orchestrator;
use super::delivery_ci::{
    RUN_A, answer_logs, ci_decide, ci_fixes, ci_record, deciding, job_check, red_view, summary,
    test_red, tier2,
};
use super::delivery_watch::{poll_with, watched};
use super::fixture::*;
use super::full::{attention, delivery_alerts, outcome, tier};
use super::merge::{commit, pending};
use super::wake_notes::notes;
use crate::decider::Decision;
use crate::host::Conclusion;
use crate::run::delivery::CiPhase;
use crate::run::engine::OpKind;
use crate::run::engine::stages::set_stage_head;
use crate::run::model::FixOf;

/// One red round on stage 1 at `commit(k)` (pushed, green on tier 3 locally): its log,
/// the decider's `answer` (deciders off: the fallback), a tier-2 reproduction answered
/// green when one is asked for. The CI fix task it added, if any.
pub(super) fn round(fx: &mut Fixture, k: u32, answer_with: Option<Decision>) -> Option<String> {
    let head = commit(k);
    // The final fix wave's I-2: a red while the key's fix is unfinished joins it, so each
    // round's fix has merged before the next red.
    let now = fx.now;
    for t in fx.run_mut().tasks.iter_mut() {
        if t.origin == proto::TaskOrigin::Ci && !t.state.is_finished() {
            crate::run::phases::set_state(t, proto::TaskState::Merged, now);
        }
    }
    if fx.run().stage_head(1) != Some(head.as_str()) {
        set_stage_head(fx.run_mut(), 1, &head);
        fx.run_mut().stages[0].full.green_at = Some(head.clone());
        fx.run_mut().delivery.stages[0]
            .pr
            .as_mut()
            .unwrap()
            .pushed_head = head.clone();
    }
    let before = ci_fixes(fx);
    let run = RUN_A + 100 * u64::from(k);
    poll_with(fx, red_view(&head, test_red(run)));
    answer_logs(fx, "--- FAIL: a::works");
    if let Some(decision) = answer_with {
        let (op, _) = ci_decide(fx);
        fx.decided(op, decision);
    }
    if let Some((op, _)) = tier2(fx) {
        fx.done(op, tier(outcome(2, &[])));
    }
    if let Some((op, _)) = pending(fx, "TestAt", None).first().cloned() {
        fx.done(
            op,
            crate::run::engine::OpResult::TestAt {
                red: true,
                failing: vec!["cargo test -- --exact b::other".into()],
                tail: String::new(),
                show: None,
            },
        );
    }
    ci_fixes(fx).into_iter().find(|f| !before.contains(f))
}

#[test]
fn ci_fix_max_sends_the_same_red_to_the_user() {
    let mut fx = watched();
    with_orchestrator(&mut fx);
    assert_eq!(fx.run().delivery.limits.ci_fix_max, 2, "the default");
    assert!(round(&mut fx, 1, None).is_some());
    assert!(round(&mut fx, 2, None).is_some());
    // The third red with the same key: over to the user, no task, no reproduction.
    let ops_before = fx.log.len();
    assert_eq!(round(&mut fx, 3, None), None);
    let line = "stage 1 CI still red on unknown after 2 fix tasks; over to you";
    assert!(
        attention(&fx).contains(&line.to_string()),
        "{:#?}",
        attention(&fx)
    );
    assert!(notes(&fx).contains(&line.to_string()), "{:#?}", notes(&fx));
    let kind = proto::DeliveryAlertKind::CiHandedToUser;
    assert_eq!(
        delivery_alerts(&fx),
        vec![(kind, Some(1), line.to_string())]
    );
    assert_eq!(ci_record(&fx).phase, CiPhase::ToUser);
    assert!(ops_in(&fx.log[ops_before..], "Tier").is_empty());
    assert_eq!(ci_fixes(&fx).len(), 2);
    // The line stays until the stage's CI is green.
    poll_with(
        &mut fx,
        red_view(
            &commit(3),
            vec![job_check("test", Conclusion::Success, RUN_A, 77)],
        ),
    );
    assert!(!attention(&fx).contains(&line.to_string()));
}

#[test]
fn another_failing_set_still_gets_a_task() {
    let mut fx = watched();
    fx.run_mut().delivery.limits.ci_fix_max = 1;
    assert!(round(&mut fx, 1, None).is_some());
    assert_eq!(round(&mut fx, 2, None), None, "the cap of `unknown`");
    // A red with other failing tests is its own key.
    deciding(&mut fx);
    let other = summary(&["b::other failed"], &["b::other"], CiCategory::Test);
    let fix = round(&mut fx, 3, Some(other)).expect("a task for another key");
    let task = fx.task(&fix);
    assert!(
        matches!(&task.fixes, Some(FixOf::Ci { key, .. }) if key == "b::other"),
        "{:?}",
        task.fixes
    );
}

#[test]
fn ci_fix_max_zero_sends_every_red_to_the_user() {
    let mut fx = watched();
    fx.run_mut().delivery.limits.ci_fix_max = 0;
    assert_eq!(round(&mut fx, 1, None), None);
    let line = "stage 1 CI still red on unknown after 0 fix tasks; over to you";
    assert!(
        attention(&fx).contains(&line.to_string()),
        "{:#?}",
        attention(&fx)
    );
    assert!(tier2(&fx).is_none());
    assert!(
        ops_in(&fx.log, "Tier")
            .iter()
            .all(|(_, k)| !matches!(k, OpKind::Tier(s) if s.tier == 2))
    );
}
