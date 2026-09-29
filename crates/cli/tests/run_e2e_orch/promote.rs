//! `run promote` (decisions 28 and 29), and a wake-up beside the user's typing
//! (decision 39's quiet time).

use std::time::{Duration, Instant};

use proto::{HoldState, RunState, TaskState};
use serde_json::json;

use crate::common::*;
use crate::support::orch_script::*;
use crate::support::run_adapt::triage_single;
use crate::support::run_harness::{REQUEST_WAIT, RUN_WAIT};
use crate::support::run_orch::{ORCH_WAIT, framed};
use crate::support::run_plans::{approve, commit, done};

#[test]
fn e2e_promote_starts_an_orchestrator_and_holds_its_tasks() {
    let h = harness("");
    h.decider("triage", 1, triage_single(&["a.txt"]));
    let go = h.dir.path().join("go");
    h.script(
        "worker-t1-1",
        &[wait_file(&go), commit("a.txt", "t1\n"), done("added a.txt")],
    );
    h.script("reviewer-t1-1", &[approve()]);
    green(&h, "t2", "b.txt");
    h.script(
        ORCH,
        &[
            prompt(),
            call("get_context", json!({})),
            call("run_status", json!({})),
            edit_plan(
                vec![add(plan_task("t2", &["b.txt"], json!({})))],
                json!({"submit": true}),
            ),
            expect("/held", json!("promotion")),
            expect("/awaiting_approval", json!(true)),
            until("/gate/holds/0/state", json!("approved"), RUN_WAIT * 2),
            marker(),
            read(None),
        ],
    );
    let run = h.start_goal_id("add a", &[]);
    wait_task(&h, &run, "t1", TaskState::Working);
    let repo = h.repo.display().to_string();
    ok(&h.anthrex(&["run", "promote", &run, "--dir", &repo]));
    h.orchestrator_window(&run);

    // Its addition waits for the user under hold `promotion`; t1 keeps running.
    let held = |r: &proto::RunInfo| {
        r.holds
            .iter()
            .any(|h| h.id == "promotion" && h.state == HoldState::Awaiting)
    };
    let info = h.wait_run(&run, held, ORCH_WAIT);
    assert_eq!(task(&info, "t2").hold.as_deref(), Some("promotion"));
    assert_eq!(task(&info, "t1").state, TaskState::Working);
    std::fs::write(&go, "").unwrap();
    let info = wait_task(&h, &run, "t1", TaskState::Merged);
    let t2 = task(&info, "t2");
    assert!(
        matches!(t2.state, TaskState::Pending | TaskState::Queued),
        "{t2:?}"
    );
    assert!(t2.rounds.is_empty(), "{:?}", t2.rounds);

    ok(&h.anthrex(&["run", "approve", &run, "--hold", "promotion"]));
    let info = h.wait_run(&run, |r| r.state == RunState::Complete, RUN_WAIT);
    assert_eq!(task(&info, "t2").state, TaskState::Merged);
    // The orchestrator read the verdict through `run_status`.
    wait_passed(&h, 1);
}

/// `daemon::run::driver::wake::SUBMIT_DELAY` (private to the driver): the paste, then
/// this long, then the `\r` that submits it.
const SUBMIT_DELAY_MS: u64 = 200;

#[test]
fn e2e_wake_does_not_collide_with_typing() {
    let h = harness("");
    let block = h.dir.path().join("block");
    h.script(
        "worker-t1-1",
        &[
            wait_file(&block),
            call(
                "task_blocked",
                json!({"kind": "question", "reason": "which name?"}),
            ),
            read(None),
        ],
    );
    let steps = [
        prompt(),
        edit_plan(
            vec![add(plan_task("t1", &["a.txt"], json!({})))],
            json!({"submit": true}),
        ),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        read(None),
        marker(),
        read(None),
    ];
    let (run, window) = start(&h, &steps);
    approve_plan(&h, &run);
    wait_task(&h, &run, "t1", TaskState::Working);
    h.wait_orchestrator_idle(window);

    // Typing every 300 ms for 3 s, with `wake_quiet_secs = 1`; t1 blocks early on.
    let started = Instant::now();
    let (mut typed, mut last_typed, mut blocked_at) = (0, 0, None);
    while started.elapsed() < Duration::from_secs(3) {
        // Taken before the keystroke is sent, so it is never after the daemon's own
        // record of the client's input.
        (typed, last_typed) = (typed + 1, now_ms());
        h.type_into(window, b"x");
        if started.elapsed() >= Duration::from_millis(300) && !block.exists() {
            std::fs::write(&block, "").unwrap();
        }
        let info = h.run(&run).unwrap();
        if blocked_at.is_none() && task(&info, "t1").state == TaskState::Blocked {
            blocked_at = Some(now_ms());
        }
        assert!(h.read_messages(ORCH).is_empty(), "a paste while typing");
        std::thread::sleep(Duration::from_millis(300));
    }
    let blocked_at = blocked_at.expect("t1 blocked while the user typed");
    assert!(blocked_at < last_typed, "{blocked_at} {last_typed}");

    // The wake-up comes a quiet second after the last keystroke, after the typing.
    h.wait_messages(ORCH, 1, REQUEST_WAIT);
    let message = &h.read_messages(ORCH)[0];
    let raw = message["raw"].as_str().unwrap();
    let keys = "x".repeat(typed);
    let text = message["text"].as_str().unwrap();
    let wake = text
        .strip_prefix(keys.as_str())
        .unwrap_or_else(|| panic!("{message}"));
    assert!(
        wake.contains("t1 blocked (question): which name?"),
        "{wake}"
    );
    assert_eq!(raw, format!("{keys}{}", framed(wake)), "{message}");
    // `at` is the submitting `\r`, which the driver writes `SUBMIT_DELAY` after the
    // paste; the paste itself comes no sooner than `wake_quiet_secs` after the input.
    let submitted = epoch_ms(message["at"].as_str().unwrap());
    assert!(
        // Less 1 ms: both stamps are cut to whole milliseconds.
        submitted + 1 >= last_typed + 1_000 + SUBMIT_DELAY_MS,
        "the paste was submitted {} ms after the last keystroke",
        submitted.saturating_sub(last_typed)
    );
    assert!(epoch_ms(message["first_at"].as_str().unwrap()) <= last_typed);
    wait_passed(&h, 1);
}
