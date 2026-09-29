//! Milestone 9 task M9.13: decision 39's wake-up, pasted into the orchestrator's PTY
//! window by the driver only when the window is idle and quiet, never on attention, a
//! newer one replacing one not yet delivered; and decision 13's exit. Through a real
//! daemon, with `fake-agent` as the orchestrator (`orchestrator-run-1`) and as the
//! triage decider; no real agent.

mod support;

use std::time::{Duration, Instant};

use proto::Status;
use serde_json::{Value, json};
use support::run_harness::{REQUEST_WAIT, RunHarness};
use support::run_orch::*;

const ORCH: &str = "orchestrator-run-1";

fn raw(message: &Value) -> String {
    message["raw"].as_str().unwrap_or_default().to_string()
}

/// A planned run whose orchestrator runs `steps`; the run and its window, `Working`.
fn started(steps: &[Value]) -> (RunHarness, String, u32) {
    let h = RunHarness::orch("", &[]);
    h.script(ORCH, steps);
    let run = h.start_goal_id("rework storage", &[]);
    let window = h.orchestrator_window(&run);
    h.wait_window(
        window,
        "working",
        |w| w.status == Status::Working,
        ORCH_WAIT,
    );
    (h, run, window)
}

#[test]
fn wake_is_delivered_only_when_idle_and_quiet() {
    let mut steps = working_until_typed();
    steps.extend([read_message(), read_message()]);
    let (h, run, window) = started(&steps);

    // A user edit wakes the orchestrator; its window is `Working`: nothing.
    h.add_task(&run, "t1");
    std::thread::sleep(Duration::from_millis(2_500));
    assert!(
        h.read_messages(ORCH).is_empty(),
        "{:?}",
        h.read_messages(ORCH)
    );
    assert_eq!(h.window(window).unwrap().status, Status::Working);

    // The user types; the turn ends at once. The window is `Done`, with input just
    // now and `wake_quiet_secs = 1`: nothing yet. After a second: the paste, then `\r`.
    let typed = Instant::now();
    h.type_into(window, b"go\r");
    let at = h.wait_messages(ORCH, 1, REQUEST_WAIT);
    let waited = at.duration_since(typed);
    assert!(waited >= Duration::from_secs(1), "{waited:?}");
    assert!(waited < Duration::from_secs(5), "{waited:?}");
    let message = &h.read_messages(ORCH)[0];
    let text = message["text"].as_str().unwrap();
    assert!(text.starts_with("[anthrex]"), "{text}");
    assert!(text.contains("the user edited the plan"), "{text}");
    assert_eq!(raw(message), framed(text), "{message}");
    let info = h.wait_run(
        &run,
        |r| {
            r.orchestrator
                .as_ref()
                .is_some_and(|o| o.wakes == 1 && o.notes.is_empty())
        },
        REQUEST_WAIT,
    );
    assert!(info.orchestrator.unwrap().live);
}

