//! Milestone 9.1 task M9.1.22: a red tier 3 end to end, through the real binary and a
//! real daemon on `/tmp` paths: bisected to its culprit and fixed by the engine's fix
//! task, or, with no single culprit, raised with the orchestrator, which plans the fix.
//! `fake-agent` plays every agent (`support/run_tiers.rs`'s repository and scripts).

mod support;

use proto::{AgentRole, FullState, ModelEntry, RunInfo, RunState, TaskOrigin, TaskState};
use serde_json::{Value, json};
use support::orch_script::{
    add, edit_plan, marker, passed, plan_task, prompt, until as status_until,
};
use support::run_harness::{REQUEST_WAIT, RunHarness};
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

/// The run completed, or cannot: it stopped running (halted, for one), or a task
/// blocked.
fn settled(r: &RunInfo) -> bool {
    !matches!(
        r.state,
        RunState::Running | RunState::AwaitingApproval | RunState::Planning
    ) || r.tasks.iter().any(|t| t.state == TaskState::Blocked)
}

/// Waits, at most `REQUEST_WAIT`, for the bisect's history line (its effect is an
/// append the engine emits with the end of the bisect).
fn the_bisect_line(h: &RunHarness) -> Value {
    let lines = until("the bisect history line", REQUEST_WAIT, || {
        let lines = bisect_lines(h);
        (!lines.is_empty()).then_some(lines)
    });
    assert_eq!(lines.len(), 1, "{lines:#?}");
    lines[0].clone()
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
        |r| r.tasks.iter().any(|t| t.id == "fix1") || !r.attention.is_empty() || settled(r),
        3 * TIER_WAIT,
    );
    assert!(
        run.tasks.iter().any(|t| t.id == "fix1"),
        "no fix task: {:?}",
        run.attention
    );
    // Checked at once (ruling C-26, 2): a bisect that blamed the wrong merge fails
    // here, not a `TIER_WAIT` later.
    let line = the_bisect_line(&h);
    assert_eq!(
        (&line["stage"], &line["culprit"], &line["fix_task"]),
        (&json!(1), &json!("t2"), &json!("fix1")),
        "{line}"
    );
    assert_eq!(line["tests"], json!(["b::full"]), "{line}");
    // Decision 36's probes, in order: the base, the head, then the halving of the
    // three merges (t1's green, t2's red). The head is t3's merge, so t2's is `~1` and
    // t1's `~2` on the first-parent line. Decision 33 retries a red run once by name,
    // so the tier-3 check's red retry comes first, and each red probe runs twice.
    let head = line["head"].as_str().expect("the bisected head");
    let base = run_json(&run)["base_sha"].as_str().unwrap().to_string();
    let at = |rev: &str| h.git(&["rev-parse", &format!("{head}{rev}")]);
    let probed: Vec<String> = tier_log(&h)
        .iter()
        .filter(|l| in_full(l) && args(l) == ["--one", "b::full"])
        .map(|l| l["head"].as_str().unwrap_or_default().to_string())
        .collect();
    let (head, t1, t2) = (head.to_string(), at("~2"), at("~1"));
    assert_eq!(
        probed,
        [&head, &base, &head, &head, &t1, &t2, &t2].map(String::clone),
        "the single-test runs in .full"
    );
    assert_eq!(line["probes"], json!(4), "{line}");
    let (fix, t2) = (t(&run, "fix1"), t(&run, "t2"));
    assert_eq!(fix.owns, t2.owns, "the culprit's owns, copied exactly");
    // Decision 37: the culprit's route one rung up (`roster::escalate`), from the
    // run's own roster.
    let roster: Vec<ModelEntry> =
        serde_json::from_value(run_json(&run)["roster"].clone()).expect("run.json's roster");
    let up = daemon::run::roster::escalate(&roster, &t2.route);
    assert_ne!(up, t2.route, "the harness's route has a rung above it");
    assert_eq!(fix.route, up, "the culprit's route, one rung up");
    // Then fix1's own path.
    let run = h.wait_run(&id, settled, TIER_WAIT);
    assert_eq!(run.state, RunState::Complete, "{}", report(&run));

    let fix = t(&run, "fix1");
    assert_eq!(fix.state, TaskState::Merged);
    assert_eq!(fix.origin, TaskOrigin::Bisect);
    assert_eq!(fix.fixes.as_deref(), Some("bisect of t2"));
    for stage in &run.stages {
        assert_eq!(stage.full.state, FullState::Green, "{stage:?}");
        assert_eq!(stage.full.commit, stage.head, "{stage:?}");
    }
    assert!(
        fix.rounds.iter().any(|r| r.role == AgentRole::Worker),
        "fix1's worker ran"
    );

    let log = tier_log(&h);
    let full: Vec<_> = log.iter().filter(|l| in_full(l)).collect();
    assert!(
        runs_of(&log, "check.sh")
            .iter()
            .filter(|l| in_full(l))
            .count()
            >= 2,
        "tier 3 ran red, then green after the fix: {full:#?}"
    );
    assert_eq!(bisect_lines(&h).len(), 1, "one bisect only");
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
    // The plan gate, and the note: t1's and t2's paths one after the other, then
    // tier 3 and the bisect's probes inside a third `TIER_WAIT` (ruling C-26, 1).
    let long = 3 * TIER_WAIT;
    let long_ms = u64::try_from(long.as_millis()).unwrap();
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
        // Bounded (ruling C-26, 2): a note that never comes fails the script.
        json!({"read_message": {"expect": note, "timeout_ms": long_ms}}),
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
    let gone = |r: &RunInfo| r.orchestrator.as_ref().is_some_and(|o| !o.live);
    let run = h.wait_run(
        &id,
        |r| settled(r) || gone(r) || !bisect_lines(&h).is_empty(),
        long,
    );
    assert!(
        !bisect_lines(&h).is_empty(),
        "no bisect ended: {}",
        report(&run)
    );
    // The bisect has ended without a culprit: the idle orchestrator is woken within
    // one engine step, the driver's tick and `wake_quiet_secs` (1 s).
    until("the wake note", REQUEST_WAIT, || {
        h.read_messages(ORCH)
            .iter()
            .any(|m| m["text"].as_str().is_some_and(|t| t.contains(note)))
            .then_some(())
    });
    // Then t3's path and tier 3 again.
    let run = h.wait_run(&id, |r| settled(r) || gone(r), TIER_WAIT);
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
    let line = &bisects[0];
    assert_eq!(line["culprit"], Value::Null);
    assert_eq!(line["fix_task"], Value::Null);
    // The no-culprit branch this scenario is built for (ruling C-26, 4): the failing
    // test passes alone at the head, which holds both markers.
    let head = line["head"].as_str().expect("the bisected head");
    for marker in ["mods/a/PAIR", "mods/c/PAIR"] {
        h.git(&["cat-file", "-e", &format!("{head}:{marker}")]);
    }
    assert_eq!(
        line["reason"],
        json!(format!(
            "the failing tests pass alone at {}; they fail only with the whole suite",
            &head[..7]
        )),
        "{line}"
    );
}
