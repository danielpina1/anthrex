//! Milestone 9.8 end to end (MR §7, §8): the role table through a real daemon with
//! `fake-agent` as the runtimes it finds. No test reaches a real agent: every runtime
//! command is `fake-agent` or a path that does not exist.
//!
//! `each_role_launches_its_configured_model` and
//! `the_design_flow_launches_the_brainstorm_and_reviewer_rows` are the milestone's
//! acceptance tests (M9.8.15, ruling F39): written after every launcher change, they
//! pin what tasks M9.8.7a to M9.8.14 built, as M9.8.1's golden test pinned what came
//! before. They passed on their first run.

mod support;

use proto::{BlockReason, DocGateKind, RunState, TaskState};
use serde_json::{Value, json};
use support::orch_script::{
    add, call, edit_plan, expect, marker, passed, plan_task, prompt, read, until,
};
use support::run_adapt::{STORED_PROFILE, hang};
use support::run_design::{
    DESIGN_LINES, Draft, Findings, OrchDesign, ask, brainstormer_name, merge, reviewer_name,
    spec_for_review,
};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_orch::ORCH_WAIT;
use support::run_plans::{approve, capture, commit, done, done_tdd, plan, t, task};

/// MR §7: a role whose runtime's CLI is missing fails its launch naming the role and
/// the way out. The reviewer row is the Codex default and Codex's command does not
/// exist: the worker (the small row, Claude) finishes green, and the review's start
/// blocks the task with `reviewer: codex not found; choose another model in C-b S`.
#[test]
fn a_missing_cli_names_the_role() {
    let codex = ("ANTHREX_CODEX_BIN", "/nonexistent/anthrex-test/codex");
    let reviewer = "[models.reviewer]\nmodel = \"codex:default\"\n";
    let h = RunHarness::with_env_and_config("", &[codex], reviewer);
    h.script(
        "worker-t1-1",
        &[commit("a.rs", "fn a() {}\n"), done("added a")],
    );
    let id = h.start(&plan("", &[task("t1", &["a.rs"], "")]), true);
    let blocked = |r: &proto::RunInfo| t(r, "t1").state == TaskState::Blocked;
    let run = h.wait_run(&id, blocked, RUN_WAIT);
    let t1 = t(&run, "t1");
    let block = t1.block.as_ref().expect("a block");
    assert_eq!(block.reason, BlockReason::Environment, "{block:?}");
    let want = "reviewer: codex not found; choose another model in C-b S";
    assert!(block.text.contains(want), "{block:?}");
    assert!(t1.rounds.iter().any(|r| r.role == proto::AgentRole::Worker));
}

/// MR §8's table (M9.8.15), every row naming a distinct model, plus the hub row
/// (ruling F21).
const MODELS: &str = r#"[models.orchestrator]
model = "claude:fake-orchestrator"
effort = "high"
[models.planner]
model = "claude:fake-planner"
[models.implementer.small]
model = "codex:fake-small"
effort = "low"
[models.implementer.medium]
model = "claude:fake-medium"
effort = "medium"
fallback = "codex:fake-medium-fallback"
[models.implementer.hub]
model = "claude:fake-hub"
[models.test_writer]
model = "codex:fake-writer"
[models.reviewer]
model = "codex:fake-reviewer"
[models.research]
model = "claude:fake-research"
[models.helpers]
model = "claude:fake-helpers"
[models.helpers.run_name]
model = "claude:fake-run-name"
[models.brainstorm]
first = "claude:fake-brainstorm-a"
second = "codex:fake-brainstorm-b"
"#;

/// The orchestrator's script: the first orchestrator session of the run.
const ORCH: &str = "orchestrator-run-1";

/// The test the hub task's writer writes, and its two versions.
const TEST: &str = "t_feat";
const FAILING: &str = "exit 1\n";
const PASSING: &str = "echo PASS t_feat\n";

/// The first argv `fake-agent` recorded for the session claimed as `name`.
fn argv(h: &RunHarness, name: &str) -> Vec<String> {
    let lines = h.io_lines(name, "args");
    let first = lines.first().unwrap_or_else(|| {
        panic!("{name} never started\n{}", h.log_tail());
    });
    serde_json::from_str(first).unwrap()
}

/// Whether `argv` holds `flag` immediately followed by `value`.
fn has(argv: &[String], flag: &str, value: &str) -> bool {
    argv.windows(2).any(|w| w[0] == flag && w[1] == value)
}

