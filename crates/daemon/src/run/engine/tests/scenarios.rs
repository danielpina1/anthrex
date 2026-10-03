//! Whole-branch review of milestone 9.1 (ruling C-27): cross-task scenarios over the
//! engine, kept from the reviewer's throwaway set because each crosses two or more of
//! the milestone's tasks (bisect, propagate, stages, edits, pause, restart, history).
//! Every scenario checks [`invariants`] at each interesting step: one queue op at a
//! time, one tier-3 job or probe at a time, one bisect at a time, no history record id
//! reused with other content, and no completion while work is undelivered or bisected.
//! The helpers are shared with `scenarios_c27.rs`.

use std::collections::BTreeMap;
use std::path::PathBuf;

use proto::{PlanEdit, PlanTask, RunState, TaskState};

use super::bisect::{TEST, answer, merge_next, probe, probe_result, with_orchestrator};
use super::dispatch::{edit, replies};
use super::fixture::*;
use super::full::{full_job, later, outcome, profile, tier};
use super::merge::{claim, commit, doc_task, head_of, pending, pending_one, window_of};
use super::propagate::{land_propagates, propagates, stages_on};
use crate::run::engine::full::{FullWhy, request};
use crate::run::engine::stages::Rebaseline;
use crate::run::engine::{Effect, EventKind, OpKind, OpResult};
use crate::run::model::{FixOf, OpId};

pub(super) const REPO: &str = "/tmp/data/repos/r-1";

pub(super) fn plan_task(toml: &str) -> PlanTask {
    let text = plan_with(&profile(), &[toml.to_string()]);
    crate::run::plan::parse_plan(&text).unwrap().tasks.remove(0)
}

pub(super) fn amend(task: &str, stage: Option<u16>, priority: Option<i32>) -> PlanEdit {
    PlanEdit::AmendTask {
        task_id: task.into(),
        brief: None,
        acceptance: None,
        route: None,
        test_mode: None,
        test_mode_reason: None,
        priority,
        size: None,
        deps: None,
        stage,
        race: None,
        pair: None,
    }
}

/// The queue-width ops pending.
pub(super) fn queue_ops(fx: &Fixture) -> Vec<(OpId, &'static str)> {
    fx.run()
        .pending_ops
        .values()
        .filter_map(|p| match p.kind {
            OpKind::MergeCandidate { .. } => Some((p.op, "MergeCandidate")),
            OpKind::Propagate(_) => Some((p.op, "Propagate")),
            OpKind::CreateStageBranch { .. } => Some((p.op, "CreateStageBranch")),
            _ => None,
        })
        .collect()
}

/// Run-level tier 3 jobs and probes pending.
pub(super) fn full_ops(fx: &Fixture) -> Vec<OpId> {
    fx.run()
        .pending_ops
        .values()
        .filter(|p| {
            p.task_id.is_none() && matches!(&p.kind, OpKind::TestAt(_))
                || matches!(&p.kind, OpKind::Tier(s) if s.tier == 3)
        })
        .map(|p| p.op)
        .collect()
}

/// Every AppendHistory in the log: `(record_id, serialised line)`.
pub(super) fn history_log(effects: &[Effect]) -> Vec<(String, String)> {
    ops_in(effects, "AppendHistory")
        .into_iter()
        .map(|(_, kind)| match kind {
            OpKind::AppendHistory {
                record_id, line, ..
            } => (record_id, serde_json::to_string(&line).unwrap()),
            _ => unreachable!(),
        })
        .collect()
}

/// The invariants every scenario checks after every interesting step.
pub(super) fn invariants(fx: &Fixture, what: &str) {
    let q = queue_ops(fx);
    assert!(q.len() <= 1, "{what}: queue width > 1: {q:?}");
    let f = full_ops(fx);
    assert!(f.len() <= 1, "{what}: two tier-3/probe ops: {f:?}");
    if let Some(op) = f.first() {
        assert_eq!(fx.run().full_op, Some(*op), "{what}: full_op mismatch");
    }
    let bisects = fx
        .run()
        .stages
        .iter()
        .filter(|s| s.bisect.is_some())
        .count();
    assert!(bisects <= 1, "{what}: two bisects");
    // History: no record id with two different contents.
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for (id, line) in history_log(&fx.log) {
        if let Some(prev) = seen.get(&id) {
            assert_eq!(
                prev, &line,
                "{what}: record id {id} reused with other content"
            );
        }
        seen.insert(id, line);
    }
    // Completion never happens while work is undelivered / red / bisecting / due.
    if fx.run().state == RunState::Complete && !fx.run().cancelled && !fx.run().finish_edit {
        let run = fx.run();
        assert!(
            crate::run::engine::propagate::undelivered(run).is_empty(),
            "{what}: complete but undelivered"
        );
        assert!(
            run.stages.iter().all(|s| s.bisect.is_none()),
            "{what}: complete while bisecting"
        );
    }
}

