//! Milestone 9.16: the brakes end to end, through a real daemon (its socket and data
//! directory under the harness's `/tmp` directory) and `fake-agent` as both runtimes.
//! M9.16.8: the hold persists across a restart, and a held run finishes what it
//! started. M9.16.10: the restart test drives `anthrex hold on|off|status` and reads
//! `anthrex run status`; the other sends a raw `RunRequest::Hold`. M9.16.11: the quota
//! brake from a Claude `rate_limit_event` and a Codex rollout (fake-agent's
//! `rate_limit` and `rollout` steps), the hold's round trip, the failover to a row's
//! fallback, and the context handoff (`run_e2e_brakes_handoff.rs`).
//!
//! Ruling C19: every daemon here runs with U11's defaults (`BRAKES_ON`), where the
//! other harnesses turn the quota brake off.

use crate::support;

use proto::brakes::QuotaState;
use proto::{AgentRole, RunReply, RunRequest, RunState, Runtime, TaskState};
use serde_json::{Value, json};
use support::brakes::BRAKES_ON;
use support::run_harness::{RUN_WAIT, RunHarness};
use support::run_plans::*;

const HOLD_TEXT: &str = "dispatch is held daemon-wide · anthrex hold off";

fn hold(h: &RunHarness, on: bool) -> Option<proto::DispatchHold> {
    match h.request(RunRequest::Hold { on }) {
        RunReply::Held { hold, .. } => hold,
        other => panic!("not held: {other:?}\n{}", h.log_tail()),
    }
}

/// `anthrex <args>`'s stdout; it must succeed.
fn cli(h: &RunHarness, args: &[&str]) -> String {
    let out = h.anthrex(args);
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(
        out.status.success(),
        "anthrex {args:?}: {stdout}{}\n{}",
        String::from_utf8_lossy(&out.stderr),
        h.log_tail()
    );
    stdout
}

/// Decisions 16 and 34: `anthrex hold on`, a restart, the hold is still on with its
/// first `since` (`hold status` and `run status` say so); `hold off`, a restart, it is
/// off and `run status` prints no brake line.
#[test]
fn hold_persists_across_daemon_restart() {
    let mut h = RunHarness::with_config("", BRAKES_ON, &[]);
    let on = cli(&h, &["hold", "on"]);
    assert_eq!(
        on.trim_end(),
        "dispatch held daemon-wide; running sessions continue · anthrex hold off releases it"
    );
    let first = h.snapshot().brakes.hold.expect("held");
    assert_eq!(first.by, "user");
    h.restart_daemon(&[]);
    assert_eq!(h.snapshot().brakes.hold, Some(first.clone()));
    // Milestone 9.19 decision 3 (ruling R6): the CLI prints its own zone, which the
    // harness pins to `TZ=UTC0`, so `+00:00`; against the daemon's `now`, which may
    // pass midnight between the snapshots, so either day's form is right.
    let before = h.snapshot().now;
    let status = cli(&h, &["hold", "status"]);
    let runs = cli(&h, &["run", "status"]);
    let after = h.snapshot().now;
    let forms = [before, after].map(|now| {
        let since = common::numbers::clock(first.since, now, common::numbers::Zone::Local(0));
        format!("hold: on since {since} (user)")
    });
    let first_line = status.lines().next().unwrap_or_default().to_owned();
    assert!(forms.contains(&first_line), "{status}");
    assert!(runs.starts_with(&format!("{first_line}\n")), "{runs}");

    assert_eq!(cli(&h, &["hold", "off"]).trim_end(), "dispatch released");
    assert_eq!(cli(&h, &["hold", "off"]).trim_end(), "dispatch is not held");
    h.restart_daemon(&[]);
    assert_eq!(h.snapshot().brakes.hold, None);
    let status = cli(&h, &["hold", "status"]);
    assert_eq!(status.lines().next(), Some("hold: off"), "{status}");
    assert!(!cli(&h, &["run", "status"]).contains("hold:"));
}