/// Asserts a Claude session's argv names `model` and, when given, `--effort <effort>`.
fn claude_runs(argv: &[String], model: &str, effort: Option<&str>) {
    assert!(has(argv, "--model", model), "--model {model}: {argv:?}");
    match effort {
        Some(e) => assert!(has(argv, "--effort", e), "--effort {e}: {argv:?}"),
        None => assert!(!argv.iter().any(|a| a == "--effort"), "{argv:?}"),
    }
}

/// Asserts a Codex session's argv names `model` (`-m`) and, when given, its effort
/// (`-c model_reasoning_effort="<effort>"`); with none, no effort at all.
fn codex_runs(argv: &[String], model: &str, effort: Option<&str>) {
    assert!(has(argv, "-m", model), "-m {model}: {argv:?}");
    let efforts: Vec<&String> = (argv.iter())
        .filter(|a| a.starts_with("model_reasoning_effort="))
        .collect();
    match effort {
        Some(e) => assert_eq!(
            efforts,
            [&format!("model_reasoning_effort=\"{e}\"")],
            "{argv:?}"
        ),
        None => assert!(efforts.is_empty(), "{argv:?}"),
    }
}

/// The `argv` of every recorded decider call of `kind`.
fn decider_argvs(h: &RunHarness, kind: &str) -> Vec<Vec<String>> {
    let calls = match kind {
        "run_name" => h.run_name_calls(),
        _ => h.decider_calls(),
    };
    (calls.iter())
        .filter(|c| c["kind"] == kind)
        .map(|c| serde_json::from_value(c["argv"].clone()).unwrap())
        .collect()
}

/// A research task with no `owns` (decision 24).
fn research_task(id: &str) -> Value {
    json!({
        "id": id, "title": format!("Task {id}"), "brief": format!("Do {id}."),
        "acceptance": [format!("{id} is done")], "owns": [], "size": "S",
        "kind": "research",
    })
}

/// MR §8: a run whose rows name distinct fake models launches each role with its
/// configured model, read from the argv `fake-agent` recorded. A sub-planner adds the
/// `S` task `t1` to epic `a` (ruling F21: the planner row); the orchestrator adds the
/// racing `M` task `t2`, the paired hub task `t3` and the research task `t4`.
///
/// Escalation and the racer as tasks M9.8.7a and M9.8.11 defined them: the racer runs
/// the row's fallback at its default effort (no effort flag); the medium row's model
/// runs at the row's effort.
#[test]
fn each_role_launches_its_configured_model() {
    let h = RunHarness::orch_with_config("", &[], MODELS);
    h.stored_profile(&format!("{STORED_PROFILE}hub = [\"tests/**\"]\n"));
    h.script(
        "planner-a-1",
        &[
            call("get_context", json!({})),
            call(
                "submit_epic",
                json!({"edits": [add(plan_task("t1", &["a.txt"], json!({})))]}),
            ),
        ],
    );
    h.script(
        "worker-t1-1",
        &[commit("a.txt", "a\n"), done("added a.txt")],
    );
    h.script(
        "racer-t2-a-1",
        &[commit("b.txt", "b\n"), done("added b.txt in lane a")],
    );
    h.script("racer-t2-b-1", &[hang()]);
    h.script(
        "test_writer-t3-1",
        &[
            commit(&format!("tests/{TEST}.sh"), FAILING),
            capture("red", "git rev-parse HEAD"),
            done_tdd(TEST, "{{red}}"),
        ],
    );
    h.script(
        "worker-t3-1",
        &[commit(&format!("tests/{TEST}.sh"), PASSING), done("green")],
    );
    h.script(
        "scout-t4-1",
        &[call(
            "submit_scout_report",
            json!({"summary": "check.sh is the whole check.",
                "files": [{"path": "check.sh", "why": "the check"}]}),
        )],
    );
    for reviewer in ["reviewer-t1-1", "reviewer-t2-1", "reviewer-t3-1"] {
        h.script(reviewer, &[approve()]);
    }
    h.script("reviewer-a-int1-1", &[approve()]);

    let racing = json!({"size": "M", "race": true});
    let paired = json!({"size": "M", "pair": true, "test_to_write": TEST,
        "test_mode": "tdd", "test_mode_reason": null});
    let steps = [
        prompt(),
        call(
            "spawn_subplanner",
            json!({"epic": "a", "title": "Epic a", "area": ["a.txt"],
                "brief": "Plan epic a."}),
        ),
        until("/planners/0/state", json!("finished"), ORCH_WAIT),
        edit_plan(
            vec![
                add(plan_task("t2", &["b.txt"], racing)),
                add(plan_task("t3", &["tests/**"], paired)),
                add(research_task("t4")),
            ],
            json!({"submit": true}),
        ),
        expect("/awaiting_approval", json!(true)),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        until("/run/complete", json!(true), RUN_WAIT * 3),
        marker(),
        read(None),
    ];
    h.script(ORCH, &steps);
    let run = h.start_goal_id("add the files", &[]);
    h.orchestrator_window(&run);
    h.wait_run(&run, |r| r.state == RunState::AwaitingApproval, ORCH_WAIT);
    let out = h.anthrex(&["run", "approve", &run]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT * 3);
    for (id, state) in [
        ("t1", TaskState::Merged),
        ("t2", TaskState::Merged),
        ("t3", TaskState::Merged),
        ("t4", TaskState::Reported),
    ] {
        assert_eq!(t(&info, id).state, state, "{id}\n{}", h.log_tail());
    }
    assert!(t(&info, "t3").hub, "t3 owns the hub");
    h.wait_log(
        "the orchestrator's script to pass its expectations",
        |log| passed(log, ORCH) >= 1,
        RUN_WAIT,
    );

    // The orchestrator in its window, at its row's effort.
    claude_runs(&argv(&h, ORCH), "fake-orchestrator", Some("high"));
    // The helpers: the run name on its own kind's row, triage on the helpers row.
    let run_names = decider_argvs(&h, "run_name");
    assert_eq!(run_names.len(), 1, "{run_names:?}");
    claude_runs(&run_names[0], "fake-run-name", None);
    let triages = decider_argvs(&h, "triage");
    assert!(!triages.is_empty(), "no triage call");
    for argv in &triages {
        claude_runs(argv, "fake-helpers", None);
    }
    // The sub-planner, on the planner row.
    claude_runs(&argv(&h, "planner-a-1"), "fake-planner", None);
    // The S worker: Codex, the small row's model and effort.
    codex_runs(&argv(&h, "worker-t1-1"), "fake-small", Some("low"));
    // The M task's racers: lane a on the medium row; lane b on its fallback, at that
    // model's default effort (decision 28, M9.8.7a).
    claude_runs(&argv(&h, "racer-t2-a-1"), "fake-medium", Some("medium"));
    codex_runs(&argv(&h, "racer-t2-b-1"), "fake-medium-fallback", None);
    // The hub task: its test writer on the test_writer row, its worker on the hub row.
    codex_runs(&argv(&h, "test_writer-t3-1"), "fake-writer", None);
    claude_runs(&argv(&h, "worker-t3-1"), "fake-hub", None);
    // The research task's scout, on the research row.
    claude_runs(&argv(&h, "scout-t4-1"), "fake-research", None);
    // Every reviewer, the epic's integration review's too, on the reviewer row.
    for reviewer in [
        "reviewer-t1-1",
        "reviewer-t2-1",
        "reviewer-t3-1",
        "reviewer-a-int1-1",
    ] {
        codex_runs(&argv(&h, reviewer), "fake-reviewer", None);
    }
}

