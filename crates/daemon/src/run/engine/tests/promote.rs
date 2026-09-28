//! Milestone 9 task M9.7: `run promote` gives a fast-path run an orchestrator
//! (decision 29). The fast-path task runs on; what the orchestrator adds before it
//! submits waits under hold `promotion`; a promotion recorded before milestone 9 is
//! performed on resume or on the tick.

use proto::{HoldState, OrchestratorChoice, RunPath, RunState, Runtime, TaskState};
use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use super::orch::{ORCH, add, answer, edit_plan};
use crate::run::engine::{Effect, EngineState, EventKind, OpKind, OpResult};
use crate::run::model::Run;
use crate::run::triage::mark_fast;

/// A running fast-path run of `t1` with the deciders off, its run branch made.
fn fast() -> Fixture {
    let mut fx = Fixture::new(&plan_with(PROFILE, &[task("t1", "S", "auth", "")]));
    fx.start_with(false, |run: &mut Run| {
        mark_fast(run, super::orch::triage(RunPath::Fast), None);
    });
    let (op, _) = fx.op("CreateRunBranch");
    fx.done(op, OpResult::Worktree { head: BASE.into() });
    fx
}

fn promote(fx: &mut Fixture, choice: Option<OrchestratorChoice>) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Promote {
        reply,
        run_id: RUN_ID.into(),
        orchestrator: choice,
    })
}

/// [`fast`], promoted, with its orchestrator in window [`ORCH`].
fn promoted() -> Fixture {
    promoted_from(fast())
}

/// `fx`, a running fast-path run, promoted, with its orchestrator in window [`ORCH`].
fn promoted_from(mut fx: Fixture) -> Fixture {
    promote(&mut fx, None);
    let (op, _) = fx.op("CreateOrchestrator");
    fx.done(
        op,
        OpResult::Window {
            window_id: ORCH,
            pid: None,
        },
    );
    fx
}

#[test]
fn promote_creates_an_orchestrator_and_leaves_t1_running() {
    let mut fx = fast();
    let t1 = fx.task("t1").clone();
    assert_eq!(t1.state, TaskState::Preparing);
    let choice = OrchestratorChoice {
        runtime: Runtime::Codex,
        model: None,
    };
    let effects = promote(&mut fx, Some(choice));
    assert_eq!(
        replies(&effects),
        vec![Ok(format!(
            "promoted: run {RUN_ID} now has an orchestrator; it starts in a moment (anthrex run status {RUN_ID})"
        ))]
    );
    let run = fx.run();
    assert_eq!(run.path, Some(RunPath::Plan));
    assert_eq!(run.promote_requested_at, Some(fx.now));
    assert_eq!(run.state, RunState::Running);
    let o = run.orch.orchestrator.as_ref().unwrap();
    assert_eq!(o.route.runtime, Runtime::Codex, "the user's choice");
    assert!(!o.plan_submitted);
    let creates = ops_in(&effects, "CreateOrchestrator");
    assert_eq!(creates.len(), 1);
    let OpKind::CreateOrchestrator { spec, .. } = &creates[0].1 else {
        unreachable!()
    };
    let first = spec.initial_prompt.clone().unwrap();
    assert!(
        first.contains("promoted from the fast path at the user's request"),
        "{first}"
    );
    assert_eq!(first, o.first_prompt);
    assert_eq!(*fx.task("t1"), t1, "t1 is exactly as it was");
    // The attention line of milestone 8b is gone: the orchestrator shows instead.
    let info = &crate::run::snapshot::snapshot(&fx.state, fx.now).runs[0];
    assert!(
        !info
            .attention
            .iter()
            .any(|a| a.starts_with("promotion requested")),
        "{:?}",
        info.attention
    );
    assert_eq!(
        info.orchestrator.as_ref().map(|o| o.route.runtime),
        Some(Runtime::Codex)
    );
}

#[test]
fn promote_repeat_reply_has_no_time() {
    let mut fx = promoted();
    let before = fx.run().clone();
    fx.now += 600;
    let effects = promote(&mut fx, None);
    assert_eq!(
        replies(&effects),
        vec![Ok(format!("run {RUN_ID} was already marked for promotion"))]
    );
    assert!(ops_in(&effects, "CreateOrchestrator").is_empty());
    assert_eq!(*fx.run(), before);
}

