//! M9.9 review fixes, M1 to M3: `run override` is refused for research and review
//! tasks; decision 25 restarts a rewritten task at most `MAX_REWRITE_RESTARTS` times;
//! an integration review is made only under a valid id, and its prompt numbers the
//! round from that id.

use proto::{BlockReason, PlanEdit, TaskState};

use super::dispatch::{edit, replies};
use super::fixture::*;
use super::kinds::{B1, H1, research, review, reviewer_window, running, window_task};
use super::kinds_integration::{C1, mail_epic, merge_real};
use super::orch::{add, edit_plan, launched};
use super::planners::epic;
use crate::run::engine::{Effect, EventKind, OpKind};
use crate::run::orch::contract::integration_review_prompt;
use crate::run::snapshot::snapshot;
use serde_json::json;

fn override_task(fx: &mut Fixture, id: &str) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Override {
        reply,
        run_id: RUN_ID.into(),
        task_id: id.into(),
        reason: "ship it".into(),
    })
}

const KINDS: &str = "override applies only to code and docs tasks";

#[test]
fn override_is_refused_for_research_and_review_tasks() {
    let mut fx = running("", &[research("r1", ""), review("v1", "S")]);
    fx.tick();
    assert_eq!(fx.task("v1").state, TaskState::Review);
    for id in ["r1", "v1"] {
        let effects = override_task(&mut fx, id);
        assert_eq!(replies(&effects), vec![Err(KINDS.to_string())], "{id}");
    }
    assert_eq!(fx.task("v1").state, TaskState::Review);
    assert!(fx.run().merge_queue.is_empty());
    // An integration review, in review, too.
    let mut fx = mail_epic(&["m1"]);
    merge_real(&mut fx, "m1", C1);
    assert_eq!(fx.task("mail-int1").state, TaskState::Review);
    let effects = override_task(&mut fx, "mail-int1");
    assert_eq!(replies(&effects), vec![Err(KINDS.to_string())]);
}

pub(super) fn rewrite(fx: &mut Fixture, brief: &str) -> Vec<Effect> {
    edit(
        fx,
        vec![PlanEdit::AmendTask {
            task_id: "t1".into(),
            brief: Some(brief.into()),
            acceptance: None,
            route: None,
            test_mode: None,
            test_mode_reason: None,
            priority: None,
            size: None,
            deps: None,
            stage: None,
            race: None,
            pair: None,
        }],
    )
}

/// `t1`, restarted, is blocked `mis_sized` again (rung 3 raised it once more).
pub(super) fn mis_sized_again(fx: &mut Fixture) {
    fx.run_mut()
        .pending_ops
        .retain(|_, p| p.task_id.as_deref() != Some("t1"));
    let task = fx.task_mut("t1");
    task.state = TaskState::Blocked;
    task.fresh_session = None;
    task.block = Some(proto::BlockInfo {
        reason: BlockReason::MisSized,
        text: "does not fit".into(),
    });
    for round in &mut task.rounds {
        round.ended = true;
        round.turn_open = false;
    }
}

/// A running `--yes` planned run whose orchestrator added `t1`, started and then
/// blocked `mis_sized`.
fn orchestrated_mis_sized() -> Fixture {
    let mut fx = launched(true);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    fx.launch_all();
    mis_sized_again(&mut fx);
    fx
}

/// The orchestrator's rewrite of `t1`'s brief.
fn orch_rewrite(fx: &mut Fixture, brief: &str) -> Vec<Effect> {
    let amend = json!({"op": "amend_task", "task_id": "t1", "brief": brief});
    edit_plan(fx, json!({"edits": [amend]}))
}

#[test]
fn a_task_is_restarted_by_rewrites_at_most_three_times() {
    let mut fx = orchestrated_mis_sized();
    for n in 1..=3 {
        orch_rewrite(&mut fx, &format!("step {n}"));
        let task = fx.task("t1");
        assert_eq!(task.state, TaskState::Working, "rewrite {n}");
        assert_eq!(task.orch.rewrite_restarts, n);
        mis_sized_again(&mut fx);
    }
    let effects = orch_rewrite(&mut fx, "step 4");
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    let task = fx.task("t1");
    // The rewrite stands; the task waits for the user.
    assert_eq!(task.spec.brief, "step 4");
    assert_eq!(task.state, TaskState::Blocked);
    let block = task.block.clone().unwrap();
    assert_eq!(block.reason, BlockReason::Environment);
    assert_eq!(block.text, "rewritten 3 times; the user decides");
    assert_eq!(task.orch.rewrite_restarts, 3);
    assert!(ops_in(&effects, "DiffSoFar").is_empty());
}

