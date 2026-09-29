//! Milestone 9 task M9.9: the per-epic integration review (decision 37), an engine-made
//! review task `<e>-int<n>` run through decision 36's dispatch, and what it holds
//! (decision 38).

use proto::{AgentRole, IntegrationState, RunState, TaskKind, TaskState};
use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use super::kinds::{B1, H1, approve, changes, reviewer_window, running, verdict};
use super::merge::pending_one;
use super::orch::{add, edit_plan, error};
use super::planners::{epic, planning_mail, submit_epic, task_in};
use crate::run::engine::{Effect, OpKind, OpResult};
use crate::run::model::ReviewLevel;
use crate::run::orch::contract::integration_review_prompt;
use crate::run::roster::pick_reviewer;

pub(super) const C1: &str = "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1";
pub(super) const C2: &str = "c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2c2";
pub(super) const C3: &str = "c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3c3";

/// Task `id` goes through the real merge queue and merges at `commit`; its own ops
/// in flight are dropped first, and its worktrees' removals answered after.
pub(super) fn merge_real(fx: &mut Fixture, id: &str, commit: &str) -> Vec<Effect> {
    fx.run_mut()
        .pending_ops
        .retain(|_, p| p.task_id.as_deref() != Some(id));
    let task = fx.task_mut(id);
    task.state = TaskState::MergeQueue;
    task.head = Some(HEAD.into());
    for round in &mut task.rounds {
        round.ended = true;
        round.turn_open = false;
    }
    fx.run_mut().merge_queue.push(id.into());
    fx.tick();
    let (op, _) = pending_one(fx, "MergeCandidate", Some(id));
    let mut effects = fx.done(
        op,
        OpResult::Merged {
            commit: commit.into(),
        },
    );
    let removals: Vec<_> = fx
        .run()
        .pending_ops
        .values()
        .filter(|p| p.task_id.as_deref() == Some(id))
        .filter(|p| matches!(p.kind, OpKind::RemoveWorktree { .. }))
        .map(|p| p.op)
        .collect();
    for op in removals {
        effects.extend(fx.done(op, OpResult::Removed { salvage_ref: None }));
    }
    effects
}

/// A running `--yes` run whose orchestrator added `t1` and whose sub-planner of `mail`
/// added `ids` (each owning `crates/mail/**`), approved at once.
pub(super) fn mail_epic(ids: &[&str]) -> Fixture {
    let mut fx = planning_mail(true);
    let edits: Vec<_> = ids.iter().map(|id| task_in(id, "mail")).collect();
    let effects = submit_epic(&mut fx, json!(edits));
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    fx
}

fn int_task(fx: &Fixture, id: &str) -> Option<crate::run::model::Task> {
    fx.run().task(id).cloned()
}

fn verify_refs(effects: &[Effect]) -> usize {
    ops_in(effects, "VerifyRefs").len()
}

/// Every merge done, the review `mail-int1` then runs to `verdict_args`.
pub(super) fn reviewed(verdict_args: serde_json::Value) -> Fixture {
    let mut fx = mail_epic(&["m1"]);
    merge_real(&mut fx, "t1", C1);
    merge_real(&mut fx, "m1", C2);
    let window = reviewer_window(&mut fx, "mail-int1", "+x\n");
    let effects = verdict(&mut fx, window, "mail-int1", verdict_args);
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    fx
}

#[test]
fn integration_review_task_is_made_when_the_epic_is_merged() {
    let mut fx = mail_epic(&["m1", "m2"]);
    merge_real(&mut fx, "m1", C1);
    // One task of the epic is still to merge.
    assert!(int_task(&fx, "mail-int1").is_none());
    assert_eq!(epic(&fx, "mail").base.as_deref(), Some(BASE));
    assert_eq!(
        epic(&fx, "mail").merges,
        vec![("m1".to_string(), C1.to_string())]
    );
    merge_real(&mut fx, "m2", C2);
    let task = int_task(&fx, "mail-int1").expect("the engine made the review");
    assert_eq!(task.spec.kind, TaskKind::Review);
    assert_eq!(task.spec.title, "integration review of epic mail, round 1");
    assert_eq!(task.spec.epic.as_deref(), Some("mail"));
    let target = format!("{BASE}..{C2}");
    assert_eq!(task.spec.review_target.as_deref(), Some(target.as_str()));
    assert_eq!(task.size, proto::Size::M);
    assert!(task.spec.owns.is_empty());
    assert_eq!(task.orch.integration_of.as_deref(), Some("mail"));
    assert_eq!(task.review_level, Some(ReviewLevel::Frontier));
    assert_eq!(task.orch.gate_hold, None);
    let record = epic(&fx, "mail");
    assert_eq!(record.integration_state, IntegrationState::Reviewing);
    assert_eq!(record.integration_rounds, 1);
    assert_eq!(record.merges.len(), 2);
    // It runs through decision 36's dispatch: its target is resolved first.
    let (_, kind) = pending_one(&fx, "ResolveTarget", Some("mail-int1"));
    let OpKind::ResolveTarget { target: t, .. } = kind else {
        unreachable!()
    };
    assert_eq!(t, target);
    assert_eq!(fx.task("mail-int1").state, TaskState::Review);
    // Nothing more is made while it runs.
    fx.tick();
    assert!(int_task(&fx, "mail-int2").is_none());
}