#[test]
fn wake_is_not_delivered_on_attention() {
    let steps = [
        json!({"hook": "UserPromptSubmit", "payload": {"prompt": "plan"}}),
        json!({"hook": "Notification", "payload": {"notification_type": "permission_prompt", "message": "Claude needs your permission to use Read"}}),
        json!({"wait_ms": 6000}),
        read_message(),
        read_message(),
    ];
    let h = RunHarness::orch("", &[]);
    h.script(ORCH, &steps);
    let run = h.start_goal_id("rework storage", &[]);
    let window = h.orchestrator_window(&run);
    h.wait_window(
        window,
        "attention",
        |w| w.status == Status::Attention,
        ORCH_WAIT,
    );
    h.add_task(&run, "t1");
    // While the window asks for attention (a permission prompt, perhaps): nothing.
    let mut attention_seen = 0;
    while h.window(window).unwrap().status == Status::Attention {
        attention_seen += 1;
        assert!(
            h.read_messages(ORCH).is_empty(),
            "{:?}",
            h.read_messages(ORCH)
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        attention_seen >= 10,
        "the window left attention too soon to tell"
    );
    // Its turn ends, with no client input: delivered.
    h.wait_messages(ORCH, 1, REQUEST_WAIT);
    let text = h.read_messages(ORCH)[0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(text.contains("the user edited the plan"), "{text}");
}

#[test]
fn a_newer_wake_replaces_an_undelivered_one() {
    let mut steps = working_until_typed();
    steps.extend([read_message(), read_message()]);
    let (h, run, window) = started(&steps);
    h.add_task(&run, "t1");
    h.add_task(&run, "t2");
    h.type_into(window, b"go\r");
    h.wait_messages(ORCH, 1, REQUEST_WAIT);
    // The window is idle again after the paste: a second paste would come now.
    std::thread::sleep(Duration::from_millis(3_000));
    let messages = h.read_messages(ORCH);
    assert_eq!(messages.len(), 1, "{messages:#?}");
    let text = messages[0]["text"].as_str().unwrap();
    assert!(text.contains("add t1") && text.contains("add t2"), "{text}");
}

#[test]
fn orchestrator_exit_adds_the_attention_line_and_suspends_wakes() {
    let mut steps = working_until_typed();
    steps.push(json!({"exit": 0}));
    let (h, run, window) = started(&steps);
    h.type_into(window, b"go\r");
    let line = format!(
        "the orchestrator (window {window}) exited; restart it with anthrex restart {window}"
    );
    let info = h.wait_run(&run, |r| r.attention.contains(&line), ORCH_WAIT);
    assert!(!info.orchestrator.unwrap().live);
    // The run goes on; a change adds a note, and nothing is pasted anywhere.
    h.add_task(&run, "t1");
    std::thread::sleep(Duration::from_millis(2_500));
    let info = h.run(&run).unwrap();
    let o = info.orchestrator.unwrap();
    assert_eq!(o.wakes, 0);
    assert!(
        o.notes
            .iter()
            .any(|n| n.contains("the user edited the plan")),
        "{:?}",
        o.notes
    );
    assert_eq!(info.state, proto::RunState::Planning);
}

/// M9.13 review, item 3: the user's `anthrex restart <n>` of the orchestrator is not
/// its exit. At no point is it reported exited: no attention line, no log line, and it
/// stays live, so a wake-up would not be dropped.
#[test]
fn a_manual_restart_of_the_orchestrator_is_not_an_exit() {
    let steps = [
        json!({"hook": "UserPromptSubmit", "payload": {"prompt": "plan"}}),
        read_message(),
        read_message(),
    ];
    let h = RunHarness::orch("", &[]);
    h.script(ORCH, &steps);
    let run = h.start_goal_id("rework storage", &[]);
    let window = h.orchestrator_window(&run);
    h.wait_window(window, "a session", |w| w.session_id.is_some(), ORCH_WAIT);
    let line = format!(
        "the orchestrator (window {window}) exited; restart it with anthrex restart {window}"
    );
    let out = h.anthrex(&["restart", &window.to_string()]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let until = Instant::now() + Duration::from_secs(4);
    while Instant::now() < until {
        let info = h.run(&run).unwrap();
        assert!(!info.attention.contains(&line), "{:?}", info.attention);
        assert!(info.orchestrator.unwrap().live, "reported not live");
        std::thread::sleep(Duration::from_millis(50));
    }
    // A user edit persists the run at once; its log never says the window exited.
    h.add_task(&run, "t1");
    let exited = format!("the orchestrator's window {window} exited");
    let deadline = Instant::now() + REQUEST_WAIT;
    loop {
        let json = h.run_json(&run);
        let log: Vec<String> = json["log"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["text"].as_str().unwrap_or_default().to_string())
            .collect();
        assert!(!log.iter().any(|l| l.contains(&exited)), "{log:#?}");
        if json["tasks"].as_array().is_some_and(|t| !t.is_empty()) {
            break;
        }
        assert!(Instant::now() < deadline, "the edit was not persisted");
        std::thread::sleep(Duration::from_millis(100));
    }
}