/// U12 and decision 18: the hold stops new sessions only. A task whose worker turn is
/// open when the hold comes on finishes that turn, is reviewed and merged, and the run
/// completes; it never leaves `Running` for a paused state on the way.
#[test]
fn hold_keeps_run_running_and_open_turns_alive() {
    let h = RunHarness::with_config("", BRAKES_ON, &[]);
    let go = h.dir.path().join("go");
    let wait = format!("while [ ! -f '{}' ]; do sleep 0.05; done", go.display());
    h.script(
        "worker-t1-1",
        &[sh(&wait), commit("a.txt", "a\n"), done("added a")],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let id = h.start(&plan("", &[task("t1", &["a.txt"], "")]), true);
    h.wait_run(
        &id,
        |r| window_of(r, "t1", AgentRole::Worker).is_some(),
        RUN_WAIT,
    );

    assert!(hold(&h, true).is_some());
    let run = h.run(&id).unwrap();
    assert_eq!(run.state, RunState::Running);
    assert!(
        run.attention.iter().any(|l| l == HOLD_TEXT),
        "{:?}",
        run.attention
    );

    std::fs::write(&go, "").unwrap();
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    assert!(h.snapshot().brakes.hold.is_some(), "the hold is still on");
}

/// fake-agent's `rate_limit` step: Claude's `rate_limit_event` at `seven_day` (a fraction).
fn rate_limit(seven_day: f64) -> Value {
    json!({"rate_limit": {"seven_day": seven_day}})
}

/// fake-agent's `rollout` step: a Codex `token_count` whose `secondary` window is
/// `minutes` long at `percent`.
fn rollout(minutes: u64, percent: f64) -> Value {
    json!({"rollout": {"secondary": {"window_minutes": minutes, "used_percent": percent}}})
}

/// A `sh` step that waits, at most one `RUN_WAIT`, for `path`.
fn wait_for(path: &std::path::Path) -> Value {
    sh(&format!(
        "for i in $(seq 1 1500); do [ -e '{}' ] && exit 0; sleep 0.2; done; exit 1",
        path.display()
    ))
}

/// Waits until the snapshot says `runtime` is quota-braked.
fn wait_braked(h: &RunHarness, runtime: Runtime) {
    until(&format!("{runtime} to be braked"), RUN_WAIT, || {
        let snapshot = h.snapshot();
        let quota = snapshot
            .brakes
            .quota
            .iter()
            .find(|q| q.runtime == runtime)?;
        (quota.state == QuotaState::Braked && quota.trip.is_some()).then_some(())
    });
}

/// Waits until `anthrex run status` prints a line containing `needle`; the output.
fn wait_status_line(h: &RunHarness, needle: &str) -> String {
    until(&format!("run status to print {needle:?}"), RUN_WAIT, || {
        let out = cli(h, &["run", "status"]);
        out.contains(needle).then_some(out)
    })
}

/// Whether `<io>/<script>.args` exists: the script's session started.
fn started(h: &RunHarness, script: &str) -> bool {
    !h.io_lines(script, "args").is_empty()
}

/// Decisions 3, 4 and 11: a Claude worker's `rate_limit_event` at 0.97 of the 7-day
/// window trips the brake (95 %): the next Claude task stays queued and the run says
/// why, while the task that read it is still reviewed and merged. `t1`'s reviewer
/// waits for the test to see the trip, so `t2`'s turn comes after it.
#[test]
fn e2e_claude_rate_limit_brakes_new_claude_tasks() {
    let h = RunHarness::with_config("max_writers = 1\n", BRAKES_ON, &[]);
    let go = h.dir.path().join("go");
    h.script(
        "worker-t1-1",
        &[rate_limit(0.97), commit("a.txt", "a\n"), done("added a")],
    );
    h.script("reviewer-t1-1", &[wait_for(&go), approve()]);
    h.script("worker-t2-1", &[commit("b.txt", "b\n"), done("added b")]);
    let tasks = [
        task("t1", &["a.txt"], ""),
        task("t2", &["b.txt"], "deps = [\"t1\"]"),
    ];
    let id = h.start(&plan("", &tasks), true);

    wait_braked(&h, Runtime::Claude);
    std::fs::write(&go, "").unwrap();
    let run = h.wait_run(&id, |r| t(r, "t1").state == TaskState::Merged, RUN_WAIT);
    assert_eq!(t(&run, "t2").state, TaskState::Queued);
    let out = wait_status_line(&h, "claude dispatch held: 7d at 97% (\u{2265}95%) until ");
    let run = h.run(&id).unwrap();
    assert_eq!(run.state, RunState::Running, "{out}");
    assert_eq!(t(&run, "t2").state, TaskState::Queued, "{out}");
    assert!(!started(&h, "worker-t2-1"), "t2's worker started:\n{out}");
}

/// Decision 5: a Codex worker's rollout (`token_count` under the harness's
/// `CODEX_HOME`) at 99 % of the 10080-minute window brakes Codex only: the next Codex
/// task waits while a Claude task launches and merges.
#[test]
fn e2e_codex_rollout_brakes_codex_only() {
    let h = RunHarness::with_config(
        "max_writers = 1\n",
        &format!("{CODEX_MEDIUM}{BRAKES_ON}"),
        &[],
    );
    let go = h.dir.path().join("go");
    h.script(
        "worker-t1-1",
        &[
            rollout(10080, 99.0),
            commit("a.txt", "a\n"),
            done("added a"),
        ],
    );
    h.script("reviewer-t1-1", &[wait_for(&go), approve()]);
    h.script("worker-t2-1", &[commit("b.txt", "b\n"), done("added b")]);
    h.script("worker-t3-1", &[commit("c.txt", "c\n"), done("added c")]);
    h.script("reviewer-t3-1", &[approve()]);
    let after = "deps = [\"t1\"]";
    let tasks = [
        task_m("t1", &["a.txt"], ""),
        task_m("t2", &["b.txt"], after),
        task("t3", &["c.txt"], after),
    ];
    let id = h.start(&plan("", &tasks), true);
    let run = h.wait_run(&id, |r| !r.tasks.is_empty(), RUN_WAIT);
    assert_eq!(t(&run, "t1").route.runtime, Runtime::Codex);

    wait_braked(&h, Runtime::Codex);
    std::fs::write(&go, "").unwrap();
    let run = h.wait_run(&id, |r| t(r, "t3").state == TaskState::Merged, RUN_WAIT);
    assert_eq!(t(&run, "t1").state, TaskState::Merged);
    assert_eq!(t(&run, "t3").route.runtime, Runtime::Claude);
    assert_eq!(t(&run, "t2").state, TaskState::Queued);
    assert!(!started(&h, "worker-t2-1"));
    let claude = h.snapshot().brakes.quota;
    let claude = claude.iter().find(|q| q.runtime == Runtime::Claude);
    assert!(claude.is_none_or(|q| q.trip.is_none()), "{claude:?}");
    wait_status_line(&h, "codex dispatch held: 7d at 99% (\u{2265}95%) until ");
}

/// Decisions 15–18 through the CLI: `anthrex hold on` while `t1`'s worker turn is
/// open; that task finishes and merges, and no new task starts; `anthrex hold off`, and
/// the next task starts and the run completes.
#[test]
fn e2e_hold_on_off_round_trip() {
    let h = RunHarness::with_config("max_writers = 1\n", BRAKES_ON, &[]);
    let go = h.dir.path().join("go");
    h.script(
        "worker-t1-1",
        &[wait_for(&go), commit("a.txt", "a\n"), done("added a")],
    );
    h.script("reviewer-t1-1", &[approve()]);
    h.script("worker-t2-1", &[commit("b.txt", "b\n"), done("added b")]);
    h.script("reviewer-t2-1", &[approve()]);
    let tasks = [task("t1", &["a.txt"], ""), task("t2", &["b.txt"], "")];
    let id = h.start(&plan("", &tasks), true);
    h.wait_run(
        &id,
        |r| window_of(r, "t1", AgentRole::Worker).is_some(),
        RUN_WAIT,
    );

    cli(&h, &["hold", "on"]);
    std::fs::write(&go, "").unwrap();
    let run = h.wait_run(&id, |r| t(r, "t1").state == TaskState::Merged, RUN_WAIT);
    assert_eq!(run.state, RunState::Running);
    assert_eq!(t(&run, "t2").state, TaskState::Queued);
    assert!(!started(&h, "worker-t2-1"));
    assert!(
        run.attention.iter().any(|l| l == HOLD_TEXT),
        "{:?}",
        run.attention
    );

    assert_eq!(cli(&h, &["hold", "off"]).trim_end(), "dispatch released");
    let run = h.wait_run(&id, complete, RUN_WAIT);
    assert_eq!(t(&run, "t2").state, TaskState::Merged);
    assert!(started(&h, "worker-t2-1"));
    assert!(!run.attention.iter().any(|l| l == HOLD_TEXT));
}

/// U17 (decisions 36–42): `implementer.medium` is Claude with a Codex fallback, from a
/// repository `models.toml`. `t1`'s Claude worker reads 0.97 of the 7-day window, so
/// `t2`, on the same row and owning nothing of `t1`'s, starts on its fallback; the run
/// says so, `t2`'s reviewer is not on `t2`'s Codex model, and its history line records
/// the failover.
#[test]
fn e2e_braked_claude_task_fails_over_to_codex_fallback() {
    let h = RunHarness::with_config("max_writers = 1\n", BRAKES_ON, &[]);
    let models =
        "[models.implementer.medium]\nmodel = \"claude:default\"\nfallback = \"codex:default\"\n";
    let repo_dir = h.repo_dir();
    std::fs::create_dir_all(&repo_dir).unwrap();
    std::fs::write(repo_dir.join(config::models::REPO_FILE), models).unwrap();
    let go = h.dir.path().join("go");
    h.script(
        "worker-t1-1",
        &[rate_limit(0.97), commit("a.txt", "a\n"), done("added a")],
    );
    h.script("reviewer-t1-1", &[wait_for(&go), approve()]);
    h.script("worker-t2-1", &[commit("b.txt", "b\n"), done("added b")]);
    h.script("reviewer-t2-1", &[approve()]);
    let tasks = [
        task_m("t1", &["a.txt"], ""),
        task_m("t2", &["b.txt"], "deps = [\"t1\"]"),
    ];
    let id = h.start(&plan("", &tasks), true);
    let run = h.wait_run(&id, |r| !r.tasks.is_empty(), RUN_WAIT);
    assert_eq!(t(&run, "t1").route.runtime, Runtime::Claude);

    wait_braked(&h, Runtime::Claude);
    std::fs::write(&go, "").unwrap();
    let run = h.wait_run(
        &id,
        |r| window_of(r, "t2", AgentRole::Worker).is_some(),
        RUN_WAIT,
    );
    let worker = t(&run, "t2")
        .rounds
        .iter()
        .find(|r| r.role == AgentRole::Worker);
    let worker = worker.unwrap().route.clone();
    assert_eq!(worker.runtime, Runtime::Codex, "{worker:?}");
    wait_status_line(&h, "claude braked: t2 started on codex (fallback)");

    let run = h.wait_run(&id, complete, RUN_WAIT);
    let rounds = &t(&run, "t2").rounds;
    let reviewer = rounds
        .iter()
        .find(|r| r.role == AgentRole::Reviewer)
        .unwrap();
    assert_ne!(
        (reviewer.route.runtime, &reviewer.route.model),
        (worker.runtime, &worker.model),
        "the reviewer got the author's model"
    );
    // A run's records are due at its accept (`run::history::run_outcome`).
    h.accept(&id);
    let lines = until("t2's history line", RUN_WAIT, || {
        let lines = h.history_lines("task");
        (lines.into_iter()).find(|l| l["run_id"] == id.as_str() && l["task_id"] == "t2")
    });
    assert!(lines.to_string().contains("\"failover\""), "{lines}");
}

/// Ruling C19: with U11's defaults, a run whose sessions report no reading says once,
/// in its log, that Claude's quota is unmeasured.
#[test]
fn e2e_defaults_announce_unmeasured_once() {
    let h = RunHarness::with_config("", BRAKES_ON, &[]);
    for (id, file) in [("t1", "a.txt"), ("t2", "b.txt")] {
        h.script(
            &format!("worker-{id}-1"),
            &[commit(file, "x\n"), done("added")],
        );
        h.script(&format!("reviewer-{id}-1"), &[approve()]);
    }
    let tasks = [task("t1", &["a.txt"], ""), task("t2", &["b.txt"], "")];
    let id = h.start(&plan("", &tasks), true);
    h.wait_run(&id, complete, RUN_WAIT);
    let log = support::run_pr::log_lines(&h, &id);
    let blind = "claude quota is unmeasured; dispatch is not braked";
    assert_eq!(log.iter().filter(|l| *l == blind).count(), 1, "{log:#?}");
}

/// Decisions 22–27 (U13): a chain whose orchestrator's transcript, named by its prompt
/// hook as Claude names it, is 90 % of the 200,000-token default window. `run start
/// --goal --continue` hands off: a fresh orchestrator window, whose first message names
/// the fill, while the old one is renamed `<h4>/orchestrator (handed off)` and still
/// listed by `anthrex ls`; the new run's inspector JSON has `handoff`.
#[test]
fn e2e_continue_hands_off_a_full_orchestrator() {
    use support::orch_script::{
        edit_plan, marker, passed, prompt, read as read_msg, until as poll,
    };
    use support::run_harness::{FINISH_WAIT, REQUEST_WAIT};
    use support::run_rounds::{CONTINUE_WAIT, ORCH, SUMMARY_1, SUMMARY_WAIT, green, h4, staged};

    let h = RunHarness::orch_with_config("", &[], BRAKES_ON);
    // The session's last main-chain assistant line: 2 + 20,000 + 159,994 + 4 = 180,000.
    let transcript = h.dir.path().join("orchestrator-transcript.jsonl");
    let usage = json!({"input_tokens": 2, "cache_creation_input_tokens": 20000,
        "cache_read_input_tokens": 159994, "output_tokens": 4});
    let line = json!({"type": "assistant", "isSidechain": false, "message": {
        "model": "claude-fake", "role": "assistant", "usage": usage,
        "content": [{"type": "text", "text": "planned"}]}});
    std::fs::write(&transcript, format!("{line}\n")).unwrap();
    let mut first = prompt();
    first["payload"]["transcript_path"] = json!(transcript.display().to_string());
    green(&h, "t1", "a.txt");
    h.script(
        ORCH,
        &[
            first,
            edit_plan(
                vec![staged("t1", "a.txt", 1)],
                json!({"submit": true, "decided": [], "out_of_scope": []}),
            ),
            poll("/run/complete", json!(true), RUN_WAIT),
            edit_plan(vec![], json!({"summary": SUMMARY_1})),
            poll("/run/state", json!("accepted"), FINISH_WAIT),
            read_msg(None),
        ],
    );
    let fresh = "orchestrator-run-2";
    h.script(fresh, &[marker(), read_msg(None)]);
    let run = h.start_goal_id("add a", &["--yes"]);
    let old = h.orchestrator_window(&run);
    h.wait_summary(&run, SUMMARY_1, SUMMARY_WAIT);
    h.accept(&run);

    let next = h.continue_goal("next", &run);
    let window = h.orchestrator_window(&next);
    assert_ne!(window, old, "the goal adopted the full session");
    let renamed = format!("{}/orchestrator (handed off)", h4(&run));
    h.wait_window(old, "renamed", |w| w.name == renamed, CONTINUE_WAIT);
    let ls = cli(&h, &["ls"]);
    assert!(ls.contains(&renamed), "{ls}");
    h.wait_log(
        "the fresh orchestrator",
        |log| passed(log, fresh) >= 1,
        REQUEST_WAIT,
    );
    let said = h.io_lines(fresh, "stdin").join("\n");
    let fill =
        "The previous session ended with context 90% full (180k / 200k); this one starts fresh.";
    assert!(said.contains(fill), "{said}");
    let handoff = &h.status_json(&next)["handoff"];
    assert!(handoff.is_object(), "{handoff}");
    assert_eq!(handoff["from_run"], json!(h4(&run)), "{handoff}");
}