#[test]
fn tasks_added_before_submit_after_promote_carry_the_promotion_hold() {
    let mut fx = promoted();
    let (ok, value) = answer(&edit_plan(
        &mut fx,
        json!({"edits": [add("t2", "mail"), add("t3", "sms")]}),
    ));
    assert!(ok, "{value}");
    assert_eq!(value["held"], "promotion");
    assert_eq!(value["awaiting_approval"], false);
    let hold = fx.run().orch.gate_holds[0].clone();
    assert_eq!(hold.id, "promotion");
    assert_eq!(hold.kind, proto::HoldKind::Promotion);
    assert_eq!(hold.state, HoldState::Drafting);
    assert_eq!(hold.tasks, vec!["t2".to_string(), "t3".to_string()]);
    fx.tick();
    for id in ["t2", "t3"] {
        assert_eq!(fx.task(id).state, TaskState::Queued, "{id} waits");
        assert_eq!(fx.task(id).orch.gate_hold.as_deref(), Some("promotion"));
    }
    // A user's own `run edit` of the promoted run is not held (and is no longer
    // refused as a fast-path addition).
    let effects = super::dispatch::edit(
        &mut fx,
        vec![proto::PlanEdit::AddTask {
            task: super::holds::plan_task("t4", "[\"crates/log/**\"]"),
        }],
    );
    assert_eq!(replies(&effects), vec![Ok("applied 1 edit".to_string())]);
    assert_eq!(fx.task("t4").orch.gate_hold, None);
}

#[test]
fn promotion_hold_is_awaiting_after_submit() {
    let mut fx = promoted();
    edit_plan(&mut fx, json!({"edits": [add("t2", "mail")]}));
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [], "submit": true})));
    assert!(ok, "{value}");
    let run = fx.run();
    assert_eq!(run.orch.gate_holds[0].state, HoldState::Awaiting);
    assert!(run.orch.orchestrator.as_ref().unwrap().plan_submitted);
    assert_eq!(run.state, RunState::Running);
    assert_eq!(fx.task("t2").state, TaskState::Queued);
    // M9.7 second review, ruling 1: until the user approves, an addition joins the
    // awaiting hold (the first review's "no longer held" was the defect).
    edit_plan(&mut fx, json!({"edits": [add("t3", "sms")]}));
    assert_eq!(fx.task("t3").orch.gate_hold.as_deref(), Some("promotion"));
    let reply = fx.reply();
    let effects = fx.next(EventKind::Orch(
        crate::run::engine::OrchEvent::ApproveHold {
            reply,
            run_id: RUN_ID.into(),
            hold: "promotion".into(),
        },
    ));
    assert!(replies(&effects)[0].is_ok());
    assert_eq!(fx.task("t2").state, TaskState::Preparing);
}

/// A fast-path run on which milestone 8b recorded `run promote`.
fn recorded() -> Fixture {
    let mut fx = fast();
    fx.run_mut().promote_requested_at = Some(1_500);
    fx
}

#[test]
fn pre_m9_promote_request_is_performed_on_resume_and_on_tick() {
    // On the tick, while it runs.
    let mut fx = recorded();
    let effects = fx.tick();
    assert_eq!(ops_in(&effects, "CreateOrchestrator").len(), 1);
    assert_eq!(fx.run().path, Some(RunPath::Plan));
    assert_eq!(
        fx.run().promote_requested_at,
        Some(1_500),
        "its own time kept"
    );
    assert!(ops_in(&fx.tick(), "CreateOrchestrator").is_empty(), "once");

    // After a daemon restart the run is paused: nothing on the tick, then on resume.
    let mut fx = recorded();
    let runs: Vec<_> = fx.state.runs.values().cloned().collect();
    fx.state = EngineState::default();
    fx.next(EventKind::Restore {
        runs,
        replay: Vec::new(),
        held: Vec::new(),
    });
    assert_eq!(fx.run().state, RunState::Paused);
    assert!(ops_in(&fx.tick(), "CreateOrchestrator").is_empty());
    let reply = fx.reply();
    let effects = fx.next(EventKind::Resume {
        reply,
        run_id: RUN_ID.into(),
        rebaseline: None,
    });
    assert_eq!(ops_in(&effects, "CreateOrchestrator").len(), 1);
    assert!(fx.run().orch.orchestrator.is_some());

    // The control: a fast-path run with no recorded wish is left alone.
    let mut fx = fast();
    assert!(ops_in(&fx.tick(), "CreateOrchestrator").is_empty());
}

