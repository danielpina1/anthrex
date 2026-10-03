//! Milestone 9.3 task 4a: a round's work goes only in new stages (decisions 13 and 15):
//! earlier rounds are read-only, a dependency on an earlier round is met, a `Single`
//! run is widened for its first later round, and the layout is fixed for round 1 only.

use std::collections::BTreeSet;

use proto::{PrState, RunState};
use serde_json::json;

use super::delivery_land::pr_record;
use super::fixes::spec as fixes_spec;
use super::fixture::*;
use super::goal_rounds_start::{complete, iterate, reply, started};
use super::kinds_integration::{C1, merge_real};
use super::merge::pending;
use super::orch::{add, answer, edit_plan, launched};
use crate::run::delivery::StageDelivery;
use crate::run::engine::fixes::add_fix;
use crate::run::engine::{OpKind, OpResult, schedule, stages};
use crate::run::model::{StageLayout, StageRecord};

/// KG §2.4's refusal of stage `s` of round `r`.
fn earlier(s: u16, r: u32) -> String {
    format!("stage {s} belongs to round {r}, which is done; put new work in a new stage")
}

/// An `add_task` of `id` in `stage`, depending on `deps`.
pub(super) fn add_in(id: &str, module: &str, stage: u16, deps: &[&str]) -> serde_json::Value {
    let mut edit = add(id, module);
    edit["task"]["stage"] = json!(stage);
    edit["task"]["deps"] = json!(deps);
    edit
}

/// The messages of a rejected `edit_plan`.
fn rejected(effects: &[crate::run::engine::Effect]) -> Vec<String> {
    let (ok, value) = answer(effects);
    assert!(!ok, "{value}");
    value["errors"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["message"].as_str().unwrap().to_string())
        .collect()
}

/// [`complete`], iterated into round 2 by the user.
fn round_two() -> Fixture {
    let mut fx = complete();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    fx
}

/// The `CreateStageBranch` ops in flight: (op, branch, from).
pub(super) fn creating(fx: &Fixture) -> Vec<(u64, String, String)> {
    pending(fx, "CreateStageBranch", None)
        .into_iter()
        .map(|(op, kind)| match kind {
            OpKind::CreateStageBranch { branch, from, .. } => (op, branch, from),
            _ => unreachable!(),
        })
        .collect()
}

/// `anthrex/<run>/stage-<n>`.
fn stage_branch(n: u16) -> String {
    format!("anthrex/{RUN_ID}/stage-{n}")
}

/// Round 2's `edits` submitted and approved by the user.
pub(super) fn plan_round(fx: &mut Fixture, edits: serde_json::Value) {
    let effects = edit_plan(fx, json!({"edits": edits, "submit": true}));
    assert!(answer(&effects).0, "{effects:#?}");
    assert_eq!(fx.run().state, RunState::AwaitingApproval);
    fx.approve();
    assert_eq!(fx.run().state, RunState::Running);
}

/// Decision 15: new tasks only in the round's stages; earlier rounds' tasks read-only.
#[test]
fn new_tasks_only_in_new_stages() {
    let mut fx = round_two();
    let effects = edit_plan(&mut fx, json!({"edits": [add("t2", "mail")]}));
    assert_eq!(rejected(&effects), vec![earlier(1, 1)]);
    assert!(fx.run().task("t2").is_none());
    let effects = edit_plan(&mut fx, json!({"edits": [add_in("t3", "mail", 2, &[])]}));
    assert!(answer(&effects).0, "{effects:#?}");
    assert_eq!(fx.task("t3").round, 2);
    assert_eq!(fx.task("t1").round, 1);
    let amend = json!({"op": "amend_task", "task_id": "t1", "brief": "again"});
    assert_eq!(
        rejected(&edit_plan(&mut fx, json!({"edits": [amend]}))),
        vec![earlier(1, 1)]
    );
    let split = json!({"op": "split_task", "task_id": "t1", "into": [add("t4", "x")["task"]]});
    assert_eq!(
        rejected(&edit_plan(&mut fx, json!({"edits": [split]}))),
        vec![earlier(1, 1)]
    );
    // A dependency onto an earlier round's task is a new task's to declare, not an
    // edit of the earlier task.
    let dep = json!({"op": "add_dep", "task_id": "t3", "dep": "t1"});
    assert!(answer(&edit_plan(&mut fx, json!({"edits": [dep]}))).0);
    // Fix round 1, I3: nor may this round's task be moved into an earlier stage.
    let amend = json!({"op": "amend_task", "task_id": "t3", "stage": 1});
    assert_eq!(
        rejected(&edit_plan(&mut fx, json!({"edits": [amend]}))),
        vec![earlier(1, 1)]
    );
    assert_eq!(fx.task("t3").stage(), 2);
    // Moving it within the round's stages is fine.
    let amend = json!({"op": "amend_task", "task_id": "t3", "stage": 3});
    let edits = json!([add_in("t5", "web", 2, &[]), amend]);
    let effects = edit_plan(&mut fx, json!({ "edits": edits }));
    assert!(answer(&effects).0, "{effects:#?}");
    assert_eq!(fx.task("t3").stage(), 3);
}

/// Fix round 1, M2: only an earlier round's cancelled task is a met dependency; one
/// cancelled in the same round is still refused (`validate_graph`'s rule).
#[test]
fn a_cancelled_dependency_of_the_same_round_is_still_refused() {
    let mut fx = round_two();
    let effects = edit_plan(&mut fx, json!({"edits": [add_in("t3", "mail", 2, &[])]}));
    assert!(answer(&effects).0, "{effects:#?}");
    let cancel = json!({"op": "cancel_task", "task_id": "t3"});
    assert!(answer(&edit_plan(&mut fx, json!({"edits": [cancel]}))).0);
    let effects = edit_plan(&mut fx, json!({"edits": [add_in("t4", "cli", 2, &["t3"])]}));
    assert_eq!(rejected(&effects), vec!["t3 is cancelled".to_string()]);
    assert!(fx.run().task("t4").is_none());
}

