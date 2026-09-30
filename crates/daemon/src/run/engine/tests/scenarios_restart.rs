//! Whole-branch review of milestone 9.1 (ruling C-27): the cross-task scenarios over
//! a restart, a pause and a late or replayed result, kept from the reviewer's set.
//! Each checks `scenarios::invariants` at every interesting step.

use proto::PlanEdit;

use super::bisect::{TEST, merge_next};
use super::bisect::{answer, probe, probe_result};
use super::control::resume;
use super::control_restore::restart;
use super::dispatch::edit;
use super::fixture::*;
use super::full::profile;
use super::full::{full_job, later, outcome, tier};
use super::merge::commit;
use super::merge::doc_task;
use super::propagate::{land_propagates, merged_at, propagate, propagates, stages_on};
use super::scenarios::{
    bisecting_with_stage2, five, flush_history, history_log, invariants, probes_pending, roundtrip,
};
use crate::run::engine::OpKind;
use crate::run::engine::full::{FullWhy, request};
use crate::run::model::OpId;
use proto::RunState;

#[test]
fn s4a_restart_mid_bisect_probe_lost_and_late_result() {
    let (mut fx, mut windows) = bisecting_with_stage2();
    merge_next(&mut fx, &mut windows, "t3", &commit(3));
    let (lost, spec) = probe(&fx);
    let (prop_lost, _) = propagate(&fx);
    roundtrip(&mut fx);
    restart(&mut fx, Vec::new());
    assert_eq!(fx.run().state, RunState::Paused);
    assert!(
        fx.run()
            .pending_ops
            .values()
            .all(|p| !matches!(p.kind, OpKind::TestAt(_) | OpKind::Propagate(_)))
    );
    // A late result of the pre-restart ops is ignored.
    let before = fx.run().clone();
    fx.done(lost, probe_result(false, &spec.commit));
    fx.done(prop_lost, merged_at(&commit(33)));
    let after = fx.run();
    assert_eq!(
        after.stages, before.stages,
        "a late result changed the stages"
    );
    let effects = resume(&mut fx);
    invariants(&fx, "resumed");
    let probes = ops_in(&effects, "TestAt");
    let props = ops_in(&effects, "Propagate");
    assert_eq!(probes.len(), 1, "{effects:#?}");
    assert_eq!(props.len(), 1, "{effects:#?}");
    assert!(matches!(&probes[0].1, OpKind::TestAt(s) if s.commit == BASE));
    let probed = answer(&mut fx, 2);
    assert_eq!(probed, [BASE.to_string(), commit(2), commit(1)]);
    land_propagates(&mut fx);
    invariants(&fx, "end");
    flush_history(&mut fx);
    let run_id = fx.run().id.clone();
    let ids: Vec<String> = history_log(&fx.log)
        .into_iter()
        .map(|(i, _)| i)
        .filter(|i| i.contains("/bisect/"))
        .collect();
    assert_eq!(ids, [format!("{run_id}/bisect/1/1")]);
}

#[test]
fn s4a2_restart_with_the_probe_result_replayed() {
    let (mut fx, _) = bisecting_with_stage2();
    let (op, spec) = probe(&fx);
    roundtrip(&mut fx);
    restart(&mut fx, vec![(op, probe_result(false, &spec.commit))]);
    let b = fx.run().stage(1).unwrap().bisect.clone().unwrap();
    assert_eq!((b.probes, b.probe), (1, None));
    assert!(probes_pending(&fx).is_empty(), "paused: no probe");
    let effects = resume(&mut fx);
    let probes = ops_in(&effects, "TestAt");
    assert_eq!(probes.len(), 1, "{effects:#?}");
    assert!(matches!(&probes[0].1, OpKind::TestAt(s) if s.commit == commit(2)));
    invariants(&fx, "resumed");
    // The old op's result again (a duplicate delivery) is ignored.
    let before = fx.run().stages.clone();
    fx.done(op, probe_result(true, &spec.commit));
    assert_eq!(fx.run().stages, before);
}

