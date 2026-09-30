//! Milestone 9.1 task M9.1.18, ruling C-23: a propagate's tier-2 job is recorded like
//! any tier job, and the review's surviving mutants of the history lines are pinned.

use std::path::PathBuf;

use proto::{HistoryLine, TaskState};

use super::bisect::{TEST, merge_next, red_full};
use super::control::resume;
use super::control_restore::restart;
use super::fixture::*;
use super::full::{full_job, merge_tiered, outcome, profile, tier, verify_ok};
use super::gates::only_op;
use super::history_tiers::{answer_collecting, bisect_lines, flaky_lines, lines, tier_lines};
use super::merge::{commit, doc_task, start_on, window_of};
use super::propagate::{land_propagates, propagate, stages_on};
use super::tiers::{proved, working};
use crate::run::engine::{EventKind, OpKind, OpResult};
use crate::run::roster::peer;
use crate::run::test_support::task_toml;
use crate::run::tiers::StepKind;

const REPO: &str = "/tmp/data/repos/r-1";

/// Answers every history op (every task's too), keeping the lines, until none is
/// left: completion waits for an empty `pending_ops`.
fn flush(fx: &mut Fixture, all: &mut Vec<(String, HistoryLine)>) {
    loop {
        let ops: Vec<(crate::run::model::OpId, OpKind)> = fx
            .run()
            .pending_ops
            .values()
            .filter(|p| {
                matches!(
                    p.kind,
                    OpKind::AppendHistory { .. } | OpKind::MeasureDiff { .. }
                )
            })
            .map(|p| (p.op, p.kind.clone()))
            .collect();
        if ops.is_empty() {
            return;
        }
        for (op, kind) in ops {
            let result = match kind {
                OpKind::AppendHistory {
                    record_id, line, ..
                } => {
                    all.push((record_id, *line));
                    OpResult::HistoryAppended
                }
                _ => OpResult::Failed {
                    message: "not measured".into(),
                },
            };
            fx.done(op, result);
        }
    }
}

/// A `Multi` run on the tiered profile, `t1` in stage 1 and `t2` in stage 2, with
/// history on and `t1` merged: its propagate into stage 2 is pending.
fn propagating() -> Fixture {
    let tasks = [doc_task("t1", ""), doc_task("t2", "stage = 2")];
    let (mut fx, windows) = stages_on(&profile(), &tasks);
    fx.run_mut().repo_dir = PathBuf::from(REPO);
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    fx
}

#[test]
fn a_propagates_tier2_job_writes_its_tier_and_flaky_lines() {
    let mut fx = propagating();
    let run_id = fx.run().id.clone();
    let (op, _) = propagate(&fx);
    let mut green = outcome(2, &[]);
    green.steps[0].retried = true;
    green.steps[0].flaky = vec!["a::flaky".into()];
    let all = lines(&fx.done(
        op,
        OpResult::Merged {
            commit: commit(5),
            tier: Some(Box::new(green)),
        },
    ));
    let tiers = tier_lines(&all);
    assert_eq!(tiers.len(), 1, "{all:#?}");
    let t = &tiers[0];
    assert_eq!(t.record_id, format!("{run_id}/tier/{op}"));
    assert_eq!(
        (t.task_id.clone(), t.stage, t.tier, t.ok),
        (None, 2, 2, true)
    );
    assert_eq!(t.flaky, ["a::flaky"]);
    let flakes = flaky_lines(&all);
    assert_eq!(flakes.len(), 1, "{all:#?}");
    assert_eq!(flakes[0].record_id, format!("{run_id}/flaky/{op}/a::flaky"));
    assert_eq!((flakes[0].task_id.clone(), flakes[0].tier), (None, 2));

    // A red propagate is a tier job too.
    let mut fx = propagating();
    let (op, _) = propagate(&fx);
    let all = lines(&fx.done(
        op,
        OpResult::CandidateRed {
            code: Some(101),
            timed_out: false,
            tail: "FAILED".into(),
            secs: 3,
            tier: Some(Box::new(outcome(2, &[TEST]))),
        },
    ));
    let tiers = tier_lines(&all);
    assert_eq!(tiers.len(), 1, "{all:#?}");
    assert_eq!((tiers[0].stage, tiers[0].tier, tiers[0].ok), (2, 2, false));
    assert!(flaky_lines(&all).is_empty());
}

