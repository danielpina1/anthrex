//! What the scenarios of `run_e2e_large` share beyond M9.16's `common`: triage answers
//! of other scales and kinds, sub-planner steps and scripts, epic tasks, and the
//! engine-made integration reviews' scripts (decision 37).

use serde_json::{Value, json};

use crate::common::green;
use crate::support::orch_script::*;
use crate::support::run_harness::RunHarness;
use crate::support::run_plans::approve;

/// Base files under the epics' areas: `fake-agent`'s `git_commit` writes its file but
/// creates no directory, so each area's directory exists from the base commit.
pub const SRC_FILES: &[(&str, &str)] = &[
    ("src/api.rs", "// api\n"),
    ("src/a/mod.rs", "// a\n"),
    ("src/b/mod.rs", "// b\n"),
    ("src/c/mod.rs", "// c\n"),
];

/// M9.16's harness over [`SRC_FILES`], whose first triage answers `answer`.
pub fn harness_with(answer: Value) -> RunHarness {
    let h = RunHarness::orch("", SRC_FILES);
    h.decider("triage", 1, answer);
    h
}

/// A triage answer of `kinds` and `scale` with no fast-path task.
pub fn triage(kinds: &[&str], scale: &str) -> Value {
    json!({"answer": {"kinds": kinds, "scale": scale, "reason": "several modules", "task": null}})
}

/// The orchestrator's `spawn_subplanner` of epic `epic` over `area`.
pub fn subplanner(epic: &str, area: &[&str]) -> Value {
    call(
        "spawn_subplanner",
        json!({"epic": epic, "title": format!("Epic {epic}"), "area": area,
            "brief": format!("Plan epic {epic}.")}),
    )
}

/// A sub-planner's `submit_epic` adding `tasks`.
pub fn submit_epic(tasks: Vec<Value>) -> Value {
    call(
        "submit_epic",
        json!({"edits": tasks.into_iter().map(add).collect::<Vec<_>>()}),
    )
}

/// A sub-planner's `submit_epic` adding `tasks` that must be refused.
pub fn submit_epic_err(tasks: Vec<Value>) -> Value {
    call_err(
        "submit_epic",
        json!({"edits": tasks.into_iter().map(add).collect::<Vec<_>>()}),
    )
}

/// An S `check`-mode task owning `file`, after `deps`.
pub fn epic_task(id: &str, file: &str, deps: &[&str]) -> Value {
    plan_task(id, &[file], json!({"deps": deps}))
}

/// The first sub-planner of `epic`: `get_context`, then `steps`.
pub fn planner(h: &RunHarness, epic: &str, steps: &[Value]) {
    let mut all = vec![call("get_context", json!({}))];
    all.extend_from_slice(steps);
    h.script(&format!("planner-{epic}-1"), &all);
}

/// The green worker and reviewer of task `id`, and an approving reviewer for round `n`
/// of each epic of `epics` (`reviewer-<e>-int<n>-1`).
pub fn green_with_integration(h: &RunHarness, tasks: &[(&str, &str)], epics: &[&str]) {
    for (id, file) in tasks {
        green(h, id, file);
    }
    for epic in epics {
        h.script(&format!("reviewer-{epic}-int1-1"), &[approve()]);
    }
}

/// The two pointers of both planners' states, `finished`, one `until` each (a JSON
/// pointer has no wildcard).
pub fn until_planners_finished(n: usize) -> Vec<Value> {
    (0..n)
        .map(|i| {
            until(
                &format!("/planners/{i}/state"),
                json!("finished"),
                crate::support::run_orch::ORCH_WAIT,
            )
        })
        .collect()
}