/// Decision 13: a dependency on an earlier round's task is met, merged or cancelled.
#[test]
fn a_new_task_may_depend_on_an_earlier_round() {
    let mut fx = launched(false);
    let edits = json!([add("t1", "auth"), add("t2", "mail")]);
    assert!(answer(&edit_plan(&mut fx, json!({"edits": edits}))).0);
    let cancel = json!({"op": "cancel_task", "task_id": "t2"});
    let effects = edit_plan(&mut fx, json!({"edits": [cancel], "submit": true}));
    assert!(answer(&effects).0, "{effects:#?}");
    fx.approve();
    merge_real(&mut fx, "t1", C1);
    let (op, _) = fx.op("VerifyRefs");
    fx.done(op, OpResult::RefsOk);
    assert_eq!(fx.run().state, RunState::Complete);
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(
        &mut fx,
        json!([
            add_in("t3", "cli", 2, &["t1"]),
            add_in("t4", "docs", 2, &["t2"])
        ]),
    );
    // Stage 1 from the run head, then stage 2 from it.
    for n in [1, 2] {
        let ops = creating(&fx);
        assert_eq!(ops.len(), 1, "stage {n}: {ops:?}");
        fx.done(ops[0].0, OpResult::StageCreated);
    }
    let run = fx.run();
    for id in ["t3", "t4"] {
        let i = run.tasks.iter().position(|t| t.id() == id).unwrap();
        assert!(schedule::deps_done(run, &run.tasks[i]), "{id}");
        assert!(stages::ready_in_stage(run, i), "{id}");
        assert_ne!(run.tasks[i].state, proto::TaskState::Pending, "{id}");
        assert_ne!(run.tasks[i].state, proto::TaskState::Blocked, "{id}");
    }
}

/// Decision 13's widening: a `Single` run becomes `Multi` with no stage recorded; the
/// next running pass creates stage 1 from the run head, then the round's stage from it.
/// The delivery records stay.
#[test]
fn a_single_stage_run_widens_for_round_two() {
    let mut fx = complete();
    assert_eq!(fx.run().stage_layout, StageLayout::Single);
    fx.run_mut().delivery.stages = vec![StageDelivery {
        pr: Some(pr_record(7, PrState::Merged)),
        ..StageDelivery::default()
    }];
    let delivery = fx.run().delivery.clone();
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    let run = fx.run();
    assert_eq!(run.stage_layout, StageLayout::Multi);
    assert!(run.stages.is_empty(), "{:?}", run.stages);
    assert_eq!(run.delivery, delivery);
    let head = run.run_head.clone();
    assert_eq!(head, C1);
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    let ops = creating(&fx);
    assert_eq!(
        ops.iter()
            .map(|(_, b, f)| (b.clone(), f.clone()))
            .collect::<Vec<_>>(),
        vec![(stage_branch(1), head.clone())]
    );
    fx.done(ops[0].0, OpResult::StageCreated);
    let ops = creating(&fx);
    assert_eq!(
        ops.iter()
            .map(|(_, b, f)| (b.clone(), f.clone()))
            .collect::<Vec<_>>(),
        vec![(stage_branch(2), head.clone())]
    );
    fx.done(ops[0].0, OpResult::StageCreated);
    let rounds: Vec<(u16, u32)> = fx.run().stages.iter().map(|s| (s.n, s.round)).collect();
    assert_eq!(rounds, vec![(1, 1), (2, 2)], "each stage carries its round");
    assert_eq!(fx.run().delivery, delivery);
}

/// Decision 13: `fix_layout` runs when round 1's plan is approved, never for a later
/// round, whose stages it would clear.
#[test]
fn fix_layout_is_not_run_again_for_a_round() {
    let mut fx = complete();
    // Round 1 approved with stages (set in place: a `Multi` round 1 completing needs
    // its stages' merges and propagates).
    let run = fx.run_mut();
    run.stage_layout = StageLayout::Multi;
    let tasks_in = BTreeSet::from(["t1".to_string()]);
    let record = StageRecord::new(1, stage_branch(1), C1, tasks_in, 1_500);
    run.stages = vec![record.clone()];
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    assert_eq!(
        fx.run().stage_layout,
        StageLayout::Multi,
        "not widened again"
    );
    let logged = fx.run().log.len();
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    assert_eq!(fx.run().stages.first(), Some(&record), "stage 1 kept");
    let approvals: Vec<_> = fx.run().log[logged..]
        .iter()
        .filter(|l| l.text.starts_with("approved with"))
        .collect();
    assert!(approvals.is_empty(), "{approvals:?}");
    let ops = creating(&fx);
    assert_eq!(ops.len(), 1, "{ops:?}");
    assert_eq!(
        (ops[0].1.clone(), ops[0].2.clone()),
        (stage_branch(2), C1.to_string())
    );
}

/// Decision 13: a fix task the engine adds takes the round of the stage it fixes.
#[test]
fn engine_made_fix_tasks_take_their_stages_round() {
    let mut fx = round_two();
    let (now, route) = (fx.now, proto::RouteSpec::default());
    for (stage, round) in [(1, 1), (2, 2)] {
        let mut spec = fixes_spec(proto::TaskOrigin::Bisect, "crates/fix/**", route.clone());
        spec.stage = stage;
        let id = add_fix(fx.run_mut(), spec, now, &mut Vec::new()).expect("added");
        assert_eq!(fx.task(&id).round, round, "stage {stage}");
    }
}
