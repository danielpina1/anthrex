//! Milestone 9 task M9.7: the orchestrator's `edit_plan` (decisions 19 and 27): one
//! batch, `submit` at each state, the reply's shapes and `summary`; and
//! `spawn_subplanner` at the gate.

use proto::{RunState, TaskState};
use serde_json::json;

use super::dispatch::replies;
use super::fixture::*;
use super::orch::{
    ORCH, add, answer, but_the_log, but_the_stall, edit_plan, error, launched, orch_tool,
};
use crate::run::engine::{Effect, EventKind};
use crate::run::orch::{EpicRecord, PlannerPhase};

#[test]
fn submit_with_no_tasks_is_refused() {
    let mut fx = launched(false);
    let before = fx.run().clone();
    let effects = edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    assert_eq!(
        error(&effects),
        "the plan has no tasks yet; add tasks before submitting"
    );
    assert_eq!(*fx.run(), before);
}

#[test]
fn submit_while_a_planner_is_live_is_refused() {
    let mut fx = launched(false);
    fx.run_mut()
        .orch
        .epics
        .push(EpicRecord::new("auth", PlannerPhase::Planning));
    let before = fx.run().clone();
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "mail")], "submit": true}),
    );
    assert_eq!(
        error(&effects),
        "sub-planner auth is still planning; submit when every sub-planner has finished"
    );
    assert_eq!(
        but_the_log(fx.run()),
        but_the_log(&before),
        "the batch's edits are not applied either"
    );
    // The control: once it finished, the same call submits.
    fx.run_mut().orch.epics[0].phase = PlannerPhase::Finished;
    let (ok, _) = answer(&edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "mail")], "submit": true}),
    ));
    assert!(ok);
}

#[test]
fn submit_opens_the_gate() {
    let mut fx = launched(false);
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    let (ok, value) = answer(&effects);
    assert!(ok, "{value}");
    assert_eq!(value["awaiting_approval"], true);
    let run = fx.run();
    assert_eq!(run.state, RunState::AwaitingApproval);
    assert_eq!(run.approved_by, None);
    assert!(run.orch.orchestrator.as_ref().unwrap().plan_submitted);
    // Decision 14's pre-warm starts at the gate.
    assert_eq!(tasks_of(&effects, "PrepareWorktree"), vec!["t1"]);
    assert_eq!(fx.task("t1").state, TaskState::Queued);
}

#[test]
fn submit_with_yes_runs_at_once_and_records_approved_by() {
    let mut fx = launched(true);
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    let (ok, value) = answer(&effects);
    assert!(ok, "{value}");
    assert_eq!(value["awaiting_approval"], false);
    let run = fx.run();
    assert_eq!(run.state, RunState::Running);
    assert_eq!(run.approved_by.as_deref(), Some("--yes"));
    assert_eq!(run.approved_at, Some(fx.now));
    assert_eq!(fx.task("t1").state, TaskState::Preparing, "dispatched");
}

#[test]
fn resubmit_in_awaiting_approval_changes_nothing() {
    let mut fx = launched(false);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    let before = fx.run().clone();
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [], "submit": true})));
    assert!(ok, "{value}");
    assert_eq!(value["awaiting_approval"], true);
    assert_eq!(*fx.run(), before);
}

#[test]
fn spawn_subplanner_in_awaiting_approval_returns_to_planning() {
    let mut fx = launched(false);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    fx.complete_prepares();
    assert!(fx.task("t1").prewarmed);
    let args = json!({"epic": "mail", "title": "Mail", "area": ["crates/mail/**"],
                      "brief": "Plan the mail module"});
    let (ok, value) = answer(&orch_tool(&mut fx, ORCH, "spawn_subplanner", args));
    assert!(ok, "{value}");
    // Task M9.8: a reader slot is free, so its session starts at once (decision 31).
    assert_eq!(
        value,
        json!({"epic": "mail", "state": "planning", "hold": null})
    );
    let run = fx.run();
    assert_eq!(run.state, RunState::Planning);
    assert!(!run.orch.orchestrator.as_ref().unwrap().plan_submitted);
    assert_eq!(run.orch.epics.len(), 1);
    assert_eq!(run.orch.epics[0].phase, PlannerPhase::Planning);
    assert!(fx.task("t1").prewarmed, "pre-warmed worktrees stay");
}

#[test]
fn edit_plan_is_one_batch() {
    let mut fx = launched(false);
    edit_plan(&mut fx, json!({"edits": [add("t1", "auth")]}));
    let before = fx.run().clone();
    // The second edit names an unknown task: the first is not applied either.
    let batch = json!({"edits": [add("t2", "mail"),
        {"op": "add_dep", "task_id": "t2", "dep": "ghost"}], "submit": true,
        "summary": "not yet"});
    let (ok, value) = answer(&edit_plan(&mut fx, batch));
    assert!(!ok);
    assert_eq!(value["accepted"], false);
    assert_eq!(
        but_the_log(fx.run()),
        but_the_log(&before),
        "unchanged but for the log"
    );
    // Task M9.9 (decision 40): the rejected batch is logged, which moves the run.
    let record = fx.run().plan_edits.last().unwrap();
    assert_eq!(
        (record.source.as_str(), record.accepted),
        ("orchestrator", false)
    );
}

