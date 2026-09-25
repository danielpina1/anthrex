//! Milestone 8a, task 25: the ladder (a stall to a fresh session on the peer runtime, a
//! mis-sized task split by an edit) and the merge queue (a conflict handed back and
//! resolved, a second conflict, a red candidate), through a real daemon with
//! `fake-agent` as both runtimes.

mod support;

use std::time::Duration;

use proto::{
    AgentRole, BlockReason, PlanEdit, PlanTask, RunReply, RunRequest, Runtime, Size, TaskState,
    TestMode,
};
use serde_json::{Value, json};
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

/// `hang`: the turn stays open, silent, until it is interrupted or killed.
fn hang() -> Value {
    json!({"hang": {}})
}

fn blocked(kind: &str, reason: &str) -> Value {
    json!({"mcp_call": {"tool": "task_blocked", "args": {"kind": kind, "reason": reason}}})
}

/// Waits with `sh` until `path` exists: a deadline loop of at most one `RUN_WAIT`
/// (1500 × 0.2 s = 300 s).
pub fn wait_for_file(path: &std::path::Path) -> Value {
    sh(&format!(
        "for i in $(seq 1 1500); do [ -e '{}' ] && exit 0; sleep 0.2; done; exit 1",
        path.display()
    ))
}

/// Waits with `sh` until the run branch has `task`'s merge (the brief's loop: 1500 ×
/// 0.2 s = 300 s = one `RUN_WAIT`).
fn wait_for_merge_of(task: &str) -> Value {
    sh(&format!(
        "for i in $(seq 1 1500); do git log --all --format=%s | grep -q 'anthrex: merge {task}' && exit 0; sleep 0.2; done; exit 1"
    ))
}

const STALL_AFTER_SECS: u64 = 5;