/// M1 (deviation 3): one step hit and one ran, and there is no build step: `cache_hit`
/// is "any step hit", not "every step hit".
#[test]
fn a_job_with_one_cached_step_is_a_cache_hit() {
    let (mut fx, window) = working();
    let effects = proved(&mut fx, window);
    let (op, _) = only_op(&effects, "Tier");
    let mut job = super::tiers::outcome(1, true, false);
    job.steps[0].kind = StepKind::Tests;
    job.steps[0].cached = true;
    job.steps[1].kind = StepKind::Timing;
    let all = lines(&fx.done(op, OpResult::Tier(Box::new(job))));
    let t = &tier_lines(&all)[0];
    assert_eq!((t.cache_hit, t.cached_steps, t.steps), (true, 1, 2));
}

/// M2: the same test flaky in two steps is one flake.
#[test]
fn a_test_flaky_in_two_steps_is_one_flaky_line() {
    let (mut fx, window) = working();
    let effects = proved(&mut fx, window);
    let (op, _) = only_op(&effects, "Tier");
    let mut job = super::tiers::outcome(1, true, false);
    for step in &mut job.steps {
        step.retried = true;
        step.flaky = vec!["a::flaky".into()];
    }
    let all = lines(&fx.done(op, OpResult::Tier(Box::new(job))));
    assert_eq!(flaky_lines(&all).len(), 1, "{all:#?}");
    assert_eq!(tier_lines(&all)[0].flaky, ["a::flaky"]);
}

/// T1 (deviation 2): a tier-1 result the task no longer awaits is still a job that ran.
#[test]
fn an_unawaited_tier1_result_is_still_recorded() {
    let (mut fx, window) = working();
    let effects = proved(&mut fx, window);
    let (op, _) = only_op(&effects, "Tier");
    fx.task_mut("t1").state = TaskState::Cancelled;
    let job = super::tiers::outcome(1, true, false);
    let all = lines(&fx.done(op, OpResult::Tier(Box::new(job))));
    let tiers = tier_lines(&all);
    assert_eq!(tiers.len(), 1, "{all:#?}");
    assert_eq!(
        (tiers[0].task_id.as_deref(), tiers[0].tier),
        (Some("t1"), 1)
    );
    assert!(
        fx.task("t1").checks.is_empty(),
        "the task took nothing from it"
    );
}

/// T2, T3, T4: the stage of every tier line is the job's, here stage 2.
#[test]
fn tier_lines_carry_their_stage_at_every_tier() {
    let tasks = [doc_task("t1", ""), doc_task("t2", "stage = 2")];
    let (mut fx, windows) = stages_on(&profile(), &tasks);
    fx.run_mut().repo_dir = PathBuf::from(REPO);
    let mut all = Vec::new();
    merge_tiered(&mut fx, "t1", window_of(&windows, "t1"), &commit(1));
    land_propagates(&mut fx);
    merge_tiered(&mut fx, "t2", window_of(&windows, "t2"), &commit(2));
    flush(&mut fx, &mut all);
    let (op, spec) = super::full::full_jobs(&verify_ok(&mut fx))[0].clone();
    assert_eq!(spec.stage, 1);
    fx.done(op, tier(outcome(3, &[])));
    flush(&mut fx, &mut all);
    let (op, spec) = super::full::full_jobs(&verify_ok(&mut fx))[0].clone();
    assert_eq!(spec.stage, 2);
    fx.done(op, tier(outcome(3, &[])));
    flush(&mut fx, &mut all);

    let tiers = tier_lines(&all);
    let t2: Vec<(u8, u16)> = tiers
        .iter()
        .filter(|t| t.task_id.as_deref() == Some("t2"))
        .map(|t| (t.tier, t.stage))
        .collect();
    assert_eq!(t2, [(1, 2), (2, 2)], "{tiers:#?}");
    let full: Vec<u16> = tiers
        .iter()
        .filter(|t| t.tier == 3)
        .map(|t| t.stage)
        .collect();
    assert_eq!(full, [1, 2], "{tiers:#?}");
}