/// Second review, M-c: the cap counts the orchestrator's rewrites only. The user's
/// own rewrite always restarts, and the user's `run retry` starts the count again.
#[test]
fn the_users_rewrite_and_retry_are_not_capped() {
    let mut fx = orchestrated_mis_sized();
    for n in 1..=3 {
        orch_rewrite(&mut fx, &format!("step {n}"));
        mis_sized_again(&mut fx);
    }
    // The user's rewrite restarts it, and is not counted.
    rewrite(&mut fx, "the user's step");
    let task = fx.task("t1");
    assert_eq!(task.state, TaskState::Working);
    assert_eq!(task.orch.rewrite_restarts, 3);
    mis_sized_again(&mut fx);
    // The user's retry starts the count again: the orchestrator's next rewrite restarts.
    let reply = fx.reply();
    let effects = fx.next(EventKind::Retry {
        reply,
        run_id: RUN_ID.into(),
        task_id: "t1".into(),
    });
    assert!(matches!(&replies(&effects)[..], [Ok(_)]), "{effects:#?}");
    assert_eq!(fx.task("t1").orch.rewrite_restarts, 0);
    mis_sized_again(&mut fx);
    orch_rewrite(&mut fx, "step 5");
    let task = fx.task("t1");
    assert_eq!(task.state, TaskState::Working);
    assert_eq!(task.orch.rewrite_restarts, 1);
}

#[test]
fn no_integration_review_is_made_without_a_valid_id() {
    let mut fx = mail_epic(&["m1"]);
    // An epic whose name leaves no room for `-int<n>` in 16 characters.
    let long = "notifications";
    for record in &mut fx.run_mut().orch.epics {
        if record.epic == "mail" {
            record.epic = long.into();
        }
    }
    fx.task_mut("m1").spec.epic = Some(long.into());
    merge_real(&mut fx, "m1", C1);
    fx.tick();
    assert!(
        !fx.run()
            .tasks
            .iter()
            .any(|t| t.orch.integration_of.is_some()),
        "{:#?}",
        fx.run().tasks.iter().map(|t| t.id()).collect::<Vec<_>>()
    );
    let attention = snapshot(&fx.state, fx.now).runs[0].attention.clone();
    assert!(
        attention.contains(&format!("epic {long}: no free integration review id")),
        "{attention:#?}"
    );
    // The control: a short name gets its review, and no such line.
    let mut fx = mail_epic(&["m1"]);
    merge_real(&mut fx, "m1", C1);
    assert!(fx.run().task("mail-int1").is_some());
    let attention = snapshot(&fx.state, fx.now).runs[0].attention.clone();
    assert!(
        !attention
            .iter()
            .any(|l| l.contains("no free integration review id"))
    );
}

#[test]
fn the_integration_prompt_numbers_the_round_from_its_id() {
    let mut fx = mail_epic(&["m1"]);
    let user = crate::run::test_support::task_toml("mail-int1", "S", "[\"crates/zz/**\"]", "");
    let spec = crate::run::plan::parse_plan(&plan_with(PROFILE, &[user])).unwrap();
    edit(
        &mut fx,
        vec![PlanEdit::AddTask {
            task: spec.tasks[0].clone(),
        }],
    );
    merge_real(&mut fx, "m1", C1);
    let task = fx.task("mail-int2").clone();
    assert_eq!(task.spec.title, "integration review of epic mail, round 2");
    let window = reviewer_window(&mut fx, "mail-int2", "+x\n");
    let first_turn = fx
        .ops("CreateWindow")
        .into_iter()
        .filter(|(_, k)| window_task(k) == "mail-int2")
        .find_map(|(_, k)| match k {
            OpKind::CreateWindow { first_turn, .. } => Some(first_turn),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no reviewer for mail-int2 ({window})"));
    let record = epic(&fx, "mail").clone();
    assert_eq!(
        first_turn,
        integration_review_prompt(fx.run(), &record, 2, B1, H1)
    );
}