#[test]
fn e2e_stall_escalates_to_a_fresh_session_on_the_peer_runtime() {
    let h = RunHarness::new(&format!("stall_after_secs = {STALL_AFTER_SECS}"));
    h.script(
        "worker-t1-1",
        &[
            sh(
                "mkdir -p tests && printf 'test -f feature.txt || exit 1\\necho PASS t_feature\\n' > tests/t_feature.sh && git add -A && git commit -qm 'add t_feature'",
            ),
            hang(),
            read("interrupted after"),
            hang(),
        ],
    );
    h.script(
        "worker-t1-2",
        &[
            capture("red", "git log --format=%H --grep='add t_feature' -n 1"),
            commit("feature.txt", "feature\n"),
            done_tdd("t_feature", "{{red}}"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let tdd = task(
        "t1",
        &["tests/**", "feature.txt"],
        "route = { runtime = \"claude\", strength = \"standard\", effort = \"high\" }",
    )
    .replace(
        "test_mode = \"check\"\ntest_mode_reason = \"smoke\"\n",
        "test_mode = \"tdd\"\n",
    );
    let plan = plan(
        "single_test = \"sh tests/{test}.sh\"\ntest_passed = \"PASS {test}\"",
        &[tdd],
    );
    let id = h.start(&plan, true);

    // Two task paths (k = 2) and the two stalls waited out on the way.
    let wait = 2 * RUN_WAIT + Duration::from_secs(2 * STALL_AFTER_SECS);
    let run = h.wait_run(&id, complete, wait);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.stalls, 1, "{:#?}", t1.rounds);

    let interrupts: Vec<Value> = h
        .io_lines("worker-t1-1", "stdin")
        .iter()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|v| v["type"] == "control_request" && v["request"]["subtype"] == "interrupt")
        .collect();
    assert_eq!(interrupts.len(), 1, "{interrupts:#?}");

    let fresh = t1
        .rounds
        .iter()
        .find(|r| r.role == AgentRole::Worker && r.session == 2)
        .expect("a second worker session");
    assert_eq!(fresh.route.runtime, Runtime::Codex);
    let prompts = h.codex_messages("worker-t1-2");
    let prompt = prompts.first().expect("worker-t1-2 started on Codex");
    assert!(
        prompt.contains("This is session 2 of this task."),
        "{prompt}"
    );
    assert!(prompt.contains("tests/t_feature.sh |"), "{prompt}");
}

/// `t2a`, `t2b`: the split's halves, with disjoint `owns`.
fn half(id: &str, owns: &str) -> PlanTask {
    PlanTask {
        id: id.into(),
        title: format!("Task {id}"),
        epic: None,
        kind: proto::TaskKind::Code,
        size: Size::S,
        interface_change: false,
        test_mode: Some(TestMode::Check),
        test_mode_reason: Some("smoke".into()),
        owns: vec![owns.into()],
        deps: vec![],
        priority: 0,
        brief: format!("Do {id}"),
        acceptance: vec![format!("{id} is done")],
        test_to_write: None,
        scout_refs: vec![],
        route: Default::default(),
        budget: None,
    }
}

#[test]
fn e2e_mis_sized_task_blocks_and_is_split_by_an_edit() {
    let h = RunHarness::new("");
    h.script("worker-t2-1", &[blocked("mis_sized", "two tasks in one")]);
    for (task, file) in [("t2a", "a.txt"), ("t2b", "b.txt"), ("t3", "c.txt")] {
        h.script(
            &format!("worker-{task}-1"),
            &[commit(file, "x\n"), done(task)],
        );
        h.script(&format!("reviewer-{task}-1"), &[approve()]);
    }
    let plan = plan(
        "",
        &[
            task("t2", &["a.txt", "b.txt"], ""),
            task("t3", &["c.txt"], "deps = [\"t2\"]"),
        ],
    );
    let id = h.start(&plan, true);

    let run = h.wait_run(&id, |r| t(r, "t2").state == TaskState::Blocked, RUN_WAIT);
    let t2 = t(&run, "t2");
    assert_eq!(
        t2.block.as_ref().map(|b| b.reason),
        Some(BlockReason::MisSized),
        "{:?}",
        t2.block
    );
    assert_eq!(t2.size, Size::M);
    assert_eq!(t(&run, "t3").state, TaskState::Pending);

    match h.request(RunRequest::Edit {
        run_id: id.clone(),
        edits: vec![PlanEdit::SplitTask {
            task_id: "t2".into(),
            into: vec![half("t2a", "a.txt"), half("t2b", "b.txt")],
        }],
    }) {
        RunReply::Done { .. } => {}
        other => panic!("the split: {other:?}"),
    }

    // Three task paths one after another: t2, then t2a and t2b, then t3.
    let run = h.wait_run(&id, complete, 3 * RUN_WAIT);
    for task in ["t2a", "t2b", "t3"] {
        assert_eq!(t(&run, task).state, TaskState::Merged, "{task}");
    }
    assert_eq!(t(&run, "t2").state, TaskState::Cancelled);
    let order = h.git(&[
        "log",
        "--reverse",
        "--merges",
        "--format=%s",
        &format!("anthrex/{id}/integration"),
    ]);
    let order: Vec<&str> = order.lines().collect();
    assert_eq!(order.len(), 3, "{order:?}");
    assert!(
        order[2].starts_with("anthrex: merge t3"),
        "t3 merged before one of t2a, t2b: {order:?}"
    );
    // t3 started only after both halves merged.
    let t3_started = t(&run, "t3")
        .rounds
        .iter()
        .map(|r| r.started_at)
        .min()
        .unwrap();
    let merged_at = |task: &str| {
        let sha = t(&run, task).merge_commit.clone().unwrap();
        h.git(&["log", "-1", "--format=%ct", &sha])
            .parse::<u64>()
            .unwrap()
    };
    assert!(t3_started >= merged_at("t2a") && t3_started >= merged_at("t2b"));
}

/// The conflict tests' plan (the brief's common setup): `max_writers = 3`, `check =
/// "true"`, every task S, `check` mode (reason `test`), Claude.
fn conflict_plan(tasks: &[String]) -> String {
    format!(
        "max_writers = 3\n{}",
        plan("", tasks).replace(
            "test_mode_reason = \"smoke\"",
            "test_mode_reason = \"test\""
        )
    )
}

fn claude_task(id: &str, owns: &str) -> String {
    task(id, &[owns], "route = { runtime = \"claude\" }")
}

/// A harness for the conflict tests: `b/shared.txt` reads `base`; no small reviews.
fn conflict_harness() -> RunHarness {
    RunHarness::with_repo(
        "review.small = \"off\"",
        &[],
        true,
        &[("b/shared.txt", "base\n")],
    )
}

/// `t1`'s first turn: `a/one.txt`, and a spill into `b/shared.txt`.
fn t1_spills() -> Value {
    sh(
        "mkdir -p a && printf 'one\\n' > a/one.txt && printf 'from t1\\n' > b/shared.txt && git add -A && git commit -qm 't1 work'",
    )
}

fn write_shared(text: &str) -> Value {
    sh(&format!(
        "printf '{text}\\n' > b/shared.txt && git add b/shared.txt && git commit -qm '{text}'"
    ))
}

const RESOLVE: &str =
    "printf 'from t1 and t2\\n' > b/shared.txt && git add b/shared.txt && git commit --no-edit";

fn conflict_messages(h: &RunHarness) -> Vec<String> {
    user_texts(&h.io_lines("worker-t1-1", "stdin"))
        .into_iter()
        .filter(|t| t.starts_with("[anthrex] Your branch conflicts with the run branch"))
        .collect()
}

/// Waits until `t1` is `blocked(mis_sized)` on its spill and `t2` has merged.
fn spilled_and_t2_merged(h: &RunHarness, id: &str) {
    let run = h.wait_run(
        id,
        |r| t(r, "t1").state == TaskState::Blocked && t(r, "t2").state == TaskState::Merged,
        RUN_WAIT,
    );
    let block = t(&run, "t1").block.clone().unwrap();
    assert_eq!(block.reason, BlockReason::MisSized, "{block:?}");
    assert!(
        block
            .text
            .contains("changed files outside owns: b/shared.txt"),
        "{block:?}"
    );
}

fn override_t1(h: &RunHarness, id: &str) {
    let output = h.anthrex(&["run", "override", id, "t1", "--reason", "shared-ok"]);
    assert!(
        output.status.success(),
        "run override: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn e2e_conflict_is_handed_back_and_resolved() {
    let h = conflict_harness();
    h.script(
        "worker-t1-1",
        &[
            t1_spills(),
            done("t1"),
            read("conflicts with the run branch"),
            sh(RESOLVE),
            done("resolved"),
        ],
    );
    h.script("worker-t2-1", &[write_shared("from t2"), done("t2")]);
    let id = h.start(
        &conflict_plan(&[claude_task("t1", "a/**"), claude_task("t2", "b/**")]),
        true,
    );
    spilled_and_t2_merged(&h, &id);
    let session_1 = t(&h.run(&id).unwrap(), "t1")
        .rounds
        .iter()
        .find(|r| r.role == AgentRole::Worker && r.session == 1)
        .and_then(|r| r.session_id.clone())
        .expect("session 1's id");
    override_t1(&h, &id);

    // t2's path and t1's (the hand-back resumes the same session).
    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.conflicts, 1);
    assert_eq!(t1.merged_without_approval.as_deref(), Some("shared-ok"));
    assert_eq!(conflict_messages(&h).len(), 1);

    let argvs: Vec<Vec<String>> = h
        .io_lines("worker-t1-1", "args")
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    assert!(
        argvs.iter().any(|argv| argv
            .windows(2)
            .any(|w| w[0] == "--resume" && w[1] == session_1)),
        "no --resume {session_1}: {argvs:#?}"
    );
    let shared = h.git(&["show", &format!("anthrex/{id}/integration:b/shared.txt")]);
    assert_eq!(shared, "from t1 and t2");

    // No proof, check or review for t1 between the hand-back and its merge.
    let report = report_with(&run, "handing the run head back");
    let section = report.split("## t1:").nth(1).expect("t1's section");
    let history: Vec<&str> = section
        .lines()
        .skip_while(|l| *l != "History:")
        .skip(1)
        .take_while(|l| l.starts_with("- "))
        .collect();
    let back = history
        .iter()
        .position(|l| l.contains("handing the run head back"))
        .unwrap_or_else(|| panic!("no hand-back in {history:#?}"));
    let merged = history
        .iter()
        .position(|l| l.contains("merged at"))
        .unwrap_or_else(|| panic!("no merge in {history:#?}"));
    let between = &history[back + 1..merged];
    assert!(
        between.iter().any(|l| l.contains("next: merge_queue")),
        "{history:#?}"
    );
    for line in between {
        assert!(
            !line.contains("proof") && !line.contains("check") && !line.contains("review"),
            "a gate ran after the hand-back: {line}\n{history:#?}"
        );
    }
}

#[test]
fn e2e_second_conflict_blocks_the_task() {
    let h = conflict_harness();
    let release = h.dir.path().join("release-t3");
    h.script(
        "worker-t1-1",
        &[
            t1_spills(),
            done("t1"),
            read("conflicts with the run branch"),
            wait_for_merge_of("t3"),
            sh(RESOLVE),
            done("resolved"),
        ],
    );
    h.script("worker-t2-1", &[write_shared("from t2"), done("t2")]);
    // Held until t1's hand-back, so t3's merge lands between the two candidates.
    h.script(
        "worker-t3-1",
        &[wait_for_file(&release), write_shared("from t3"), done("t3")],
    );
    let id = h.start(
        &conflict_plan(&[
            claude_task("t1", "a/**"),
            claude_task("t2", "b/**"),
            claude_task("t3", "b/**"),
        ]),
        true,
    );
    spilled_and_t2_merged(&h, &id);
    override_t1(&h, &id);
    h.wait_run(&id, |r| t(r, "t1").conflicts == 1, RUN_WAIT);
    std::fs::write(&release, "").unwrap();

    // t2, then t3 (which waits for t2 on b/**), then t1's second candidate.
    let run = h.wait_run(
        &id,
        |r| t(r, "t1").state == TaskState::Blocked && t(r, "t3").state == TaskState::Merged,
        3 * RUN_WAIT,
    );
    let t1 = t(&run, "t1");
    assert_eq!(
        t1.block.as_ref().map(|b| b.reason),
        Some(BlockReason::Conflict),
        "{:?}",
        t1.block
    );
    assert_eq!(t1.conflicts, 2);
    assert_eq!(
        conflict_messages(&h).len(),
        1,
        "{:#?}",
        conflict_messages(&h)
    );
}

/// Commits `path` (in a new directory: `git_commit` writes into an existing one only).
fn add_file(path: &str) -> Value {
    sh(&format!(
        "mkdir -p \"$(dirname {path})\" && printf 'x\\n' > {path} && git add {path} && git commit -qm 'add {path}'"
    ))
}

/// Fails when both `a/flag` and `b/need-no-flag` exist.
const FLAG_CHECK: &str = "if [ -e a/flag ] && [ -e b/need-no-flag ]; then echo 'both present'; exit 1; fi\necho check ok\n";

#[test]
fn e2e_red_candidate_goes_back_to_the_worker() {
    let h = RunHarness::with_repo("", &[], true, &[("check.sh", FLAG_CHECK)]);
    h.script("worker-t1-1", &[add_file("a/flag"), done("flag")]);
    h.script("reviewer-t1-1", &[approve()]);
    h.script(
        "worker-t2-1",
        &[
            // t1 merges first, so t2's candidate is the red one.
            wait_for_merge_of("t1"),
            add_file("b/need-no-flag"),
            done("need no flag"),
            read("merged cleanly into the run branch, but the check failed on the merged result"),
            sh("git rm -q b/need-no-flag && git commit -qm 'drop need-no-flag'"),
            done("dropped it"),
        ],
    );
    h.script("reviewer-t2-1", &[approve()]);
    h.script("reviewer-t2-2", &[approve()]);
    let plan = format!(
        "max_writers = 2\n{}",
        plan("", &[task("t1", &["a/**"], ""), task("t2", &["b/**"], "")])
    )
    .replace("check = \"true\"", "check = \"sh check.sh\"");
    let id = h.start(&plan, true);

    // t1's path, then t2's two.
    let run = h.wait_run(&id, complete, 2 * RUN_WAIT);
    let t2 = t(&run, "t2");
    assert_eq!(t2.state, TaskState::Merged);
    assert_eq!(t2.bounces.merge, 1, "{:?}", t2.bounces);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
}
