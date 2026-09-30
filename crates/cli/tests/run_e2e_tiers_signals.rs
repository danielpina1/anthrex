//! Milestone 9.1 task M9.1.21: a tier command's isolation, test-weakening signals and
//! `message` to a stage end to end, through the real binary and a real daemon on
//! `/tmp` paths, with `fake-agent` workers (`support/run_tiers.rs`).

mod support;

use proto::TaskState;
use serde_json::{Value, json};
use support::run_harness::RunHarness;
use support::run_plans::*;
use support::run_tiers::*;

/// Decisions 40–42's keys: the tests live under `mods/*/tests/`, `#[ignore]` skips one.
const SIGNAL_KEYS: &str = "test_paths = [\"mods/*/tests/**\"]\nskip_markers = [\"#[ignore]\"]";

fn harness() -> RunHarness {
    RunHarness::with_config("", "", &tier_repo_files())
}

/// Task `id`'s entry of `run.json`.
fn task_json(run: &proto::RunInfo, id: &str) -> Value {
    run_json(run)["tasks"]
        .as_array()
        .expect("run.json lists its tasks")
        .iter()
        .find(|t| t["spec"]["id"] == id)
        .cloned()
        .expect("the task is in run.json")
}

fn expect_error(text: &str) -> Value {
    json!({"expect_error_contains": {"text": text}})
}