#[test]
fn edit_plan_reply_shapes() {
    let mut fx = launched(false);
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [add("t1", "auth")]})));
    assert!(ok);
    assert_eq!(
        value,
        json!({"accepted": true, "revision": fx.run().orch.digest_rev,
               "awaiting_approval": false,
               "notes": ["t1: the profile has no test_passed: the test proof will require the test's name in the single-test command's output (rule 8.1)",
                         "t1: size not backed by a scout report"],
               "held": null})
    );
    let bad = json!({"op": "add_task", "task": {"id": "t2", "title": "Two", "size": "S",
        "owns": ["crates/mail/**"], "brief": "b", "acceptance": ["a"],
        "budget": {"tool_calls": 1, "minutes": 1}}});
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [bad]})));
    assert!(!ok);
    assert_eq!(
        value,
        json!({"accepted": false, "errors": [{"task": "t2", "field": "budget",
            "rule": "7.1.budget",
            "message": "budgets come from the task's size; leave budget out (rule 7.1)"}]})
    );
}

#[test]
fn summary_on_a_complete_run_is_accepted_and_edits_are_not() {
    let mut fx = launched(false);
    fx.run_mut().state = RunState::Complete;
    let effects = edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "summary": "done"}),
    );
    assert_eq!(
        error(&effects),
        "edits are not accepted on a complete run; only a summary is"
    );
    let effects = orch_tool(
        &mut fx,
        ORCH,
        "spawn_subplanner",
        json!({"epic": "mail",
        "title": "Mail", "area": ["crates/mail/**"], "brief": "b"}),
    );
    assert_eq!(error(&effects), format!("run {RUN_ID} is complete"));
    let (ok, value) = answer(&edit_plan(
        &mut fx,
        json!({"edits": [], "summary": "All done."}),
    ));
    assert!(ok, "{value}");
    let o = fx.run().orch.orchestrator.as_ref().unwrap();
    assert_eq!(o.summary.as_deref(), Some("All done."));
}

#[test]
fn summary_is_written_to_the_report() {
    let mut fx = launched(false);
    let summary = "First line.\n## Tasks\nSecond line.";
    edit_plan(&mut fx, json!({"edits": [], "summary": "an earlier one"}));
    edit_plan(&mut fx, json!({"edits": [], "summary": summary}));
    let report = crate::run::report::render(fx.run(), fx.now);
    let expected = format!(
        "# anthrex run {RUN_ID}\n\n## Summary from the orchestrator\n\nFirst line.\n    \\## Tasks\n    Second line.\n\nGoal: "
    );
    assert!(report.starts_with(&expected), "{report}");
    assert!(!report.contains("an earlier one"), "the last one wins");
    assert_eq!(report.matches("\n## Tasks\n").count(), 1);
}

/// M9.16 (found end to end): the summary rewrites the report at once, so a `complete`
/// run's `REPORT.md` opens with it before the user accepts or discards the run.
#[test]
fn a_summary_rewrites_the_report() {
    let mut fx = launched(false);
    fx.run_mut().state = RunState::Complete;
    let effects = edit_plan(&mut fx, json!({"edits": [], "summary": "All done."}));
    assert!(answer(&effects).0, "{effects:?}");
    assert!(
        effects
            .iter()
            .any(|e| matches!(e, Effect::WriteReport { run_id } if run_id == RUN_ID)),
        "{effects:?}"
    );
}

/// The user's `run edit`, with or without decision 13's user submit.
fn user_edit(fx: &mut Fixture, edits: Vec<proto::PlanEdit>, submit: bool) -> Vec<Effect> {
    let reply = fx.reply();
    fx.next(EventKind::Edit {
        reply,
        run_id: RUN_ID.into(),
        edits,
        scope: crate::run::validate::EditScope::Run,
        refusals: Vec::new(),
        submit,
    })
}

/// M9.7 review fixes, ruling 5: a planning run whose orchestrator cannot go on is
/// completed by the user alone: their own tasks, their submit, their approval.
#[test]
fn a_user_submit_while_planning_opens_the_gate() {
    let mut fx = launched(false);
    let t1 = proto::PlanEdit::AddTask {
        task: super::holds::plan_task("t1", "[\"crates/auth/**\"]"),
    };
    let effects = user_edit(&mut fx, vec![t1], true);
    assert_eq!(
        replies(&effects),
        vec![Ok(format!(
            "applied 1 edit; the plan of run {RUN_ID} was submitted: it awaits approval"
        ))]
    );
    let run = fx.run();
    assert_eq!(run.state, RunState::AwaitingApproval);
    assert!(run.orch.orchestrator.as_ref().unwrap().plan_submitted);
    assert_eq!(
        replies(&fx.approve()),
        vec![Ok(format!("run {RUN_ID} approved"))]
    );
    fx.tick();
    assert_eq!(fx.task("t1").state, TaskState::Preparing);
    // A submit alone, with no task, is refused as the orchestrator's is.
    let mut fx = launched(false);
    let before = fx.run().clone();
    let effects = user_edit(&mut fx, Vec::new(), true);
    assert_eq!(
        replies(&effects),
        vec![Err(
            "the plan has no tasks yet; add tasks before submitting".to_string()
        )]
    );
    assert_eq!(*fx.run(), before);
}

