//! The Claude tool-search fix (2026-09-27): a worker's and a reviewer's Claude session
//! gets `ENABLE_TOOL_SEARCH=false` exactly once, on top of its spec's `env`, the
//! variables `HeadlessHandle::spawn` sets; a Codex session does not.

use super::*;
use crate::headless::session_vars;
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
use proto::Effort;

fn pins(runtime: Runtime, env: &[(String, String)]) -> usize {
    session_vars(runtime, env)
        .iter()
        .filter(|(key, _)| key == "ENABLE_TOOL_SEARCH")
        .inspect(|(_, value)| assert_eq!(value, "false"))
        .count()
}

fn route(runtime: Runtime) -> Route {
    Route {
        runtime,
        model: "m".into(),
        effort: Effort::MEDIUM,
    }
}

fn run() -> Run {
    run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ))
}

#[test]
fn a_claude_worker_gets_tool_search_off_exactly_once() {
    let run = run();
    let worker = worker_spec(&run, &run.tasks[0]);
    assert_eq!(worker.runtime, Runtime::Claude);
    assert_eq!(pins(worker.runtime, &worker.env), 1, "{:?}", worker.env);
}

#[test]
fn a_claude_reviewer_gets_tool_search_off_exactly_once() {
    let run = run();
    let reviewer = reviewer_spec(&run, &run.tasks[0], &route(Runtime::Claude));
    assert_eq!(
        pins(reviewer.runtime, &reviewer.env),
        1,
        "{:?}",
        reviewer.env
    );
    let codex = reviewer_spec(&run, &run.tasks[0], &route(Runtime::Codex));
    assert_eq!(pins(codex.runtime, &codex.env), 0, "{:?}", codex.env);
}