#[test]
fn e2e_a_tier_command_that_starts_a_daemon_gets_its_own_socket() {
    let h = harness();
    h.script(
        "worker-t1-1",
        &[commit("mods/b/src.txt", "b2\n"), done("changed b")],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(
        &tier_plan(&h, "", &[task("t1", &["mods/b/src.txt"], "")]),
        true,
    );
    let run = h.wait_run(&id, complete, TIER_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);

    let steps: Vec<_> = tier_log(&h)
        .into_iter()
        .filter(|l| l["script"] != "graph.sh")
        .collect();
    assert!(
        runs_of(&steps, "test.sh").len() >= 2,
        "b and c were tested: {steps:#?}"
    );
    let own_socket = h.socket().display().to_string();
    let own_data = h.data().display().to_string();
    for step in &steps {
        let env = &step["env"];
        let tmp = env["TMPDIR"].as_str().unwrap_or_default();
        assert!(!tmp.is_empty(), "{step}");
        let tmp = tmp.trim_end_matches('/');
        let socket = env["ANTHREX_SOCKET"].as_str().unwrap_or_default();
        let data = env["ANTHREX_DATA_DIR"].as_str().unwrap_or_default();
        assert_eq!(socket, format!("{tmp}/d.sock"), "{step}");
        assert_eq!(data, format!("{tmp}/data"), "{step}");
        assert_ne!(socket, own_socket);
        assert_ne!(data, own_data);
        // Decision 28: a fresh directory per step, never inside the checkout.
        let cwd = step["cwd"].as_str().unwrap_or_default();
        assert!(!tmp.starts_with(cwd), "{step}");
    }
    let dirs: std::collections::BTreeSet<&str> = steps
        .iter()
        .filter_map(|s| s["env"]["TMPDIR"].as_str())
        .collect();
    assert_eq!(dirs.len(), steps.len(), "one TMPDIR per step: {steps:#?}");
    // The harness daemon is unaffected: it still answers on its own socket.
    assert!(h.run(&id).is_some());
}

#[test]
fn e2e_deleted_test_file_bounces() {
    let h = harness();
    let base = h.git(&["rev-parse", "HEAD"]);
    // The path in the command is shell-quoted, as `deleted_test_file_message` quotes
    // every path.
    let message = format!(
        "[anthrex] task_done rejected:\n\
         deleted test file mods/b/tests/t.rs; restore it or own it exactly\n\
         Restore it (git checkout {} -- 'mods/b/tests/t.rs', then commit) and call task_done again. If this task must delete it, call task_blocked with kind question and ask for the plan to be amended.",
        &base[..7]
    );
    h.script(
        "worker-t1-1",
        &[
            commit("mods/b/src.txt", "b2\n"),
            sh("git rm -q mods/b/tests/t.rs && git commit -qm 'delete the test'"),
            done_expecting_error(),
            expect_error("deleted test file mods/b/tests/t.rs; restore it or own it exactly"),
            sh(&format!(
                "git checkout {} -- mods/b/tests/t.rs && git commit -qm 'restore the test'",
                &base[..7]
            )),
            done("changed b"),
        ],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(
        &tier_plan(&h, SIGNAL_KEYS, &[task("t1", &["mods/b/**"], "")]),
        true,
    );
    // Ruling C-25 (fail fast): stop at completion, a block, or the claim passing the
    // done gate without a bounce, which is what a missing bounce looks like.
    let run = h.wait_run(
        &id,
        |r| {
            let t1 = t(r, "t1");
            let past_done = matches!(
                t1.state,
                TaskState::Check | TaskState::Review | TaskState::MergeQueue | TaskState::Merged
            );
            complete(r) || t1.state == TaskState::Blocked || (past_done && t1.bounces.done == 0)
        },
        TIER_WAIT,
    );
    let t1 = t(&run, "t1");
    assert_eq!(
        (t1.bounces.done, t1.rung),
        (1, 1),
        "the claim was bounced once"
    );
    let run = h.wait_run(&id, complete, TIER_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!((t1.bounces.done, t1.rung), (1, 1));
    let log = task_json(&run, "t1")["failure_log"].clone();
    assert!(
        log.as_array()
            .is_some_and(|l| l.iter().any(|entry| entry == message.as_str())),
        "{log:#}"
    );
    let integration = format!("anthrex/{id}/integration");
    assert!(
        h.git(&["show", &format!("{integration}:mods/b/tests/t.rs")])
            .contains("fn works")
    );
}

#[test]
fn e2e_skip_marker_must_be_justified_by_the_reviewer() {
    let h = harness();
    h.script(
        "worker-t1-1",
        &[
            commit(
                "mods/b/tests/t.rs",
                "#[test]\n#[ignore]\nfn works() {\n    assert!(true);\n}\n",
            ),
            done("skipped a slow test"),
        ],
    );
    let refusal = "the review must address W1: add a finding whose text starts with each id";
    let answer =
        json!({"severity": "minor", "text": "W1 accepted: the test is slow and runs nightly"});
    h.script(
        "reviewer-t1-1",
        &[
            json!({"mcp_call": {"tool": "submit_review", "args": {"verdict": "approve", "summary": "ok", "findings": []}, "expect_error": true}}),
            expect_error(refusal),
            json!({"mcp_call": {"tool": "submit_review", "args": {"verdict": "approve", "summary": "ok", "findings": [answer]}}}),
        ],
    );
    let id = h.start(
        &tier_plan(&h, SIGNAL_KEYS, &[task("t1", &["mods/b/**"], "")]),
        true,
    );
    let run = h.wait_run(&id, complete, TIER_WAIT);
    let t1 = t(&run, "t1");
    assert_eq!(t1.state, TaskState::Merged);
    assert_eq!(t1.weakening.len(), 1, "{:?}", t1.weakening);
    let w1 = &t1.weakening[0];
    assert_eq!(
        (w1.id.as_str(), w1.kind.as_str(), w1.path.as_str(), w1.line),
        ("W1", "skip_marker", "mods/b/tests/t.rs", Some(2))
    );
    assert_eq!(
        w1.answered.as_deref(),
        Some("accepted: the test is slow and runs nightly")
    );
    // The reviewer (Codex, prompted through its argv) was shown the signal; its
    // first submission was refused, as its script's `expect_error_contains` checked.
    let prompt = h.codex_messages("reviewer-t1-1").concat();
    assert!(
        prompt.contains("W1 mods/b/tests/t.rs:2: added skip marker #[ignore]"),
        "{prompt}"
    );
    report_with(&run, "Test changes:");
}

#[test]
fn e2e_message_to_a_stage() {
    // Ruling C-25 (fail fast): a worker whose turn ends with no message waiting is
    // nudged after 5 s, so a stage-2 worker's next input is the message or the nudge.
    let h = RunHarness::with_config("stall_after_secs = 5", "", &tier_repo_files());
    let go = h.dir.path().join("go");
    let wait = sh(&format!(
        "for i in $(seq 1 1500); do [ -e '{}' ] && exit 0; sleep 0.2; done; exit 1",
        go.display()
    ));
    let text = "[anthrex] Message from the user (info): the schema moved";
    h.script(
        "worker-t1-1",
        &[wait.clone(), commit("mods/a/src.txt", "a2\n"), done("a")],
    );
    for (task, file) in [("t2", "mods/b/src.txt"), ("t3", "mods/c/src.txt")] {
        h.script(
            &format!("worker-{task}-1"),
            &[
                wait.clone(),
                json!({"read_message": {}}),
                commit(file, "2\n"),
                done(task),
            ],
        );
    }
    for task in ["t1", "t2", "t3"] {
        h.script(&format!("reviewer-{task}-1"), &[approve()]);
    }
    let tasks = [
        task("t1", &["mods/a/src.txt"], ""),
        task("t2", &["mods/b/src.txt"], "stage = 2"),
        task("t3", &["mods/c/src.txt"], "stage = 2"),
    ];
    let id = h.start(&tier_plan(&h, "", &tasks), true);
    h.wait_run(
        &id,
        |r| {
            r.tasks.iter().all(|task| {
                task.state == TaskState::Working && task.rounds.iter().any(|r| r.ended_at.is_none())
            })
        },
        TIER_WAIT,
    );

    let repo = h.repo.display().to_string();
    let out = h.anthrex(&[
        "--dir", &repo, "run", "message", &id, "stage:2", "the", "schema", "moved",
    ]);
    assert!(out.status.success(), "{out:?}");
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "applied 1 edit; message for t2, t3\n"
    );
    std::fs::write(&go, "").unwrap();
    // Each stage-2 worker's next input after its first turn is the message.
    for name in ["worker-t2-1", "worker-t3-1"] {
        let next = until("the stage-2 worker's next input", TIER_WAIT, || {
            user_texts(&h.io_lines(name, "stdin")).get(1).cloned()
        });
        assert!(next.contains(text), "{name}'s next input: {next}");
    }
    let run = h.wait_run(&id, complete, TIER_WAIT);
    assert!(run.tasks.iter().all(|t| t.state == TaskState::Merged));
    assert!(
        !h.io_lines("worker-t1-1", "stdin")
            .iter()
            .any(|l| l.contains(text)),
        "stage 1's worker got no message"
    );
}