fn hold_verdict(fx: &mut Fixture, hold: &str, approve: bool) -> Vec<Effect> {
    let reply = fx.reply();
    let (run_id, hold) = (RUN_ID.to_string(), hold.to_string());
    fx.next(EventKind::Orch(if approve {
        crate::run::engine::OrchEvent::ApproveHold {
            reply,
            run_id,
            hold,
        }
    } else {
        crate::run::engine::OrchEvent::RejectHold {
            reply,
            run_id,
            hold,
        }
    }))
}

fn prepared(fx: &Fixture, id: &str) -> bool {
    fx.ops("PrepareWorktree")
        .iter()
        .any(|(_, k)| op_task(k) == id)
}

/// M9.7 review fixes, ruling 1: the promotion hold keys on the promotion, not on
/// `approved_at`. A fast-path run started under milestone 8b has none.
#[test]
fn a_promoted_run_with_no_approval_time_holds_its_additions() {
    let mut fx = fast();
    fx.run_mut().approved_at = None;
    let mut fx = promoted_from(fx);
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [add("t2", "mail")]})));
    assert!(ok, "{value}");
    assert_eq!(value["held"], "promotion");
    fx.tick();
    assert_eq!(fx.task("t2").state, TaskState::Queued);
    assert!(!prepared(&fx, "t2"), "t2 is not dispatched");
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [], "submit": true})));
    assert!(ok, "{value}");
    fx.tick();
    assert!(!prepared(&fx, "t2"), "the hold awaits the user");
    let effects = hold_verdict(&mut fx, "promotion", true);
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    assert_eq!(fx.task("t2").state, TaskState::Preparing);
    assert!(prepared(&fx, "t2"));
}

fn child(id: &str, module: &str) -> serde_json::Value {
    add(id, module)["task"].clone()
}

/// M9.7 review fixes, ruling 2: a task split from a held task keeps its hold.
#[test]
fn split_children_of_a_promotion_held_task_stay_held() {
    let mut fx = promoted();
    edit_plan(&mut fx, json!({"edits": [add("t2", "mail")]}));
    edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    let split = json!({"op": "split_task", "task_id": "t2",
        "into": [child("t2a", "mail_a"), child("t2b", "mail_b")]});
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [split]})));
    assert!(ok, "{value}");
    assert_eq!(value["held"], "promotion");
    fx.tick();
    for id in ["t2a", "t2b"] {
        assert_eq!(fx.task(id).state, TaskState::Queued, "{id} waits");
        assert_eq!(fx.task(id).orch.gate_hold.as_deref(), Some("promotion"));
        assert!(!prepared(&fx, id), "{id} is not dispatched");
    }
    let hold = &fx.run().orch.gate_holds[0];
    assert_eq!(hold.tasks, vec!["t2a".to_string(), "t2b".to_string()]);
    assert_eq!(fx.task("t2").state, TaskState::Cancelled);
    assert_eq!(fx.task("t2").orch.gate_hold, None, "split left the hold");
    let effects = hold_verdict(&mut fx, "promotion", false);
    assert_eq!(
        replies(&effects),
        vec![Ok(format!(
            "hold promotion of run {RUN_ID} rejected: 2 tasks cancelled"
        ))]
    );
    for id in ["t2a", "t2b"] {
        assert_eq!(fx.task(id).state, TaskState::Cancelled, "{id}");
    }
}

/// M9.7 second review, ruling 1, scenario A: an empty submit first. The promotion hold
/// is created `Awaiting` with no task, so the user's approval still ends the window.
#[test]
fn an_empty_submit_after_promote_holds_later_additions_until_approval() {
    let mut fx = promoted();
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [], "submit": true})));
    assert!(ok, "{value}");
    assert_eq!(value["held"], serde_json::Value::Null);
    let hold = fx.run().orch.gate_holds[0].clone();
    assert_eq!(
        (hold.id.as_str(), hold.state, hold.tasks.len()),
        ("promotion", HoldState::Awaiting, 0)
    );
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [add("t2", "mail")]})));
    assert!(ok, "{value}");
    assert_eq!(value["held"], "promotion");
    assert_eq!(value["awaiting_approval"], true);
    fx.tick();
    assert_eq!(fx.task("t2").state, TaskState::Queued);
    assert!(!prepared(&fx, "t2"), "t2 waits for the user");
    let effects = hold_verdict(&mut fx, "promotion", true);
    assert_eq!(
        replies(&effects),
        vec![Ok(format!(
            "hold promotion of run {RUN_ID} approved: 1 task may start"
        ))]
    );
    assert!(prepared(&fx, "t2"), "dispatched on approval");
}