/// Every AppendHistory record id emitted more than once (a note, not a failure).
pub(super) fn repeated_ids(fx: &Fixture) -> Vec<String> {
    let mut count: BTreeMap<String, usize> = BTreeMap::new();
    for (id, _) in history_log(&fx.log) {
        *count.entry(id).or_default() += 1;
    }
    count
        .into_iter()
        .filter(|(_, n)| *n > 1)
        .map(|(k, _)| k)
        .collect()
}

/// Answers every history op.
pub(super) fn flush_history(fx: &mut Fixture) {
    loop {
        let ops: Vec<(OpId, OpKind)> = fx
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
                OpKind::AppendHistory { .. } => OpResult::HistoryAppended,
                _ => OpResult::Failed {
                    message: "not measured".into(),
                },
            };
            fx.done(op, result);
        }
    }
}

/// Starts a red tier 3 on stage `n` through 9.2's `request` (the stage head is red).
pub(super) fn red_tier3(fx: &mut Fixture, n: u16) -> Vec<Effect> {
    let now = fx.now;
    let mut effects = Vec::new();
    assert!(
        request(fx.run_mut(), n, FullWhy::Deliver, now, &mut effects),
        "tier 3 did not start: {:#?}",
        fx.run().log.iter().rev().take(5).collect::<Vec<_>>()
    );
    fx.log.extend(effects);
    let (op, spec) = full_job(fx);
    assert_eq!(spec.stage, n);
    fx.done(op, tier(outcome(3, &[TEST])))
}

pub(super) fn probes_pending(fx: &Fixture) -> Vec<(OpId, String)> {
    pending(fx, "TestAt", None)
        .into_iter()
        .map(|(op, k)| match k {
            OpKind::TestAt(s) => (op, s.commit.clone()),
            _ => unreachable!(),
        })
        .collect()
}

/// t1,t2,t3 in stage 1; t4 in stage 2; t5 in stage 2 depending on t3 (not started).
pub(super) fn five() -> (Fixture, Vec<(String, u32)>) {
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", ""),
        doc_task("t3", ""),
        doc_task("t4", "stage = 2"),
        doc_task("t5", "stage = 2\ndeps = [\"t3\"]"),
    ];
    let (mut fx, windows) = stages_on(&profile(), &tasks);
    fx.run_mut().repo_dir = PathBuf::from(REPO);
    with_orchestrator(&mut fx);
    (fx, windows)
}

/// five(), t1 and t2 merged into stage 1 (propagated), stage 1 red at c2 and bisecting:
/// probe G in flight.
pub(super) fn bisecting_with_stage2() -> (Fixture, Vec<(String, u32)>) {
    let (mut fx, mut windows) = five();
    merge_next(&mut fx, &mut windows, "t1", &commit(1));
    land_propagates(&mut fx);
    merge_next(&mut fx, &mut windows, "t2", &commit(2));
    land_propagates(&mut fx);
    invariants(&fx, "setup");
    red_tier3(&mut fx, 1);
    let b = fx
        .run()
        .stage(1)
        .unwrap()
        .bisect
        .clone()
        .expect("bisecting");
    assert_eq!(b.candidates.len(), 2, "{b:#?}");
    assert_eq!(probes_pending(&fx).len(), 1);
    (fx, windows)
}

pub(super) fn halt_and_rebaseline(fx: &mut Fixture, stages: Vec<(u16, String)>) -> Vec<Effect> {
    {
        let run = fx.run_mut();
        run.state = RunState::Halted;
        run.halted_reason = Some("moved".into());
    }
    let head = stages.last().unwrap().1.clone();
    let reply = fx.reply();
    fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: Some(Rebaseline {
            base: BASE.into(),
            head,
            stages,
            salvaged: None,
        }),
    })
}

pub(super) fn roundtrip(fx: &mut Fixture) {
    let text = serde_json::to_string(fx.run()).unwrap();
    *fx.run_mut() = serde_json::from_str(&text).unwrap();
}

