//! Whole-branch review of milestone 9.1, ruling C-27: the reviewer's three failing
//! scenarios, adapted (items 3 and 4), and the refused resume and restarted backoff of
//! items 5 and 6.

use proto::{PlanEdit, RunState};

use super::bisect::{answer, probe, probe_result, with_orchestrator};
use super::control::resume;
use super::control_restore::restart;
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::full::{attention, block, full_job, later, profile};
use super::merge::{commit, pending, start_on, window_of};
use super::propagate::land_propagates;
use super::scenarios::{
    bisecting_with_stage2, five, halt_and_rebaseline, history_log, invariants, red_tier3, roundtrip,
};
use super::wake_notes::notes;
use crate::run::engine::bisect::REBASELINED;
use crate::run::engine::{OpKind, OpResult};
use crate::run::model::{FixOf, StageMerge};

fn task_merge(id: &str, n: u32) -> StageMerge {
    StageMerge::Task {
        id: id.into(),
        commit: commit(n),
    }
}

/// Scenario s3: a rebaseline that drops t2's merge from stage 1 while stage 1 is
/// bisected (G green, H's red result recorded while halted) ends the bisect
/// `rebaselined`, writes its history line, truncates the stage's line and forgets the
/// red on the dropped commit; nothing is probed and no fix task blames t2.
#[test]
fn a_rebaseline_during_a_bisect_ends_it_and_blames_nothing() {
    let (mut fx, _windows) = bisecting_with_stage2();
    let (op, spec) = probe(&fx);
    assert_eq!(spec.commit, BASE);
    fx.done(op, probe_result(false, BASE));
    let (h_op, h) = probe(&fx);
    assert_eq!(h.commit, commit(2));
    fx.run_mut().state = RunState::Halted;
    fx.run_mut().halted_reason = Some("moved".into());
    let effects = fx.done(h_op, probe_result(true, &commit(2)));
    assert!(ops_in(&effects, "TestAt").is_empty());
    let s2_head = fx.run().stage_head(2).unwrap().to_string();
    let effects = halt_and_rebaseline(&mut fx, vec![(1, commit(1)), (2, s2_head)]);
    assert_eq!(fx.run().state, RunState::Running, "{effects:#?}");

    let s = fx.run().stage(1).unwrap().clone();
    assert!(s.bisect.is_none(), "{:#?}", s.bisect);
    assert_eq!(s.merges, [task_merge("t1", 1)]);
    assert_eq!(s.full.red_at, None);
    assert_eq!(s.full.note, None);
    let ended = s.full.ended.last().expect("the ended bisect");
    assert_eq!(ended.reason.as_deref(), Some(REBASELINED));
    assert_eq!((&ended.culprit, &ended.fix_task), (&None, &None));
    let lines: Vec<String> = history_log(&effects)
        .into_iter()
        .filter(|(id, _)| id.ends_with("/bisect/1/1"))
        .map(|(_, line)| line)
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].contains("\"reason\":\"rebaselined\""),
        "{}",
        lines[0]
    );
    invariants(&fx, "after the rebaseline");

    let effects = later(&mut fx, 10);
    let probed: Vec<String> = ops_in(&effects, "TestAt")
        .into_iter()
        .filter_map(|(_, k)| match k {
            OpKind::TestAt(s) => Some(s.commit),
            _ => None,
        })
        .collect();
    assert!(!probed.contains(&commit(2)), "{probed:?}");
    assert!(
        fx.run().task("fix1").is_none(),
        "a fix task after a rebaseline"
    );
    invariants(&fx, "after the rebaseline's pass");
}

/// A probe in flight at the rebaseline is dropped, and its late result changes nothing.
#[test]
fn a_probe_in_flight_at_a_rebaseline_is_dropped_and_its_late_result_ignored() {
    let (mut fx, _windows) = bisecting_with_stage2();
    let (op, _) = probe(&fx);
    let s2_head = fx.run().stage_head(2).unwrap().to_string();
    halt_and_rebaseline(&mut fx, vec![(1, commit(1)), (2, s2_head)]);
    assert!(!fx.run().pending_ops.contains_key(&op));
    assert_ne!(fx.run().full_op, Some(op));
    let before = fx.run().stages.clone();
    fx.done(op, probe_result(true, BASE));
    assert_eq!(
        fx.run().stages,
        before,
        "a late probe result changed the stages"
    );
    assert!(fx.run().task("fix1").is_none());
    invariants(&fx, "late result");
}

/// Scenario s3b: after a rebaseline that dropped c2 from stage 1, a later red bisects
/// over the stage's line as it now is: c2 is never probed and t3 is blamed.
#[test]
fn a_bisect_after_a_rebaseline_never_probes_a_dropped_commit() {
    let (mut fx, mut windows) = five();
    super::bisect::merge_next(&mut fx, &mut windows, "t1", &commit(1));
    land_propagates(&mut fx);
    super::bisect::merge_next(&mut fx, &mut windows, "t2", &commit(2));
    land_propagates(&mut fx);
    let s2_head = fx.run().stage_head(2).unwrap().to_string();
    halt_and_rebaseline(&mut fx, vec![(1, commit(1)), (2, s2_head)]);
    assert_eq!(fx.run().state, RunState::Running);
    assert_eq!(fx.run().stage(1).unwrap().merges, [task_merge("t1", 1)]);
    fx.tick();
    land_propagates(&mut fx);
    super::bisect::merge_next(&mut fx, &mut windows, "t3", &commit(3));
    land_propagates(&mut fx);
    red_tier3(&mut fx, 1);
    let probed = answer(&mut fx, 3);
    assert!(!probed.contains(&commit(2)), "{probed:?}");
    assert_eq!(probed, [BASE.to_string(), commit(3), commit(1)]);
    assert!(
        matches!(&fx.task("fix1").fixes, Some(FixOf::Bisect { culprit, .. }) if culprit == "t3")
    );
    invariants(&fx, "after the bisect");
}