/// M9.7 second review, ruling 1, scenario B: an addition made after the submit, while
/// the user has not decided, joins the awaiting hold. After the approval, decision 29's
/// "only new epics are held" applies.
#[test]
fn additions_after_submit_join_the_awaiting_promotion_hold() {
    let mut fx = promoted();
    // Room for t1 to t4 at once, so a task left queued is one that was held.
    fx.run_mut().limits.max_writers = 4;
    edit_plan(&mut fx, json!({"edits": [add("t2", "mail")]}));
    edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    assert_eq!(fx.run().orch.gate_holds[0].state, HoldState::Awaiting);
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [add("t3", "sms")]})));
    assert!(ok, "{value}");
    assert_eq!(value["held"], "promotion");
    assert_eq!(
        fx.run().orch.gate_holds[0].tasks,
        vec!["t2".to_string(), "t3".to_string()]
    );
    fx.tick();
    for id in ["t2", "t3"] {
        assert!(!prepared(&fx, id), "{id} waits for the user");
    }
    let effects = hold_verdict(&mut fx, "promotion", true);
    assert_eq!(
        replies(&effects),
        vec![Ok(format!(
            "hold promotion of run {RUN_ID} approved: 2 tasks may start"
        ))]
    );
    for id in ["t2", "t3"] {
        assert!(prepared(&fx, id), "{id} dispatched on approval");
    }
    // After the approval, an addition in no new epic is not held.
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [add("t4", "log")]})));
    assert!(ok, "{value}");
    assert_eq!(value["held"], serde_json::Value::Null);
    assert_eq!(fx.task("t4").orch.gate_hold, None);
    fx.tick();
    assert!(prepared(&fx, "t4"), "t4 dispatched");
}

/// M9.7 second review, ruling 2: `past_gate` counts the promotion, so a promoted M8b
/// run with no approval time holds a new epic once the promotion was approved.
#[test]
fn a_promoted_m8b_run_holds_a_new_epic_after_submit() {
    let mut fx = fast();
    fx.run_mut().approved_at = None;
    let mut fx = promoted_from(fx);
    edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    let effects = hold_verdict(&mut fx, "promotion", true);
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    assert_eq!(fx.run().approved_at, None);
    let args = json!({"epic": "mail", "title": "Epic mail",
        "area": ["crates/mail/**"], "brief": "Plan mail"});
    let (ok, value) = answer(&super::orch::orch_tool(
        &mut fx,
        ORCH,
        "spawn_subplanner",
        args,
    ));
    assert!(ok, "{value}");
    assert_eq!(value["hold"], "epic:mail");
}

/// M9.7 second review, ruling 7: a rejection does not end the promotion window. The
/// next addition opens a new round, `promotion-2`, under the same rules.
#[test]
fn after_a_rejected_promotion_additions_open_a_new_round() {
    let mut fx = promoted();
    edit_plan(&mut fx, json!({"edits": [add("t2", "mail")]}));
    edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    let effects = hold_verdict(&mut fx, "promotion", false);
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [add("t3", "sms")]})));
    assert!(ok, "{value}");
    assert_eq!(value["held"], "promotion-2");
    let round = fx.run().orch.gate_holds[1].clone();
    assert_eq!(
        (round.id.as_str(), round.kind, round.state, round.tasks),
        (
            "promotion-2",
            proto::HoldKind::Promotion,
            HoldState::Drafting,
            vec!["t3".to_string()]
        )
    );
    fx.tick();
    assert!(!prepared(&fx, "t3"), "t3 waits for the new round");
    edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    fx.tick();
    assert!(!prepared(&fx, "t3"), "the new round awaits the user");
    let effects = hold_verdict(&mut fx, "promotion-2", true);
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    assert!(prepared(&fx, "t3"), "dispatched on approval");
}

/// M9.7 second review, ruling 7: after a rejection, an empty submit opens the new round
/// `Awaiting` with no task, and a later addition joins it.
#[test]
fn after_a_rejected_promotion_an_empty_submit_opens_an_awaiting_round() {
    let mut fx = promoted();
    edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    hold_verdict(&mut fx, "promotion", false);
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [], "submit": true})));
    assert!(ok, "{value}");
    let round = fx.run().orch.gate_holds[1].clone();
    assert_eq!(
        (round.id.as_str(), round.state, round.tasks.len()),
        ("promotion-2", HoldState::Awaiting, 0)
    );
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [add("t2", "mail")]})));
    assert!(ok, "{value}");
    assert_eq!(value["held"], "promotion-2");
    fx.tick();
    assert!(!prepared(&fx, "t2"), "t2 waits for the user");
}