#[test]
fn s1_bisect_with_propagate_and_edits() {
    let (mut fx, mut windows) = bisecting_with_stage2();
    // t3 merges into stage 1 while the probe runs: a propagate 1->2 follows.
    merge_next(&mut fx, &mut windows, "t3", &commit(3));
    invariants(&fx, "after t3");
    assert_eq!(probes_pending(&fx).len(), 1, "the probe still runs");
    let props = propagates(&fx);
    assert_eq!(
        props.len(),
        1,
        "propagate issued beside the probe: {props:#?}"
    );
    // Edits while both are in flight: move not-started t5 to stage 1 with a priority,
    // and add t6 to stage 2.
    let effects = edit(
        &mut fx,
        vec![
            amend("t5", Some(1), Some(7)),
            PlanEdit::AddTask {
                task: plan_task(&doc_task("t6", "stage = 2")),
            },
        ],
    );
    assert_eq!(
        replies(&effects),
        vec![Ok("applied 2 edits".to_string())],
        "{effects:#?}"
    );
    invariants(&fx, "after edit");
    assert_eq!(fx.task("t5").stage(), 1);
    assert_eq!(probes_pending(&fx).len(), 1, "edit kept the probe");
    assert_eq!(propagates(&fx).len(), 1, "edit kept the propagate");
    // The bisect continues to its culprit (red from c2).
    let probed = answer(&mut fx, 2);
    invariants(&fx, "after bisect");
    assert_eq!(probed, [BASE.to_string(), commit(2), commit(1)]);
    assert!(
        matches!(&fx.task("fix1").fixes, Some(FixOf::Bisect { culprit, stage: 1, .. }) if culprit == "t2")
    );
    // The propagate lands.
    land_propagates(&mut fx);
    invariants(&fx, "after propagate");
    assert!(fx.run().stage(2).unwrap().tasks_in.contains("t3"));
    // t5 is in stage 1 and runs there; t6 in stage 2.
    fx.tick();
    windows.extend(fx.launch_all());
    invariants(&fx, "after launches");
    flush_history(&mut fx);
    assert!(repeated_ids(&fx).is_empty(), "{:?}", repeated_ids(&fx));
}

/// Amending a stage-2 task into a brand-new stage 3 while the probe and propagate
/// are in flight: the stage branch waits for the propagate, then is created once.
#[test]
fn s1b_new_stage_during_bisect_and_propagate() {
    let (mut fx, mut windows) = bisecting_with_stage2();
    merge_next(&mut fx, &mut windows, "t3", &commit(3));
    let effects = edit(&mut fx, vec![amend("t5", Some(3), Some(3))]);
    assert_eq!(
        replies(&effects),
        vec![Ok("applied 1 edit".to_string())],
        "{effects:#?}"
    );
    invariants(&fx, "after edit");
    assert!(
        pending(&fx, "CreateStageBranch", None).is_empty(),
        "waits for the propagate"
    );
    land_propagates(&mut fx);
    invariants(&fx, "after landing");
    let creates = pending(&fx, "CreateStageBranch", None);
    assert_eq!(creates.len(), 1, "{:#?}", fx.run().pending_ops);
    let (op, _) = creates[0].clone();
    fx.done(op, OpResult::StageCreated);
    invariants(&fx, "stage 3 created");
    assert!(fx.run().stage(3).is_some());
    answer(&mut fx, 2);
    invariants(&fx, "bisect ended");
    assert!(fx.run().task("fix1").is_some());
    assert_eq!(
        ops_in(&fx.log, "CreateStageBranch").len(),
        1 + 2,
        "stage 1, 2 at start, 3 now"
    );
}