#[test]
fn integration_review_takes_the_first_free_round_id() {
    let mut fx = mail_epic(&["m1"]);
    // The user's own `run edit` may use the id (M8a's rules only).
    let user = crate::run::test_support::task_toml("mail-int1", "S", "[\"crates/zz/**\"]", "");
    let spec = crate::run::plan::parse_plan(&plan_with(PROFILE, &[user])).unwrap();
    let effects = super::dispatch::edit(
        &mut fx,
        vec![proto::PlanEdit::AddTask {
            task: spec.tasks[0].clone(),
        }],
    );
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    merge_real(&mut fx, "m1", C1);
    let task = int_task(&fx, "mail-int2").expect("the next free id");
    assert_eq!(task.orch.integration_of.as_deref(), Some("mail"));
    assert_eq!(task.spec.kind, TaskKind::Review);
    // The user's task is untouched.
    let theirs = fx.task("mail-int1");
    assert_eq!(theirs.spec.kind, TaskKind::Code);
    assert_eq!(theirs.orch.integration_of, None);
    assert_eq!(epic(&fx, "mail").integration_rounds, 1);
}

#[test]
fn integration_changes_holds_completion() {
    let mut fx = reviewed(changes());
    assert_eq!(fx.task("mail-int1").state, TaskState::Reported);
    assert_eq!(
        epic(&fx, "mail").integration_state,
        IntegrationState::Changes
    );
    let effects = fx.tick();
    assert_eq!(verify_refs(&fx.log), 0, "{effects:#?}");
    assert_eq!(fx.run().state, RunState::Running);
    // The control: an approving verdict lets the run complete.
    let fx = reviewed(approve());
    assert_eq!(
        epic(&fx, "mail").integration_state,
        IntegrationState::Approved
    );
    assert_eq!(verify_refs(&fx.log), 1);
}

#[test]
fn fix_task_merge_makes_round_two() {
    let mut fx = reviewed(changes());
    // The orchestrator's fix task in the epic (past the cap, the review having
    // reported: M9.4's exemption).
    let mut fix = add("m9", "mail");
    fix["task"]["epic"] = json!("mail");
    let effects = edit_plan(&mut fx, json!({"edits": [fix]}));
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    assert!(int_task(&fx, "mail-int2").is_none());
    merge_real(&mut fx, "m9", C3);
    let task = int_task(&fx, "mail-int2").expect("round two");
    // The epic's base is the run head before its first merge (t1 merged at C1).
    let target = format!("{C1}..{C3}");
    assert_eq!(task.spec.review_target.as_deref(), Some(target.as_str()));
    assert_eq!(task.spec.title, "integration review of epic mail, round 2");
    let record = epic(&fx, "mail");
    assert_eq!(record.integration_rounds, 2);
    assert_eq!(record.integration_state, IntegrationState::Reviewing);
    // Round two's prompt lists round one's blocking findings.
    let window = reviewer_window(&mut fx, "mail-int2", "+y\n");
    let _ = window;
    let first = ops_in(&fx.log, "CreateWindow")
        .into_iter()
        .rev()
        .find(|(_, k)| op_task(k) == "mail-int2")
        .map(|(_, k)| k)
        .unwrap();
    let OpKind::CreateWindow { first_turn, .. } = first else {
        unreachable!()
    };
    let record = epic(&fx, "mail").clone();
    assert_eq!(
        first_turn,
        integration_review_prompt(fx.run(), &record, 2, B1, H1)
    );
    assert!(first_turn.contains("drops the error"), "{first_turn}");
}

#[test]
fn finish_closes_changes() {
    let mut fx = reviewed(changes());
    assert_eq!(verify_refs(&fx.log), 0);
    let effects = edit_plan(&mut fx, json!({"edits": [{"op": "finish"}]}));
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    assert_eq!(
        epic(&fx, "mail").integration_state,
        IntegrationState::Finished
    );
    assert_eq!(verify_refs(&fx.log), 1);
}

