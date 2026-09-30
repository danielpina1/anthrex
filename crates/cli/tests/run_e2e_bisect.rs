//! Milestone 9.1 task M9.1.22: a red tier 3 end to end, through the real binary and a
//! real daemon on `/tmp` paths: bisected to its culprit and fixed by the engine's fix
//! task, or, with no single culprit, raised with the orchestrator, which plans the fix.
//! `fake-agent` plays every agent (`support/run_tiers.rs`'s repository and scripts).

mod support;

use proto::{AgentRole, RunState, TaskOrigin, TaskState};
use serde_json::{Value, json};
use support::orch_script::{
    add, edit_plan, marker, passed, plan_task, prompt, until as status_until,
};
use support::run_harness::RunHarness;
use support::run_orch::{ORCH_LINES, triage_plan};
use support::run_plans::*;
use support::run_tiers::*;

fn green(h: &RunHarness, id: &str, steps: &[Value]) {
    h.script(&format!("worker-{id}-1"), steps);
    h.script(&format!("reviewer-{id}-1"), &[approve()]);
}

/// The repository's `history.jsonl` lines of type `bisect`.
fn bisect_lines(h: &RunHarness) -> Vec<Value> {
    let path = h.repo_dir().join(daemon::run::engine::HISTORY_FILE);
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|l| l["type"] == "bisect")
        .collect()
}

/// Whether a logged line ran in the run's tier-3 checkout, `.full`.
fn in_full(line: &Value) -> bool {
    line["cwd"].as_str().is_some_and(|c| c.ends_with("/.full"))
}

#[test]
fn e2e_red_tier3_is_bisected_to_the_culprit_and_fixed() {
    let h = RunHarness::with_config("", "", &tier_repo_files());
    green(&h, "t1", &[commit("mods/a/src.txt", "a2\n"), done("a")]);
    // Only the whole suite (and the single test `b::full`) reads the marker: t2's
    // tier 1 and tier 2 run b's module tests, which pass.
    green(&h, "t2", &[commit("mods/b/FULL_FAIL", "x\n"), done("b")]);
    green(&h, "t3", &[commit("mods/c/src.txt", "c2\n"), done("c")]);
    green(
        &h,
        "fix1",
        &[
            sh("git rm -q mods/b/FULL_FAIL && git commit -qm 'drop the marker'"),
            done("fixed b::full"),
        ],
    );
    let tasks = [
        task("t1", &["mods/a/src.txt"], ""),
        task("t2", &["mods/b/FULL_FAIL"], "deps = [\"t1\"]"),
        task("t3", &["mods/c/src.txt"], "deps = [\"t2\"]"),
    ];
    let id = h.start(&tier_plan(&h, "", &tasks), true);
    // Fail fast: the red tier 3 either adds fix1 or ends with an attention line. Three
    // task paths come first: t1, t2, t3.
    let run = h.wait_run(
        &id,
        |r| r.tasks.iter().any(|t| t.id == "fix1") || !r.attention.is_empty() || complete(r),
        3 * TIER_WAIT,
    );
    assert!(
        run.tasks.iter().any(|t| t.id == "fix1"),
        "no fix task: {:?}",
        run.attention
    );
    // Then fix1's own path.
    let run = h.wait_run(&id, complete, TIER_WAIT);

    let fix = t(&run, "fix1");
    assert_eq!(fix.state, TaskState::Merged);
    assert_eq!(fix.origin, TaskOrigin::Bisect);
    assert_eq!(fix.fixes.as_deref(), Some("bisect of t2"));
    let t2 = t(&run, "t2");
    assert_eq!(fix.owns, t2.owns, "the culprit's owns, copied exactly");
    assert_ne!(fix.route, t2.route, "the culprit's route, one rung up");
    assert!(
        fix.rounds.iter().any(|r| r.role == AgentRole::Worker),
        "fix1's worker ran"
    );

    let log = tier_log(&h);
    let full: Vec<_> = log.iter().filter(|l| in_full(l)).collect();
    let probes: Vec<_> = full
        .iter()
        .filter(|l| args(l) == ["--one", "b::full"])
        .collect();
    assert!(!probes.is_empty(), "the bisect probed b::full: {full:#?}");
    assert!(
        runs_of(&log, "check.sh")
            .iter()
            .filter(|l| in_full(l))
            .count()
            >= 2,
        "tier 3 ran red, then green after the fix: {full:#?}"
    );

    let bisects = until("the bisect history line", TIER_WAIT, || {
        let lines = bisect_lines(&h);
        (!lines.is_empty()).then_some(lines)
    });
    assert_eq!(bisects.len(), 1, "{bisects:#?}");
    let line = &bisects[0];
    assert_eq!(
        (&line["stage"], &line["culprit"], &line["fix_task"]),
        (&json!(1), &json!("t2"), &json!("fix1")),
        "{line}"
    );
    assert_eq!(line["tests"], json!(["b::full"]), "{line}");
}