#[test]
fn s2_flaky_tier2_in_stage2_while_stage1_tier3_goes_red() {
    let tasks = [
        doc_task("t1", ""),
        doc_task("t2", ""),
        doc_task("t3", "stage = 2"),
    ];
    let (mut fx, mut windows) = stages_on(&profile(), &tasks);
    fx.run_mut().repo_dir = PathBuf::from(REPO);
    with_orchestrator(&mut fx);
    let run_id = fx.run().id.clone();
    merge_next(&mut fx, &mut windows, "t1", &commit(1));
    land_propagates(&mut fx);
    merge_next(&mut fx, &mut windows, "t2", &commit(2));
    land_propagates(&mut fx);
    // Tier 3 on stage 1 starts.
    let now = fx.now;
    let mut effects = Vec::new();
    assert!(request(
        fx.run_mut(),
        1,
        FullWhy::Deliver,
        now,
        &mut effects
    ));
    fx.log.extend(effects);
    let (full_op, _) = full_job(&fx);
    // t3 (stage 2) claims and its candidate goes out beside the tier-3 job.
    let w3 = window_of(&windows, "t3");
    claim(&mut fx, "t3", w3, &head_of("t3"));
    let (op, _) = pending_one(&fx, "Tier", Some("t3"));
    fx.done(op, tier(outcome(1, &[])));
    let (cand, _) = pending_one(&fx, "MergeCandidate", Some("t3"));
    invariants(&fx, "candidate + tier 3");
    // Tier 3 red: the bisect starts.
    fx.done(full_op, tier(outcome(3, &[TEST])));
    invariants(&fx, "bisect started");
    assert_eq!(probes_pending(&fx).len(), 1);
    // The candidate is green on retry with a flaky test.
    let mut green = outcome(2, &[]);
    green.steps[0].retried = true;
    green.steps[0].flaky = vec!["a::flaky".into()];
    let rounds_before = fx.task("t3").rounds.len();
    fx.done(
        cand,
        OpResult::Merged {
            commit: commit(9),
            tier: Some(Box::new(green)),
        },
    );
    invariants(&fx, "candidate merged");
    assert_eq!(
        fx.task("t3").state,
        TaskState::Merged,
        "flaky costs no bounce"
    );
    assert_eq!(fx.task("t3").rounds.len(), rounds_before);
    for (op, _) in pending(&fx, "RemoveWorktree", Some("t3")) {
        fx.done(op, OpResult::Removed { salvage_ref: None });
    }
    answer(&mut fx, 2);
    invariants(&fx, "bisect done");
    flush_history(&mut fx);
    let all = history_log(&fx.log);
    let ids: Vec<&String> = all.iter().map(|(i, _)| i).collect();
    let tier3 = format!("{run_id}/tier/{full_op}");
    let tier2 = format!("{run_id}/tier/{cand}");
    let flaky = format!("{run_id}/flaky/{cand}/a::flaky");
    let bis = format!("{run_id}/bisect/1/1");
    for want in [&tier3, &tier2, &flaky, &bis] {
        assert_eq!(
            ids.iter().filter(|i| **i == want).count(),
            1,
            "{want} in {ids:#?}"
        );
    }
    assert!(repeated_ids(&fx).is_empty(), "{:?}", repeated_ids(&fx));
}

#[test]
fn s5_pause_holds_propagate_probe_tier3_and_stage_creation() {
    let (mut fx, _windows) = bisecting_with_stage2();
    let effects = edit(&mut fx, vec![PlanEdit::Pause]);
    assert_eq!(fx.run().state, RunState::Paused, "{effects:#?}");
    // The probe's result while paused: recorded, not re-issued.
    let (op, spec) = probe(&fx);
    let effects = fx.done(op, probe_result(false, &spec.commit));
    assert!(ops_in(&effects, "TestAt").is_empty());
    // A head move while paused (stand-in: set_stage_head): no propagate.
    crate::run::engine::stages::set_stage_head(fx.run_mut(), 1, &commit(4));
    let effects = later(&mut fx, 1_000);
    assert!(ops_in(&effects, "Propagate").is_empty(), "{effects:#?}");
    assert!(ops_in(&effects, "Tier").is_empty(), "{effects:#?}");
    assert!(ops_in(&effects, "TestAt").is_empty(), "{effects:#?}");
    // A new stage while paused: no CreateStageBranch.
    let effects = edit(&mut fx, vec![amend("t5", Some(3), None)]);
    assert_eq!(replies(&effects), vec![Ok("applied 1 edit".to_string())]);
    let effects = later(&mut fx, 10);
    assert!(ops_in(&effects, "CreateStageBranch").is_empty());
    // Resume: exactly one of probe/propagate each (one queue op at a time).
    let effects = edit(&mut fx, vec![PlanEdit::Resume]);
    invariants(&fx, "resumed");
    assert_eq!(ops_in(&effects, "TestAt").len(), 1, "{effects:#?}");
    assert_eq!(ops_in(&effects, "Propagate").len(), 1, "{effects:#?}");
    assert!(
        ops_in(&effects, "CreateStageBranch").is_empty(),
        "waits beside the propagate"
    );
}