/// MR §8 for the 9.6 design flow: the two brainstormers run the `brainstorm` row's two
/// models, and the spec's document reviewer the `reviewer` row's.
#[test]
fn the_design_flow_launches_the_brainstorm_and_reviewer_rows() {
    let h = RunHarness::orch_with_config(DESIGN_LINES, &[], MODELS);
    // The brainstormers are labelled by runtime (9.6 decision 10).
    let labels = ["claude", "codex"];
    for label in labels {
        h.brainstormer(label, 1, Draft::Fixture);
    }
    h.reviewer("spec", 1, 1, Findings::Submits(vec![]));
    let design = OrchDesign::fixture(labels);
    let mut steps = ask(design.answer.as_deref());
    steps.extend(merge(&design.labels, &design.report));
    steps.push(support::run_design::approved("brainstorm"));
    steps.extend(spec_for_review(&design.spec_draft, 1));
    steps.extend([marker(), read(None)]);
    h.script(ORCH, &steps);
    let run = h.start_goal_id("add a.txt", &[]);
    let window = h.orchestrator_window(&run);
    h.answer_question(ORCH, window);
    h.approve_doc(&run, DocGateKind::Brainstorm, 1);
    h.wait_log(
        "the orchestrator's script to pass its expectations",
        |log| passed(log, ORCH) >= 2,
        RUN_WAIT,
    );

    claude_runs(
        &argv(&h, &brainstormer_name("claude", 1)),
        "fake-brainstorm-a",
        None,
    );
    codex_runs(
        &argv(&h, &brainstormer_name("codex", 1)),
        "fake-brainstorm-b",
        None,
    );
    codex_runs(
        &argv(&h, &reviewer_name("spec", 1, 1)),
        "fake-reviewer",
        None,
    );
}