#[test]
fn no_round_after_max_bounces_plus_one() {
    let mut fx = mail_epic(&["m1"]);
    fx.run_mut().limits.max_bounces = 1;
    merge_real(&mut fx, "t1", C1);
    merge_real(&mut fx, "m1", C2);
    let window = reviewer_window(&mut fx, "mail-int1", "+x\n");
    verdict(&mut fx, window, "mail-int1", changes());
    for (n, (id, commit)) in [("m8", C3), ("m9", HEAD)].into_iter().enumerate() {
        let mut fix = add(id, "mail");
        fix["task"]["epic"] = json!("mail");
        edit_plan(&mut fx, json!({"edits": [fix]}));
        merge_real(&mut fx, id, commit);
        if n == 0 {
            // Round two (max_bounces + 1 = 2 rounds in all).
            let window = reviewer_window(&mut fx, "mail-int2", "+y\n");
            verdict(&mut fx, window, "mail-int2", changes());
        }
    }
    assert!(int_task(&fx, "mail-int3").is_none());
    let record = epic(&fx, "mail");
    assert_eq!(record.integration_rounds, 2);
    assert_eq!(record.integration_state, IntegrationState::Changes);
    fx.tick();
    assert_eq!(verify_refs(&fx.log), 0);
}

#[test]
fn plan_path_has_no_integration_review() {
    // A plan file's `epic` names no epic record: no integration review.
    let mut fx = running("", &[task("t1", "S", "auth", "epic = \"auth\"")]);
    merge_real(&mut fx, "t1", C1);
    assert!(
        fx.run()
            .tasks
            .iter()
            .all(|t| t.orch.integration_of.is_none())
    );
    assert_eq!(fx.run().tasks.len(), 1);
    assert_eq!(verify_refs(&fx.log), 1);
}

#[test]
fn integration_reviewer_route_is_the_peer_at_frontier() {
    let mut fx = mail_epic(&["m1"]);
    // The built-in roster has no frontier Codex model; this one has.
    let peer = if fx.task("m1").route.runtime == proto::Runtime::Claude {
        proto::Runtime::Codex
    } else {
        proto::Runtime::Claude
    };
    fx.run_mut().roster.push(proto::ModelEntry {
        runtime: peer,
        model: "peer-frontier".into(),
        strength: proto::Strength::Frontier,
        note: String::new(),
    });
    merge_real(&mut fx, "m1", C1);
    let author = fx.task("m1").route.clone();
    let task = fx.task("mail-int1").clone();
    let expected = pick_reviewer(&fx.run().roster, &author, ReviewLevel::Frontier);
    assert_eq!(task.route, expected);
    assert_eq!(task.route.runtime, peer, "the peer runtime");
    assert_eq!(task.route.model, "peer-frontier");
    assert_eq!(task.route.strength, proto::Strength::Frontier);
    // Its session runs on that route, with the integration prompt.
    let window = reviewer_window(&mut fx, "mail-int1", "+x\n");
    assert!(window > 0);
    let task = fx.task("mail-int1");
    assert_eq!(task.reviews.last().unwrap().route, expected);
    let (_, kind) = ops_in(&fx.log, "CreateWindow")
        .into_iter()
        .rev()
        .find(|(_, k)| op_task(k) == "mail-int1")
        .unwrap();
    let OpKind::CreateWindow {
        first_turn, spec, ..
    } = kind
    else {
        unreachable!()
    };
    let record = epic(&fx, "mail").clone();
    assert_eq!(
        first_turn,
        integration_review_prompt(fx.run(), &record, 1, B1, H1)
    );
    assert_eq!(spec.mcp.unwrap().role, AgentRole::Reviewer);
    assert_eq!(spec.runtime, expected.runtime);
}

#[test]
fn integration_review_tasks_refuse_orchestrator_edits() {
    let mut fx = mail_epic(&["m1"]);
    merge_real(&mut fx, "m1", C1);
    let before = fx.run().clone();
    let split =
        json!({"op": "split_task", "task_id": "mail-int1", "into": [add("x1", "mail")["task"]]});
    for edit in [
        json!({"op": "amend_task", "task_id": "mail-int1", "brief": "look at less"}),
        json!({"op": "cancel_task", "task_id": "mail-int1"}),
        split,
    ] {
        let text = error(&edit_plan(&mut fx, json!({"edits": [edit]})));
        assert_eq!(
            text,
            "task mail-int1 is an integration review; the engine owns it"
        );
        assert_eq!(fx.run().tasks, before.tasks);
    }
}
