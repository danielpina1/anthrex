//! Milestone 9.5 task 9: what a start froze from history is what a running run uses
//! (decision 12, ruling RH-8): an edit or a later round never re-tunes, and rung 4's
//! ceiling follows the frozen M budget (ruling RH-4) within the budget epoch.

use proto::{ClassBudget, PlanEdit, TaskState, TuningFile};
use serde_json::json;

use super::control::retry;
use super::dispatch::edit;
use super::done::one_reply;
use super::fixture::*;
use super::goal_rounds_stages::{add_in, plan_round};
use super::goal_rounds_start::{complete, iterate, reply, started};
use super::turns::killed_exit;
use crate::run::engine::{AgentSignal, Effect, OpResult};
use crate::run::refit::{Tuned, tuned};

/// What a start freezes from a `tuning.toml` whose budgets are `budgets`.
fn tuned_with(budgets: &[(&str, u32, u32)]) -> Tuned {
    let mut file = TuningFile::default();
    for &(class, tool_calls, minutes) in budgets {
        let budget = ClassBudget {
            tool_calls,
            minutes,
            tokens: None,
            samples: 34,
            at: 1_790_500_000,
        };
        file.budgets.insert(class.into(), budget);
    }
    tuned(&file, &config::Orchestrator::default())
}

fn calls_and_minutes(b: proto::Budget) -> (u32, u32) {
    (b.tool_calls, b.minutes)
}

fn tool_use(fx: &mut Fixture, window: u32) -> Vec<Effect> {
    fx.signal(
        window,
        AgentSignal::ToolUse {
            name: "Bash".into(),
            target: None,
        },
    )
}

/// `extra`'s task t1 of `size`, working in its one window, with `tuning` frozen.
fn working_tuned(size: &str, tuning: Tuned) -> (Fixture, u32) {
    let plan = plan_with(PROFILE, &[task("t1", size, "a", "")]);
    let mut fx = Fixture::with_tuning(&plan, tuning);
    fx.ready(true);
    let window = fx.launch_all()[0].1;
    (fx, window)
}

/// `run edit` adding an S task `id` in `crates/<module>`.
fn add_s(fx: &mut Fixture, id: &str, module: &str) -> Vec<Effect> {
    let text = plan_with(PROFILE, &[task(id, "S", module, "")]);
    let spec = crate::run::plan::parse_plan(&text).unwrap().tasks[0].clone();
    edit(fx, vec![PlanEdit::AddTask { task: spec }])
}

#[test]
fn refits_never_change_a_running_run() {
    let plan = plan_with(PROFILE, &[task("t1", "S", "a", "")]);
    let mut fx = Fixture::with_tuning(&plan, tuned_with(&[("s", 55, 18)]));
    fx.ready(true);
    assert_eq!(calls_and_minutes(fx.task("t1").budget), (55, 18));
    // Whatever history teaches from now on (the fixture's next build would say 80),
    // the run keeps what it froze.
    fx.tuning = tuned_with(&[("s", 80, 30)]);
    assert!(one_reply(&add_s(&mut fx, "t2", "b")).is_ok());
    assert_eq!(calls_and_minutes(fx.task("t2").budget), (55, 18));
    assert_eq!(calls_and_minutes(fx.run().limits.budget_s), (55, 18));
}

#[test]
fn a_round_is_not_retuned() {
    let mut fx = complete();
    // What round 1's start froze, then a refit that changed `tuning.toml` since.
    let config = config::Orchestrator::default();
    fx.run_mut()
        .limits
        .freeze(&tuned_with(&[("s", 55, 18)]), &config);
    fx.tuning = tuned_with(&[("s", 80, 30)]);
    assert_eq!(reply(&iterate(&mut fx, "more")), started(2));
    plan_round(&mut fx, json!([add_in("t2", "mail", 2, &[])]));
    assert_eq!(fx.task("t2").round, 2);
    assert_eq!(calls_and_minutes(fx.task("t2").budget), (55, 18));
}

/// Ruling T8-6: an S refit above the frozen M budget is not cut at M's: the ceiling is
/// never below the class's own budget.
#[test]
fn an_s_refit_above_m_keeps_its_own_ceiling() {
    let (mut fx, window) = working_tuned("S", tuned_with(&[("s", 175, 70)]));
    assert_eq!(calls_and_minutes(fx.run().limits.budget_m), (150, 60));
    fx.task_mut("t1").spent_total.tool_calls = 160;
    tool_use(&mut fx, window);
    assert_eq!(fx.task("t1").state, TaskState::Working, "161 of 175");
    fx.task_mut("t1").spent_total.tool_calls = 174;
    tool_use(&mut fx, window);
    assert_eq!(
        (fx.task("t1").state, fx.task("t1").rung),
        (TaskState::Blocked, 4)
    );
}

/// Ruling RH-4: an M task's ceiling is the larger of L's budget and twice the frozen
/// M budget (here 400 tool calls over L's 300); an S task's is the frozen M budget.
#[test]
fn rung_4_ceiling_follows_the_frozen_m_budget() {
    let (mut fx, window) = working_tuned("M", tuned_with(&[("m", 200, 90)]));
    assert_eq!(calls_and_minutes(fx.run().limits.budget_l), (300, 120));
    fx.task_mut("t1").spent_total.tool_calls = 398;
    tool_use(&mut fx, window);
    assert_eq!(fx.task("t1").state, TaskState::Working, "399 of 400");
    tool_use(&mut fx, window);
    assert_eq!(
        (fx.task("t1").state, fx.task("t1").rung),
        (TaskState::Blocked, 4)
    );

    let (mut fx, window) = working_tuned("S", tuned_with(&[("m", 200, 90)]));
    fx.task_mut("t1").spent_total.tool_calls = 198;
    tool_use(&mut fx, window);
    assert_eq!(fx.task("t1").state, TaskState::Working, "199 of 200");
    tool_use(&mut fx, window);
    assert_eq!(
        (fx.task("t1").state, fx.task("t1").rung),
        (TaskState::Blocked, 4)
    );
}

/// Ruling RH-4 with ruling T15-C1: after `run retry`, only the spend since the retry
/// counts against the frozen ceiling.
#[test]
fn rung_4_compares_within_the_budget_epoch() {
    let (mut fx, window) = working_tuned("S", tuned_with(&[("m", 200, 90)]));
    fx.task_mut("t1").spent_total.tool_calls = 199;
    tool_use(&mut fx, window);
    assert_eq!(
        (fx.task("t1").state, fx.task("t1").rung),
        (TaskState::Blocked, 4)
    );
    assert!(one_reply(&retry(&mut fx, "t1")).is_ok());
    let effects = killed_exit(&mut fx, window);
    let (op, _) = ops_in(&effects, "DiffSoFar")[0].clone();
    fx.done(
        op,
        OpResult::Diff {
            stat: String::new(),
            patch: String::new(),
        },
    );
    let fresh = fx.complete_windows()[0].1;
    assert_eq!(fx.task("t1").state, TaskState::Working);
    // 200 before the retry, 198 since: 398 in all, under the ceiling within the epoch.
    fx.task_mut("t1").spent_total.tool_calls = 398;
    tool_use(&mut fx, fresh);
    assert_eq!(
        fx.task("t1").state,
        TaskState::Working,
        "199 since the retry"
    );
    tool_use(&mut fx, fresh);
    assert_eq!(
        (fx.task("t1").state, fx.task("t1").rung),
        (TaskState::Blocked, 4)
    );
}