#[test]
fn a_user_submit_is_refused_in_any_other_state() {
    let mut fx = launched(false);
    edit_plan(
        &mut fx,
        json!({"edits": [add("t1", "auth")], "submit": true}),
    );
    let refused = |fx: &mut Fixture| {
        let before = fx.run().clone();
        let t9 = proto::PlanEdit::AddTask {
            task: super::holds::plan_task("t9", "[\"crates/t9/**\"]"),
        };
        let effects = user_edit(fx, vec![t9], true);
        assert_eq!(*fx.run(), before, "nothing is applied");
        replies(&effects).remove(0).unwrap_err()
    };
    assert_eq!(
        refused(&mut fx),
        format!("run {RUN_ID} is awaiting_approval; only a run being planned can be submitted")
    );
    fx.approve();
    assert_eq!(
        refused(&mut fx),
        format!("run {RUN_ID} is running; only a run being planned can be submitted")
    );
}

/// M9.7 second review, ruling 4: while a discard is in flight, neither the
/// orchestrator's nor the user's submit is taken.
#[test]
fn a_submit_while_the_run_is_being_discarded_is_refused() {
    let mut fx = launched(false);
    edit_plan(&mut fx, json!({"edits": [add("t1", "auth")]}));
    let reply = fx.reply();
    fx.next(EventKind::Reject {
        reply,
        run_id: RUN_ID.into(),
    });
    let before = but_the_stall(fx.run());
    let effects = edit_plan(&mut fx, json!({"edits": [], "submit": true}));
    assert_eq!(error(&effects), format!("run {RUN_ID} is being discarded"));
    assert_eq!(but_the_stall(fx.run()), before);
    let reply = fx.reply();
    let effects = fx.next(EventKind::Edit {
        reply,
        run_id: RUN_ID.into(),
        edits: Vec::new(),
        scope: crate::run::validate::EditScope::Run,
        refusals: Vec::new(),
        submit: true,
    });
    assert_eq!(
        replies(&effects),
        vec![Err(format!("run {RUN_ID} is being discarded"))]
    );
    assert_eq!(but_the_stall(fx.run()), before);
}

/// Milestone 9.3 task M9.3.7 fix round 1 (ruling: option (b)): an `edit_plan` with no
/// `edits` is an empty batch. `{}` changes nothing but the edit log, as `{"edits": []}`
/// does; `{"submit": true}` is refused on an empty plan and submits one that has
/// tasks; `{}` on a complete run gets the complete run's refusal, and a summary alone
/// is accepted there.
#[test]
fn an_edit_plan_without_edits_is_an_empty_batch() {
    let mut fx = launched(false);
    let before = fx.run().clone();
    let (ok, value) = answer(&edit_plan(&mut fx, json!({})));
    assert!(ok, "{value}");
    assert_eq!(value["accepted"], true);
    assert_eq!(but_the_log(fx.run()), but_the_log(&before));
    assert_eq!(fx.run().state, RunState::Planning);

    let effects = edit_plan(&mut fx, json!({"submit": true}));
    assert_eq!(
        error(&effects),
        "the plan has no tasks yet; add tasks before submitting"
    );
    edit_plan(&mut fx, json!({"edits": [add("t1", "auth")]}));
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"submit": true})));
    assert!(ok, "{value}");
    assert_eq!(value["awaiting_approval"], true);
    assert_eq!(fx.run().state, RunState::AwaitingApproval);

    fx.run_mut().state = RunState::Complete;
    assert_eq!(
        error(&edit_plan(&mut fx, json!({}))),
        "edits are not accepted on a complete run; only a summary is"
    );
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"summary": "All done."})));
    assert!(ok, "{value}");
    let o = fx.run().orch.orchestrator.as_ref().unwrap();
    assert_eq!(o.summary.as_deref(), Some("All done."));
}

/// M9.11 re-review finding 3: a 17-character id reaches the plan rules, whose error
/// names the task, the field and the rule (decision 19).
#[test]
fn a_long_task_id_gets_the_plan_rules_error() {
    let mut fx = launched(false);
    let id = "a".repeat(17);
    let (ok, value) = answer(&edit_plan(&mut fx, json!({"edits": [add(&id, "auth")]})));
    assert!(!ok);
    assert_eq!(value["accepted"], json!(false), "{value}");
    assert_eq!(value["errors"][0]["task"], json!(id), "{value}");
    assert_eq!(value["errors"][0]["field"], "id", "{value}");
    assert_eq!(value["errors"][0]["rule"], "id", "{value}");
}