/// A tiered stored profile for the orchestrated run, logging to `log`.
fn stored_tier_profile(log: &std::path::Path) -> String {
    format!(
        "check = \"sh check.sh {{filter:--filter %}}\"\n\
         check_timeout_secs = 10\n\
         modules = [\"mods/*\"]\n\
         module_graph = \"sh graph.sh\"\n\
         module_names = \"dir\"\n\
         build_check = \"sh build.sh\"\n\
         module_test = \"sh test.sh {{module}} {{filter:--filter %}}\"\n\
         single_test = \"sh test.sh --one {{test}}\"\n\
         test_passed = \"PASS {{test}}\"\n\
         source = [\"mods/**\"]\n\
         [env]\n\
         TIER_LOG = {:?}\n",
        log.display().to_string()
    )
}

const ORCH: &str = "orchestrator-run-1";

#[test]
fn e2e_red_tier3_without_a_culprit_wakes_the_orchestrator() {
    let h = RunHarness::adapt("claude", ORCH_LINES, &[], &tier_repo_files());
    h.stored_profile(&stored_tier_profile(&tier_log_path(&h)));
    h.decider("triage", 1, triage_plan());
    // Each marker alone is harmless; together they fail the whole suite, and the test
    // `pair::both` passes alone: no single merge is the culprit.
    green(&h, "t1", &[commit("mods/a/PAIR", "x\n"), done("a")]);
    green(&h, "t2", &[commit("mods/c/PAIR", "x\n"), done("c")]);
    green(
        &h,
        "t3",
        &[
            sh("git rm -q mods/c/PAIR && git commit -qm 'one marker only'"),
            done("removed c's marker"),
        ],
    );
    let note = "stage 1 tier 3 red, no single culprit: pair::both; plan a fix";
    let long = 3 * TIER_WAIT;
    let steps = [
        prompt(),
        edit_plan(
            vec![
                add(plan_task("t1", &["mods/a/PAIR"], json!({}))),
                add(plan_task("t2", &["mods/c/PAIR"], json!({"deps": ["t1"]}))),
            ],
            json!({"submit": true}),
        ),
        status_until("/gate/state", json!("approved"), long),
        // Idle, it is woken with the note (a `run_status` call would have shown the
        // same attention line and taken the note as seen).
        json!({"read_message": {"expect": note}}),
        // The orchestrator plans the fix with an ordinary edit.
        edit_plan(
            vec![add(plan_task("t3", &["mods/c/PAIR"], json!({})))],
            json!({}),
        ),
        marker(),
        json!({"read_message": {}}),
        json!({"read_message": {}}),
        json!({"read_message": {}}),
    ];
    h.script(ORCH, &steps);
    let id = h.start_goal_id("pair the modules", &[]);
    h.orchestrator_window(&id);
    h.wait_run(&id, |r| r.state == RunState::AwaitingApproval, long);
    let out = h.anthrex(&["run", "approve", &id]);
    assert!(out.status.success(), "{out:?}");

    // Fail fast: an orchestrator whose script missed the note exits (its
    // `read_message` expectation fails), and the run would wait for a fix forever.
    let run = h.wait_run(
        &id,
        |r| complete(r) || r.orchestrator.as_ref().is_some_and(|o| !o.live),
        4 * TIER_WAIT,
    );
    assert_eq!(run.state, RunState::Complete, "{:?}", run.attention);
    assert_eq!(t(&run, "t3").state, TaskState::Merged);
    assert!(
        run.tasks.iter().all(|t| t.origin == TaskOrigin::Plan),
        "no engine fix task: {:?}",
        run.tasks
            .iter()
            .map(|t| (&t.id, t.origin))
            .collect::<Vec<_>>()
    );
    assert!(
        passed(&h.mcp_log(), ORCH) >= 1,
        "the script passed its steps"
    );
    // The script read the note before it added t3, so it is on record by now.
    let read = h.read_messages(ORCH);
    let wake = read
        .iter()
        .find_map(|m| m["text"].as_str().filter(|t| t.contains(note)))
        .unwrap_or_else(|| panic!("no wake note: {read:#?}"));
    assert!(
        wake.starts_with(&format!("[anthrex] Run {id} changed: ")),
        "{wake}"
    );
    let bisects = bisect_lines(&h);
    assert_eq!(bisects.len(), 1, "{bisects:#?}");
    assert_eq!(bisects[0]["culprit"], Value::Null);
    assert_eq!(bisects[0]["fix_task"], Value::Null);
}