#[test]
fn s4b_restart_mid_propagate_notstarted_and_replayed() {
    for replay in [false, true] {
        let (mut fx, mut windows) = five();
        merge_next(&mut fx, &mut windows, "t1", &commit(1));
        let (op, _) = propagate(&fx);
        roundtrip(&mut fx);
        let replayed = if replay {
            vec![(op, merged_at(&commit(21)))]
        } else {
            Vec::new()
        };
        restart(&mut fx, replayed);
        if replay {
            assert_eq!(fx.run().stage_head(2), Some(commit(21).as_str()));
        }
        let effects = resume(&mut fx);
        invariants(&fx, "resumed");
        let props = ops_in(&effects, "Propagate");
        if replay {
            assert!(
                props.is_empty(),
                "replayed merged: not issued again: {effects:#?}"
            );
            assert!(propagates(&fx).is_empty());
        } else {
            assert_eq!(props.len(), 1, "{effects:#?}");
            assert_ne!(props[0].0, op);
            // A late result of the lost op is ignored.
            fx.done(op, merged_at(&commit(99)));
            assert_ne!(fx.run().stage_head(2), Some(commit(99).as_str()));
            assert_eq!(propagates(&fx).len(), 1);
            land_propagates(&mut fx);
        }
        assert!(fx.run().stage(2).unwrap().tasks_in.contains("t1"));
        invariants(&fx, "end");
    }
}

#[test]
fn x_test_at_after_bisect_ended_by_finish() {
    let (mut fx, _) = bisecting_with_stage2();
    let (op, spec) = probe(&fx);
    edit(&mut fx, vec![PlanEdit::Finish]);
    fx.done(op, probe_result(false, &spec.commit));
    assert!(fx.run().stages.iter().all(|s| s.bisect.is_none()));
    // Now a late second result for the same op.
    fx.done(op, probe_result(true, &spec.commit));
    invariants(&fx, "late");
}

/// Restart with the red tier-3 result replayed: the bisect starts paused and its first
/// probe goes out once on resume.
#[test]
fn y_restart_with_red_tier3_replayed() {
    let (mut fx, mut windows) = five();
    merge_next(&mut fx, &mut windows, "t1", &commit(1));
    land_propagates(&mut fx);
    let now = fx.now;
    let mut effects = Vec::new();
    assert!(request(
        fx.run_mut(),
        1,
        FullWhy::Deliver,
        now,
        &mut effects
    ));
    let (op, _) = full_job(&fx);
    roundtrip(&mut fx);
    let effects = restart(&mut fx, vec![(op, tier(outcome(3, &[TEST])))]);
    assert!(ops_in(&effects, "TestAt").is_empty());
    assert!(fx.run().stage(1).unwrap().bisect.is_some());
    let effects = resume(&mut fx);
    assert_eq!(ops_in(&effects, "TestAt").len(), 1, "{effects:#?}");
    invariants(&fx, "resumed");
    let run_id = fx.run().id.clone();
    let tier3: Vec<String> = history_log(&fx.log)
        .into_iter()
        .map(|(i, _)| i)
        .filter(|i| *i == format!("{run_id}/tier/{op}"))
        .collect();
    assert_eq!(tier3.len(), 1, "{tier3:?}");
}

/// A bisect line lost in a restart is re-emitted under the same id and content.
#[test]
fn y_lost_bisect_line_is_reemitted_identically() {
    let (mut fx, _) = bisecting_with_stage2();
    answer(&mut fx, 2);
    let lines: Vec<(OpId, String)> = fx
        .run()
        .pending_ops
        .values()
        .filter_map(|p| match &p.kind {
            OpKind::AppendHistory { record_id, .. } if record_id.contains("/bisect/") => {
                Some((p.op, record_id.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    roundtrip(&mut fx);
    restart(&mut fx, Vec::new());
    invariants(&fx, "restart");
    let again: Vec<(OpId, String)> = fx
        .run()
        .pending_ops
        .values()
        .filter_map(|p| match &p.kind {
            OpKind::AppendHistory { record_id, .. } if record_id.contains("/bisect/") => {
                Some((p.op, record_id.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(again.len(), 1);
    assert_eq!(again[0].1, lines[0].1);
    assert_ne!(again[0].0, lines[0].0);
}

/// A propagate result arriving while paused is recorded; the next one waits.
#[test]
fn y_propagate_result_while_paused() {
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", "stage = 2"),
        doc_task("t3", "stage = 3"),
    ];
    let (mut fx, mut windows) = stages_on(&profile(), &tasks);
    merge_next(&mut fx, &mut windows, "t1", &commit(1));
    let (op, _) = propagate(&fx);
    edit(&mut fx, vec![PlanEdit::Pause]);
    let effects = fx.done(op, merged_at(&commit(21)));
    assert_eq!(fx.run().stage_head(2), Some(commit(21).as_str()));
    assert!(ops_in(&effects, "Propagate").is_empty());
    assert!(ops_in(&later(&mut fx, 500), "Propagate").is_empty());
    let effects = edit(&mut fx, vec![PlanEdit::Resume]);
    let props = ops_in(&effects, "Propagate");
    assert_eq!(props.len(), 1, "{effects:#?}");
    assert!(matches!(&props[0].1, OpKind::Propagate(s) if s.from == 2 && s.to == 3));
    invariants(&fx, "resumed");
}
