//! Controller ruling C-22 on task M9.1.17: `run resume --rebaseline` of a `Multi` run
//! re-runs every propagate a moved ref may have dropped (item 2), and a propagate whose
//! lower head the upper stage already holds is recorded as held, with no commit.

use proto::RunState;

use super::fixture::*;
use super::full::later;
use super::merge::{commit, pending, pending_one};
use super::propagate::{merged_at, propagate, propagates};
use super::propagate_c21::t1_propagated;
use crate::run::engine::stages::Rebaseline;
use crate::run::engine::{EventKind, OpResult};
use crate::run::model::StageMerge;

/// Halts the run and resumes it with `--rebaseline`, stage 1 at `one` and stage 2 at
/// `two`.
fn rebaseline(fx: &mut Fixture, one: &str, two: &str) {
    let run = fx.run_mut();
    if run.state != RunState::Halted {
        run.state = RunState::Halted;
        run.halted_reason = Some("moved".into());
    }
    let reply = fx.reply();
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some(Rebaseline {
            base: BASE.into(),
            head: two.into(),
            stages: vec![(1, one.into()), (2, two.into())],
            salvaged: None,
        }),
    });
    assert_eq!(fx.run().state, RunState::Running, "{:#?}", fx.run().log);
}

/// The reviewer's scenario: t1's propagate landed in stage 2, then the user moved the
/// stage-2 ref back to `c2` and resumed with `--rebaseline`. The propagate runs again,
/// and the run completes only once t1 is in stage 2 again.
#[test]
fn a_rebaseline_that_drops_a_propagate_runs_it_again() {
    let mut fx = t1_propagated(merged_at(&commit(3)));
    assert!(fx.run().stage(2).unwrap().tasks_in.contains("t1"));
    // The completion guard finds the moved ref and halts the run.
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    let reason = "anthrex/r/stage-2 moved".to_string();
    fx.done(op, OpResult::RefMoved { reason });
    assert_eq!(fx.run().state, RunState::Halted);
    rebaseline(&mut fx, &commit(1), &commit(2));
    fx.tick();
    let (op, spec) = propagate(&fx);
    assert_eq!((spec.from, spec.to), (1, 2));
    assert_eq!(spec.from_head, commit(1));
    assert_eq!(spec.expected_to_head, commit(2));
    later(&mut fx, 1_000);
    assert!(
        pending(&fx, "VerifyRefs", None).is_empty(),
        "{:#?}",
        fx.run().log
    );
    assert_ne!(fx.run().state, RunState::Complete);
    fx.done(op, merged_at(&commit(5)));
    assert!(fx.run().stage(2).unwrap().tasks_in.contains("t1"));
    assert_eq!(fx.run().stage_head(2), Some(commit(5).as_str()));
    later(&mut fx, 1);
    let (op, _) = pending_one(&fx, "VerifyRefs", None);
    fx.done(op, OpResult::RefsOk);
    for (op, _) in pending(&fx, "Check", None) {
        fx.done(op, super::gates::check_result(true));
    }
    assert_eq!(fx.run().state, RunState::Complete);
}

/// A rebaseline that moves no stage head leaves the propagate it already made alone.
#[test]
fn a_rebaseline_to_the_same_heads_needs_no_propagate() {
    let mut fx = t1_propagated(merged_at(&commit(3)));
    rebaseline(&mut fx, &commit(1), &commit(3));
    fx.tick();
    assert!(propagates(&fx).is_empty());
}

/// Item 2's ancestor case: the upper stage already holds the lower head. No commit is
/// made; the stage records it holds that head and what the lower stage held.
#[test]
fn a_propagate_already_held_records_the_work_without_a_commit() {
    let fx = t1_propagated(OpResult::AlreadyHeld);
    let record = fx.run().stage(2).unwrap();
    assert_eq!(record.head, commit(2));
    assert!(record.tasks_in.contains("t1"), "{:?}", record.tasks_in);
    assert_eq!(record.synced_from.as_deref(), Some(commit(1).as_str()));
    assert!(
        !record
            .merges
            .iter()
            .any(|m| matches!(m, StageMerge::Propagate { .. })),
        "{:?}",
        record.merges
    );
    assert!(propagates(&fx).is_empty());
}

/// A stage the user moved away from what it was created from is not taken to hold the
/// lower head, even when the lower stage never moved: its propagate runs (and comes
/// back `AlreadyHeld` when nothing is missing).
#[test]
fn a_rebaseline_that_moves_only_the_upper_stage_checks_it_again() {
    let (mut fx, _) = super::propagate::stages(false);
    assert!(propagates(&fx).is_empty());
    rebaseline(&mut fx, BASE, &commit(7));
    fx.tick();
    let (op, spec) = propagate(&fx);
    assert_eq!(
        (spec.from_head, spec.expected_to_head),
        (BASE.into(), commit(7))
    );
    fx.done(op, OpResult::AlreadyHeld);
    fx.tick();
    assert!(propagates(&fx).is_empty());
    assert_eq!(fx.run().stage_head(2), Some(commit(7).as_str()));
}