/// M10/B2: a culprit whose fix task M8a's rules refuse on both routes.
#[test]
fn a_refused_fix_task_records_the_culprit_and_the_reason() {
    // t3 and t4 overlap t2's `owns` and stay open, one on each runtime, so a fix task
    // on either runtime breaks rule 9.
    let glue = "test_mode = \"check\"\ntest_mode_reason = \"glue code\"";
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", ""),
        task_toml("t3", "S", "[\"docs/t2/more/**\"]", glue),
        task_toml("t4", "S", "[\"docs/t2/other/**\"]", glue),
    ];
    let (mut fx, mut windows) = start_on(&profile(), &tasks);
    fx.run_mut().repo_dir = PathBuf::from(REPO);
    let own = fx.task("t2").route.runtime;
    fx.task_mut("t3").spec.route.runtime = Some(own);
    fx.task_mut("t4").spec.route.runtime = Some(peer(own));
    merge_next(&mut fx, &mut windows, "t1", &commit(1));
    merge_next(&mut fx, &mut windows, "t2", &commit(2));
    let since = fx.run().queue_idle_since.expect("idle");
    fx.send(since + 120, EventKind::Tick);
    let (op, _) = full_job(&fx);
    let mut all = lines(&fx.done(op, tier(outcome(3, &[TEST]))));
    answer_collecting(&mut fx, 2, &mut all);
    assert!(fx.run().task("fix1").is_none(), "refused");
    let bisects = bisect_lines(&all);
    assert_eq!(bisects.len(), 1, "{all:#?}");
    let b = &bisects[0];
    assert_eq!(
        (b.culprit.as_deref(), b.fix_task.as_deref()),
        (Some("t2"), None)
    );
    let reason = b.reason.clone().unwrap_or_default();
    assert!(reason.starts_with("fix task refused: "), "{reason}");
    assert!(reason.contains("(rule 9)"), "{reason}");
}

/// B1: a stage's second bisect is `/2`, and the count survives a restart.
#[test]
fn a_stages_second_bisect_is_numbered_two_after_a_restart() {
    let tasks = [doc_task("t1", ""), doc_task("t2", "")];
    let (mut fx, mut windows) = start_on(&profile(), &tasks);
    fx.run_mut().repo_dir = PathBuf::from(REPO);
    let run_id = fx.run().id.clone();
    merge_next(&mut fx, &mut windows, "t1", &commit(1));
    merge_next(&mut fx, &mut windows, "t2", &commit(2));
    let mut all = Vec::new();
    flush(&mut fx, &mut all);
    red_full(&mut fx);
    answer_collecting(&mut fx, 2, &mut Vec::new());
    flush(&mut fx, &mut all);
    assert_eq!(fx.run().stage(1).unwrap().full.bisects, 1);

    let text = serde_json::to_string(fx.run()).unwrap();
    let stored: crate::run::model::Run = serde_json::from_str(&text).unwrap();
    assert_eq!(stored.stages[0].full.bisects, 1);
    *fx.run_mut() = stored;
    restart(&mut fx, Vec::new());
    resume(&mut fx);
    merge_next(&mut fx, &mut windows, "fix1", &commit(3));
    flush(&mut fx, &mut all);
    red_full(&mut fx);
    answer_collecting(&mut fx, 2, &mut Vec::new());
    flush(&mut fx, &mut all);
    let ids: Vec<String> = bisect_lines(&all)
        .into_iter()
        .map(|b| b.record_id)
        .collect();
    assert_eq!(
        ids,
        [
            format!("{run_id}/bisect/1/1"),
            format!("{run_id}/bisect/1/2")
        ]
    );
}
