//! Milestone 9 task M9.8, the binding rule: on a promoted or gated run, nothing a
//! sub-planner submits runs until the user approves a hold round that holds it
//! (decision 28; M9.7 second review, items 8–10), a re-plan's included. Also the
//! orchestrator's `submit` of the epic rounds its own additions opened, and the scout
//! extract a worker's first turn carries (decision 34).

use proto::HoldState;
use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use super::gate_holds::held;
use super::orch::{ORCH, answer, edit_plan, orch_tool};
use super::planners::{
    PLANNER, epic, hold_state, planner_ended, planner_started, planner_tool, planning_mail, spawn,
    spawn_args, submit_epic, task_in,
};
use super::promote::{hold_verdict, prepared, promoted};
use crate::run::engine::planners::EPIC_RECORDED;
use crate::run::engine::{Effect, OpKind, ScoutEnd};
use crate::run::orch::PlannerPhase;

/// The one reply: whether it succeeded, and its text.
pub(super) fn reply(effects: &[Effect]) -> (bool, String) {
    let replies = replies(effects);
    assert_eq!(replies.len(), 1, "{replies:?}");
    match &replies[0] {
        Ok(text) => (true, text.clone()),
        Err(text) => (false, text.clone()),
    }
}

/// A gated run: the planner's task waits under `epic:mail`, which its accepted epic
/// submits to the user; it starts only on the user's approval.
#[test]
fn a_planners_tasks_are_held_under_its_epic_round_until_the_user_approves() {
    let mut fx = planning_mail(false);
    let effects = submit_epic(&mut fx, json!([task_in("t2", "mail")]));
    assert_eq!(replies(&effects), vec![Ok(EPIC_RECORDED.to_string())]);
    assert_eq!(fx.task("t2").orch.gate_hold.as_deref(), Some("epic:mail"));
    assert_eq!(hold_state(&fx, "epic:mail"), Some(HoldState::Awaiting));
    fx.tick();
    assert!(!prepared(&fx, "t2"), "held until the user approves");
    hold_verdict(&mut fx, "epic:mail", true);
    assert!(prepared(&fx, "t2"), "dispatched on the approval");
}

/// With `--yes` the round is approved as the epic is accepted.
#[test]
fn with_yes_a_planners_round_is_approved_on_accept() {
    let mut fx = planning_mail(true);
    submit_epic(&mut fx, json!([task_in("t2", "mail")]));
    assert_eq!(hold_state(&fx, "epic:mail"), Some(HoldState::Approved));
    assert!(prepared(&fx, "t2"));
}

/// A promoted run: the planner's work waits for its own epic's round, not the
/// promotion's; approving the promotion releases none of it. A re-plan after that
/// approval opens a new round, and its tasks wait again.
#[test]
fn a_promoted_runs_planner_and_its_replan_wait_for_their_rounds() {
    let mut fx = promoted();
    assert_eq!(spawn(&mut fx, "web")["hold"], "epic:web");
    planner_started(&mut fx, PLANNER);
    let args = json!({"edits": [task_in("t5", "web/a")]});
    let effects = planner_tool(&mut fx, (PLANNER, "web"), "submit_epic", args);
    assert_eq!(replies(&effects), vec![Ok(EPIC_RECORDED.to_string())]);
    assert_eq!(fx.task("t5").orch.gate_hold.as_deref(), Some("epic:web"));
    edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    hold_verdict(&mut fx, "promotion", true);
    fx.tick();
    assert!(!prepared(&fx, "t5"), "the promotion does not release web");
    hold_verdict(&mut fx, "epic:web", true);
    assert!(prepared(&fx, "t5"));
    // The re-plan.
    planner_ended(&mut fx, ("web", 1), ScoutEnd::Reported);
    let args = spawn_args("web", &["crates/web/**"], "More web");
    let (ok, value) = answer(&orch_tool(&mut fx, ORCH, "spawn_subplanner", args));
    assert!(ok, "{value}");
    assert_eq!(value["hold"], "epic:web.2");
    planner_started(&mut fx, PLANNER + 1);
    // Its own module, so it does not wait for t5 (decision 41's implicit dependency).
    let args = json!({"edits": [task_in("t6", "web/b")]});
    planner_tool(&mut fx, (PLANNER + 1, "web"), "submit_epic", args);
    assert_eq!(fx.task("t6").orch.gate_hold.as_deref(), Some("epic:web.2"));
    assert_eq!(hold_state(&fx, "epic:web.2"), Some(HoldState::Awaiting));
    fx.tick();
    assert!(!prepared(&fx, "t6"), "the re-plan's task waits");
    hold_verdict(&mut fx, "epic:web.2", true);
    assert!(prepared(&fx, "t6"));
}

