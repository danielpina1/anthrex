//! Milestone 9.0.7 task 11: the plan review's summary fixture (§6.4's mockup), shared
//! by `app::plan_summary`'s tests and the review's frame tests.

use super::run_fixtures::{PROJECT, run, snapshot, task};
use proto::{Route, RunInfo, RunState, RunsSnapshot, Runtime, Size, TaskInfo, TaskState, TestMode};

/// The plan's run id and goal: `kit::run_name_in` names it `Add mul() · 0723`.
pub(crate) const PLAN_RUN: &str = "add-mul-0723";
pub(crate) const PLAN_GOAL: &str = "Add mul()";

pub(crate) fn route(runtime: Runtime, model: &str) -> Route {
    serde_json::from_value(serde_json::json!({
        "runtime": runtime, "model": model, "strength": "standard", "effort": "medium",
    }))
    .expect("a Route")
}

/// A pending task of `size` in `stage` with a budget of `calls` tool calls.
pub(crate) fn plan_task(id: &str, title: &str, size: Size, stage: u16, calls: u32) -> TaskInfo {
    let mut t = task(id, title, size, TaskState::Pending);
    t.stage = stage;
    t.budget.tool_calls = calls;
    t.brief = format!("Build {title}.");
    t.owns = vec![format!("crates/{id}/src/lib.rs")];
    t.acceptance = vec![format!("{id} works")];
    t
}

/// §6.4's plan, at its gate: `t1` S (40 calls) stage 1 on `cx gpt-6-sol`; `t2` M (100)
/// stage 2 after `t1` on `cl opus`; `t3` S (50) stage 2 after `t2`, test mode none, on
/// `cl haiku`; the critical path `t1 › t2 › t3`.
pub(crate) fn three_task_plan() -> RunInfo {
    let mut info = run(PLAN_RUN, PROJECT, RunState::AwaitingApproval);
    info.goal = PLAN_GOAL.into();
    let mut t1 = plan_task("t1", "add mul() to a", Size::S, 1, 40);
    t1.route = route(Runtime::Codex, "gpt-6-sol");
    let mut t2 = plan_task("t2", "report_product in c", Size::M, 2, 100);
    t2.deps = vec!["t1".into()];
    t2.route = route(Runtime::Claude, "claude-opus-5-5");
    let mut t3 = plan_task("t3", "docs", Size::S, 2, 50);
    t3.deps = vec!["t2".into()];
    t3.test_mode = TestMode::None;
    t3.route = route(Runtime::Claude, "claude-haiku-4-5");
    info.tasks = vec![t1, t2, t3];
    info.critical_path = vec!["t1".into(), "t2".into(), "t3".into()];
    info
}

/// The plan as the frame tests draw it: `t3` after `t1` by an implied dep only, so
/// `t2` and `t3` can run together and both own `crates/c/src/lib.rs`.
pub(crate) fn overlapping_plan() -> RunsSnapshot {
    let mut info = three_task_plan();
    info.tasks[1].owns = vec!["crates/c/src/lib.rs".into()];
    info.tasks[2].owns = vec!["crates/c/src/".into()];
    info.tasks[2].deps.clear();
    info.tasks[2].implicit_deps = vec!["t1".into()];
    snapshot(10_000, vec![info])
}
