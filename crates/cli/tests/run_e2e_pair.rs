//! Milestone 9.5 task M9.5.22: the test writer, then the implementer, end to end
//! (decisions 24 to 26, spec §21), through a real daemon with `fake-agent` as both
//! runtimes. The task's route is Claude, so its test writer runs on Codex. A red commit
//! that fails is handed to the implementer; a red that passes goes back to the writer.

mod support;

use proto::{AgentRole, PairPhase, Runtime, TaskInfo, TaskState};
use serde_json::Value;
use support::run_adapt::TUNING_PROFILE;
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

const TEST: &str = "t_reset";

/// The test writer's commit of `tests/t_reset.sh` with `body`.
fn write_test(body: &str) -> Value {
    commit(&format!("tests/{TEST}.sh"), body)
}

/// The writer's red test (the brief's): it exits 1.
const FAILING: &str = "exit 1\n";
/// A test that passes: the implementer's version, and a red that checks nothing.
const PASSING: &str = "echo PASS t_reset\n";

/// The harness, its stored profile naming `tests/**` the hub (a stored profile
/// replaces a plan's `[profile]` entirely, M8b decision 6).
fn harness() -> RunHarness {
    let h = RunHarness::tuning("", "", &[]);
    h.stored_profile(&format!("{TUNING_PROFILE}hub = [\"tests/**\"]\n"));
    h
}

/// The plan: one hub M `tdd` task `t1` on a Claude route, paired, writing `t_reset`.
/// The route names strength `standard`: a hub's default is `frontier`, where the
/// built-in roster has no Codex model, so the writer would stay on Claude (decision
/// 25's fallback to the task's route).
fn pair_plan() -> String {
    let paired = task(
        "t1",
        &["tests/**"],
        &format!("pair = true\ntest_to_write = \"{TEST}\"\nroute = {{ runtime = \"claude\", strength = \"standard\" }}"),
    )
    .replace("size = \"S\"", "size = \"M\"")
    .replace(
        "test_mode = \"check\"\ntest_mode_reason = \"smoke\"\n",
        "test_mode = \"tdd\"\n",
    );
    format!("goal = \"Pair\"\n{paired}")
}

/// The implementer (the brief's): it commits a `tests/t_reset.sh` that prints `PASS
/// t_reset` and calls `task_done` with neither test nor red (the pair fills them).
fn implementer(h: &RunHarness) {
    h.script("worker-t1-1", &[write_test(PASSING), done("green")]);
}

fn roles(task: &TaskInfo) -> Vec<AgentRole> {
    task.rounds.iter().map(|r| r.role).collect()
}

/// Every turn message the session claimed as `name` got: Claude's stream-json user
/// messages and Codex's argv messages.
fn prompts(h: &RunHarness, name: &str) -> Vec<String> {
    let mut all = user_texts(&h.io_lines(name, "stdin"));
    all.extend(h.codex_messages(name));
    all
}

#[test]
fn e2e_test_writer_red_commit_is_handed_to_an_implementer() {
    let h = harness();
    h.script(
        "test_writer-t1-1",
        &[
            write_test(FAILING),
            capture("red", "git rev-parse HEAD"),
            json_done("red", TEST, "{{red}}"),
        ],
    );
    implementer(&h);
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&pair_plan(), true);
    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert!(t1.hub, "t1 owns the hub");
    assert_eq!(
        roles(t1),
        [
            AgentRole::TestWriter,
            AgentRole::Worker,
            AgentRole::Reviewer
        ]
    );
    let routes: Vec<_> = t1.rounds.iter().map(|r| r.route.clone()).collect();
    assert_eq!(
        (routes[0].runtime, routes[1].runtime),
        (Runtime::Codex, Runtime::Claude),
        "the writer on Codex, the implementer on the task's Claude route: {routes:?}, task {:?}, pair {:?}",
        t1.route,
        t1.pair
    );
    let pair = t1.pair.as_ref().expect("the pair");
    assert_eq!(pair.phase, PairPhase::Implementing);
    assert_eq!(pair.writer_route.runtime, Runtime::Codex);
    assert_eq!(pair.red_checked, Some(true));
    let red = pair.red.clone().expect("the red commit");
    let red7 = &red[..7];

    // The implementer's first turn names the red commit and its writer.
    let first = prompts(&h, "worker-t1-1");
    let needle = format!("was committed in {red7} by a separate test writer");
    assert!(
        first.first().is_some_and(|p| p.contains(&needle)),
        "{needle:?} in the implementer's first turn: {first:#?}"
    );
    // The proof the task merged on ran from the writer's red.
    let proof = t1.last_proof.as_ref().expect("the proof");
    assert_eq!(proof.red, red);
    assert!(proof.ok, "{proof:?}");
    // The reviewer is told who wrote the test.
    let reviewer = prompts(&h, "reviewer-t1-1");
    let note = format!("A separate test writer committed the test {TEST}");
    assert!(
        reviewer.iter().any(|p| p.contains(&note)),
        "{note:?} in the reviewer's prompt: {reviewer:#?}"
    );
}

#[test]
fn e2e_a_red_that_passes_goes_back_to_the_test_writer() {
    let h = harness();
    h.script(
        "test_writer-t1-1",
        &[
            write_test(PASSING),
            capture("red", "git rev-parse HEAD"),
            json_done("red", TEST, "{{red}}"),
            read("[anthrex] The red check failed"),
            write_test(FAILING),
            capture("red", "git rev-parse HEAD"),
            json_done("red again", TEST, "{{red}}"),
        ],
    );
    implementer(&h);
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&pair_plan(), true);
    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    let pair = t1.pair.as_ref().expect("the pair");
    assert_eq!(pair.red_checked, Some(true));
    assert_eq!(pair.writer_failures, 1, "{pair:?}");
    let red = pair.red.clone().expect("the second red");

    // The writer was sent back with the first red, which passed.
    let writer = prompts(&h, "test_writer-t1-1");
    let bounces: Vec<&String> = (writer.iter())
        .filter(|p| p.starts_with("[anthrex] The red check failed: at your red commit "))
        .collect();
    assert_eq!(bounces.len(), 1, "{writer:#?}");
    assert!(
        bounces[0].contains(&format!("the test {TEST} passed")),
        "{}",
        bounces[0]
    );
    // The second red sits directly on the first (review m4): the bounce names that
    // parent, and not the second red.
    let first_red = h.git(&["rev-parse", &format!("{red}^")]);
    assert!(
        bounces[0].contains(&format!("at your red commit {} ", &first_red[..7])),
        "the bounce names the first red {first_red}: {}",
        bounces[0]
    );
    assert!(
        !bounces[0].contains(&red[..7]),
        "the bounce names the first red, not the second: {}",
        bounces[0]
    );
    // Only the second red reached the implementer.
    let first = prompts(&h, "worker-t1-1");
    assert!(
        first.first().is_some_and(|p| p.contains(&red[..7])),
        "{first:#?}"
    );
    assert_eq!(
        roles(t1),
        [
            AgentRole::TestWriter,
            AgentRole::Worker,
            AgentRole::Reviewer
        ]
    );
}

/// `task_done` for the test writer: its summary, the test and the red.
fn json_done(summary: &str, test: &str, red: &str) -> Value {
    serde_json::json!({"mcp_call": {"tool": "task_done", "args": {"summary": summary, "test": test, "red": red}}})
}