fn spawn(fx: &mut Fixture, epic: &str) -> serde_json::Value {
    let args = json!({"epic": epic, "title": format!("Epic {epic}"),
        "area": [format!("crates/{epic}/**")], "brief": format!("Plan {epic}")});
    let (ok, value) = answer(&super::orch::orch_tool(fx, ORCH, "spawn_subplanner", args));
    assert!(ok, "{value}");
    value
}

/// The orchestrator's addition of `id` to `epic`, as its finished sub-planner's would be.
fn add_to_epic(fx: &mut Fixture, id: &str, epic: &str) -> serde_json::Value {
    let e = fx
        .run_mut()
        .orch
        .epics
        .iter_mut()
        .find(|e| e.epic == epic)
        .unwrap();
    e.phase = crate::run::orch::PlannerPhase::Finished;
    let mut edit = add(id, id);
    edit["task"]["epic"] = json!(epic);
    let (ok, value) = answer(&edit_plan(fx, json!({"edits": [edit]})));
    assert!(ok, "{value}");
    value
}

/// M9.7 second review, ruling 8.1: a promoted run's submit waits for its sub-planners,
/// and an epic spawned in the promotion window has its tasks held under the round.
#[test]
fn epic_spawned_before_submit_stays_held() {
    let mut fx = promoted();
    spawn(&mut fx, "mail");
    let effects = edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    assert_eq!(
        super::orch::error(&effects),
        "sub-planner mail is still planning; submit when every sub-planner has finished"
    );
    assert!(fx.run().orch.gate_holds.is_empty(), "nothing submitted");
    assert_eq!(add_to_epic(&mut fx, "t7", "mail")["held"], "promotion");
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [], "submit": true})));
    assert!(ok, "{value}");
    fx.tick();
    assert!(!prepared(&fx, "t7"), "t7 waits for the user");
    hold_verdict(&mut fx, "promotion", true);
    assert!(prepared(&fx, "t7"), "dispatched on approval");
}

/// M9.7 second review, ruling 8.2: a rejected epic, planned again, opens a new round
/// `epic:mail.2` (a `.` cannot appear in an epic id, so no epic's own hold collides).
#[test]
fn rejected_epic_replanned_stays_held() {
    let mut fx = promoted();
    edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    hold_verdict(&mut fx, "promotion", true);
    assert_eq!(spawn(&mut fx, "mail")["hold"], "epic:mail");
    assert_eq!(add_to_epic(&mut fx, "t5", "mail")["held"], "epic:mail");
    let now = fx.now;
    crate::run::engine::gate_holds::submitted(fx.run_mut(), "epic:mail", now);
    hold_verdict(&mut fx, "epic:mail", false);
    assert_eq!(fx.task("t5").state, TaskState::Cancelled);
    assert_eq!(spawn(&mut fx, "mail")["hold"], "epic:mail.2");
    assert_eq!(add_to_epic(&mut fx, "t6", "mail")["held"], "epic:mail.2");
    let now = fx.now;
    crate::run::engine::gate_holds::submitted(fx.run_mut(), "epic:mail.2", now);
    fx.tick();
    assert!(!prepared(&fx, "t6"), "t6 waits for the user");
    let effects = hold_verdict(&mut fx, "epic:mail.2", true);
    assert!(replies(&effects)[0].is_ok(), "{effects:?}");
    assert!(prepared(&fx, "t6"), "dispatched on approval");
}

/// M9.7 second review, ruling 8.3 (**pinning**): a resubmit while the promotion round
/// awaits the user submits nothing again.
#[test]
fn a_resubmit_while_the_promotion_round_awaits_submits_nothing() {
    let mut fx = promoted();
    edit_plan(&mut fx, json!({"edits": [add("t2", "mail")]}));
    edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [], "submit": true})));
    assert!(ok, "{value}");
    let submits = fx
        .run()
        .log
        .iter()
        .filter(|l| l.text == "the orchestrator submitted its additions")
        .count();
    assert_eq!(submits, 1);
}