/// Scenario y: a bisect's fix task cancelled leaves the stage red at its head with an
/// attention line naming it, and the orchestrator is woken with it exactly once.
#[test]
fn a_cancelled_bisect_fix_task_is_an_attention_line_and_one_wake() {
    let mut fx = super::bisect::merged(&["t1", "t2"], "");
    with_orchestrator(&mut fx);
    super::bisect::red_full(&mut fx);
    answer(&mut fx, 2);
    assert!(fx.run().task("fix1").is_some());
    let w = fx.launch_all();
    let window = w.iter().find(|(t, _)| t == "fix1").expect("fix1 window").1;
    super::wake_notes::clear(&mut fx);
    let effects = edit(
        &mut fx,
        vec![PlanEdit::CancelTask {
            task_id: "fix1".into(),
        }],
    );
    assert!(replies(&effects)[0].is_ok(), "{:?}", replies(&effects));
    super::turns::killed_exit(&mut fx, window);
    for _ in 0..5 {
        later(&mut fx, 200);
        for (op, _) in pending(&fx, "RemoveWorktree", Some("fix1")) {
            fx.done(op, OpResult::Removed { salvage_ref: None });
        }
    }
    let head = fx.run().stage_head(1).unwrap().to_string();
    let text = format!(
        "fix task fix1 ended without merging; tier 3 is red at {}",
        &head[..7]
    );
    assert!(attention(&fx).contains(&text), "{:?}", attention(&fx));
    let woken: Vec<String> = notes(&fx).into_iter().filter(|n| *n == text).collect();
    assert_eq!(woken.len(), 1, "{:?}", notes(&fx));
    // Completion still holds on the red head.
    assert_eq!(fx.run().state, RunState::Running);
    later(&mut fx, 600);
    let woken: Vec<String> = notes(&fx).into_iter().filter(|n| *n == text).collect();
    assert_eq!(woken.len(), 1, "woken again: {:?}", notes(&fx));
}

/// A two-task run whose tier 3 on stage 1 failed in the executor once (ruling C-18).
fn one_infra_failure() -> Fixture {
    let tasks = [
        super::merge::doc_task("t1", ""),
        super::merge::doc_task("t2", ""),
    ];
    let (mut fx, mut windows) = start_on(&profile(), &tasks);
    super::bisect::merge_next(&mut fx, &mut windows, "t1", &commit(1));
    block(&mut fx, "t2", window_of(&windows, "t2"));
    let since = fx.run().queue_idle_since.expect("idle");
    fx.send(since + 120, crate::run::engine::EventKind::Tick);
    fail_job(&mut fx);
    fx
}

fn fail_job(fx: &mut Fixture) {
    let (op, _) = full_job(fx);
    fx.done(
        op,
        OpResult::Failed {
            message: "executor died".into(),
        },
    );
}

fn infra_count(fx: &Fixture) -> u8 {
    fx.run()
        .stage(1)
        .unwrap()
        .full
        .infra
        .as_ref()
        .map_or(0, |i| i.count)
}

/// Item 5: `run resume` of a running run with no held stage is refused and changes
/// nothing: the stage keeps its backoff and no tier 3 starts.
#[test]
fn a_refused_resume_of_a_running_run_changes_nothing() {
    let mut fx = one_infra_failure();
    assert_eq!(infra_count(&fx), 1);
    let before = fx.run().clone();
    let effects = resume(&mut fx);
    let reply = replies(&effects);
    assert!(
        reply[0].as_ref().is_err_and(|e| e.ends_with("is running")),
        "{reply:?}"
    );
    assert_eq!(fx.run().stages, before.stages);
    assert_eq!(fx.run().pending_ops, before.pending_ops);
    assert_eq!(fx.run().full_op, before.full_op);
    assert!(super::full::full_jobs(&effects).is_empty(), "{effects:#?}");
}

/// Item 6: two executor failures, a restart and its resume, then a third: the stage is
/// held, as it would be without the restart.
#[test]
fn infra_failures_survive_a_restart_and_its_resume() {
    let mut fx = one_infra_failure();
    later(&mut fx, 200);
    fail_job(&mut fx);
    assert_eq!(infra_count(&fx), 2);
    roundtrip(&mut fx);
    restart(&mut fx, Vec::new());
    resume(&mut fx);
    assert_eq!(infra_count(&fx), 2, "the resume reset the count");
    later(&mut fx, 700);
    fail_job(&mut fx);
    assert_eq!(infra_count(&fx), 3);
    let held = attention(&fx).iter().any(|l| {
        l.starts_with("stage 1: could not run tier 3") && l.ends_with("anthrex run resume retries")
    });
    assert!(held, "{:?}", attention(&fx));
    // Held: no fourth job however long it waits.
    let effects = later(&mut fx, 3_600);
    assert!(super::full::full_jobs(&effects).is_empty(), "{effects:#?}");
}