/// A re-plan whose sub-planner adds no task asks the user nothing: its round, left
/// `Drafting` with no live task, is dropped once the planner has finished. Here it
/// amends `t2`, still held under `epic:mail` (M9.8 review ruling 2 lets a planner
/// amend only unapproved tasks), which stays the user's to decide.
#[test]
fn an_accepted_epic_with_no_new_task_asks_the_user_nothing() {
    let mut fx = planning_mail(false);
    submit_epic(&mut fx, json!([task_in("t2", "mail")]));
    planner_ended(&mut fx, ("mail", 1), ScoutEnd::Reported);
    assert_eq!(spawn(&mut fx, "mail")["hold"], "epic:mail.2");
    planner_started(&mut fx, PLANNER + 1);
    let amend = json!({"op": "amend_task", "task_id": "t2", "brief": "Clearer brief"});
    let args = json!({"edits": [amend]});
    let effects = planner_tool(&mut fx, (PLANNER + 1, "mail"), "submit_epic", args);
    assert_eq!(replies(&effects), vec![Ok(EPIC_RECORDED.to_string())]);
    assert_eq!(fx.task("t2").spec.brief, "Clearer brief");
    assert_eq!(hold_state(&fx, "epic:mail.2"), None);
    assert_eq!(hold_state(&fx, "epic:mail"), Some(HoldState::Awaiting));
}

/// M9.7 second review, items 8 and 10: the orchestrator's `submit` moves every
/// `Drafting` round it owns to `Awaiting`: here an epic round its own addition opened,
/// whose planner has finished. An epic whose planner is still live keeps its round
/// `Drafting` for its `submit_epic`.
#[test]
fn orchestrator_submit_submits_its_drafting_epic_rounds() {
    let mut fx = held(false);
    assert_eq!(hold_state(&fx, "epic:mail"), Some(HoldState::Drafting));
    assert_eq!(spawn(&mut fx, "web")["hold"], "epic:web");
    assert_eq!(epic(&fx, "web").phase, PlannerPhase::Planning);
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [], "submit": true})));
    assert!(ok, "{value}");
    assert_eq!(hold_state(&fx, "epic:mail"), Some(HoldState::Awaiting));
    assert_eq!(hold_state(&fx, "epic:web"), Some(HoldState::Drafting));
    hold_verdict(&mut fx, "epic:mail", true);
    assert!(prepared(&fx, "t2"), "the user's approval releases it");
}

/// Decision 34: a worker's `CreateWindow` names its task's scout reports and where the
/// driver puts their extract; filled, the first turn is the prompt with the extract.
/// A task with no `scout_refs` needs none.
#[test]
fn worker_first_turn_carries_the_scout_extract() {
    use crate::run::contract::worker_prompt;
    use crate::run::orch::extract::scout_extract;
    let tasks = [
        task(
            "t1",
            "S",
            "auth",
            "scout_refs = [\"3f9a-api\", \"onboarding\"]",
        ),
        task("t2", "S", "mail", ""),
    ];
    let mut fx = Fixture::new(&plan_with(PROFILE, &tasks));
    fx.ready(true);
    fx.run_mut().onboarding_report = Some("onboarding-7".into());
    fx.launch_all();
    let windows = fx.ops("CreateWindow");
    let slot_of = |task: &str| {
        windows
            .iter()
            .find_map(|(_, k)| match k {
                OpKind::CreateWindow {
                    extract, worktree, ..
                } if worktree.ends_with(task) => Some(extract.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no window for {task}"))
    };
    assert_eq!(slot_of("t2"), None);
    let slot = slot_of("t1").expect("t1 names reports");
    assert_eq!(
        slot.refs,
        vec!["3f9a-api".to_string(), "onboarding".to_string()]
    );
    assert_eq!(slot.onboarding.as_deref(), Some("onboarding-7"));
    let report = super::run_scouts::report("3f9a-api", "The API lives in crates/api.");
    let extract = scout_extract(&[("3f9a-api".to_string(), report)]);
    let (_, kind) = windows
        .iter()
        .find(
            |(_, k)| matches!(k, OpKind::CreateWindow { worktree, .. } if worktree.ends_with("t1")),
        )
        .unwrap();
    let OpKind::CreateWindow { first_turn, .. } = kind else {
        unreachable!()
    };
    let t1 = fx.task("t1");
    assert_eq!(first_turn, &worker_prompt(fx.run(), t1, "", ""));
    assert_eq!(
        slot.fill(first_turn, &extract),
        worker_prompt(fx.run(), t1, &extract, "")
    );
    assert!(
        slot.fill(first_turn, &extract)
            .contains("Scout report 3f9a-api:\n  The API lives in crates/api.\nFiles: none"),
    );
}
