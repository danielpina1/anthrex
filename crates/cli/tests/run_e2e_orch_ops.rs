//! Milestone 9 task M9.13: the driver executes the orchestrator's ops through a real
//! daemon, with `fake-agent` as the orchestrator in its PTY, as the run scout and
//! sub-planner sessions, and as the triage decider: a goal on the plan path starts a
//! planned run (decision 26), `CreateOrchestrator` and `RestartOrchestrator` (decisions
//! 5 and 11) with the run-live flag and the OTLP token (decision 14a), `StartScout`
//! and `StartPlanner` (decisions 20, 31, 32 and 34). No real agent.

mod support;

use std::time::Duration;

use daemon::run::orch::contract::planned_message;
use proto::{PlannerState, RunPath, RunRef, RunState, ScoutState, WindowKind};
use serde_json::{Value, json};
use support::run_harness::{REQUEST_WAIT, RunHarness, git_in};
use support::run_orch::*;

const ORCH: &str = "orchestrator-run-1";

fn stdout(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn stderr(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// A planned run whose orchestrator waits for messages; the harness, run and window.
fn waiting() -> (RunHarness, String, u32) {
    let h = RunHarness::orch("", &[]);
    h.script(
        ORCH,
        &[
            json!({"hook": "UserPromptSubmit", "payload": {"prompt": "plan"}}),
            read_message(),
        ],
    );
    let run = h.start_goal_id("rework storage", &[]);
    let window = h.orchestrator_window(&run);
    // Its hooks have reported its session, which a restart resumes.
    h.wait_window(window, "a session", |w| w.session_id.is_some(), ORCH_WAIT);
    (h, run, window)
}

/// The argv of each orchestrator process, once `n` have started (each writes its own
/// line as it starts), at most [`REQUEST_WAIT`].
fn argvs(h: &RunHarness, n: usize) -> Vec<Vec<String>> {
    let read = || -> Vec<Vec<String>> {
        h.io_lines(ORCH, "args")
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    };
    wait_until("the orchestrator's argv", || read().len() >= n);
    read()
}

/// `anthrex kill <window>`.
fn kill(h: &RunHarness, window: u32) -> std::process::Output {
    h.anthrex(&["kill", &window.to_string()])
}

fn refusal(window: u32, run: &str) -> String {
    format!(
        "window {window} is the orchestrator of run {run}; stop the run with anthrex run cancel, or restart the orchestrator with anthrex restart {window}"
    )
}

#[test]
fn start_goal_on_the_plan_path_builds_a_planned_run() {
    let h = RunHarness::orch("", &[]);
    let out = h.start_goal("rework storage", &[]);
    assert!(out.status.success(), "{}\n{}", stderr(&out), h.log_tail());
    let id = stdout(&out).trim().to_string();
    let run = h.run(&id).expect("the run is listed");
    assert_eq!(run.state, RunState::Planning);
    assert_eq!(run.path, Some(RunPath::Plan));
    assert!(run.tasks.is_empty());
    let triage = run.triage.clone().expect("its triage");
    assert_eq!(
        stderr(&out).trim_end(),
        planned_message(&triage, &id, RunPath::Plan)
    );
    // M8a's start: the run branch exists from the start (decision 26).
    h.orchestrator_window(&id);
    let branch = format!("refs/heads/{}", run.run_branch);
    let listed = || git_in(&h.repo, &["for-each-ref", "--format=%(refname)", &branch]);
    wait_until("the run branch is made", || listed() == branch);
}

#[test]
fn create_orchestrator_op_starts_the_window_and_reports_it() {
    let (h, run, window) = waiting();
    let info = h.window(window).expect("the window is listed");
    assert_eq!(info.kind, WindowKind::Pty);
    assert_eq!(info.name, format!("{}/orchestrator", &run[run.len() - 4..]));
    assert_eq!(
        info.run,
        Some(RunRef {
            run_id: run.clone(),
            task_id: None,
            role: proto::AgentRole::Orchestrator,
            session: 1,
        })
    );
    let argv = &argvs(&h, 1)[0];
    for flag in ["--mcp-config", "--allowedTools", "--append-system-prompt"] {
        assert!(argv.iter().any(|a| a == flag), "{flag}: {argv:?}");
    }
    let o = h.run(&run).unwrap().orchestrator.unwrap();
    assert!(o.live);
    assert_eq!(o.window_id, Some(window));
}

#[test]
fn create_orchestrator_op_counts_toward_max_windows() {
    let (h, run, _) = waiting();
    assert_eq!(h.run_json(&run)["windows_created"], 1);
}

#[test]
fn create_orchestrator_sets_the_run_live_flag_and_a_terminal_run_clears_it() {
    let (h, run, window) = waiting();
    let out = kill(&h, window);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains(&refusal(window, &run)),
        "{}",
        stderr(&out)
    );
    // Decision 30: a rejected plan discards the run; its window is a plain one.
    let reject = h.anthrex(&["run", "reject", &run, "--confirm", &run]);
    assert!(reject.status.success(), "{}", stderr(&reject));
    h.wait_run(&run, |r| r.state == RunState::Discarded, REQUEST_WAIT);
    let out = kill(&h, window);
    assert!(out.status.success(), "{}", stderr(&out));
}

#[test]
fn restore_sets_the_run_live_flag_for_a_non_terminal_run() {
    let (mut h, run, window) = waiting();
    h.restart_daemon(&[]);
    // Restored dormant, with its run paused: still protected.
    h.wait_run(&run, |r| r.state == RunState::Paused, REQUEST_WAIT);
    let out = kill(&h, window);
    assert!(!out.status.success());
    assert!(
        stderr(&out).contains(&refusal(window, &run)),
        "{}",
        stderr(&out)
    );
}

#[test]
fn restart_orchestrator_op_resumes_it() {
    let (mut h, run, window) = waiting();
    h.restart_daemon(&[]);
    h.wait_run(&run, |r| r.state == RunState::Paused, REQUEST_WAIT);
    let resume = h.anthrex(&["run", "resume", &run]);
    assert!(resume.status.success(), "{}", stderr(&resume));
    let info = h.wait_run(
        &run,
        |r| r.orchestrator.as_ref().is_some_and(|o| o.live),
        ORCH_WAIT,
    );
    assert_eq!(info.orchestrator.unwrap().window_id, Some(window));
    // The same window, relaunched with its role and its session resumed.
    let argvs = argvs(&h, 2);
    assert_eq!(argvs.len(), 2, "{argvs:?}");
    let resumed = &argvs[1];
    assert!(resumed.iter().any(|a| a == "--resume"), "{resumed:?}");
    assert!(resumed.iter().any(|a| a == "--mcp-config"), "{resumed:?}");
    let out = kill(&h, window);
    assert!(!out.status.success(), "the restarted window is protected");
}

/// Decision 14a: the token is 32 hex characters in `run.json`, the orchestrator's
/// OTLP header carries it (its persisted role), and a daemon restart and the resumed
/// orchestrator keep it.
#[test]
fn orchestrator_token_is_persisted_and_survives_a_restart() {
    let (mut h, run, _) = waiting();
    let token = h.run_json(&run)["orch"]["orchestrator"]["otlp_token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(token.len(), 32, "{token}");
    assert!(
        token
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
    );
    let header = format!("authorization=Bearer {token}");
    let state = h.data().join("state.json");
    let persisted = || {
        std::fs::read_to_string(&state)
            .unwrap_or_default()
            .contains(&header)
    };
    wait_until("the role's header is persisted", persisted);
    // Never in the snapshot.
    let listed = serde_json::to_string(&h.snapshot()).unwrap();
    assert!(!listed.contains(&token));

    h.restart_daemon(&[]);
    h.wait_run(&run, |r| r.state == RunState::Paused, REQUEST_WAIT);
    let resume = h.anthrex(&["run", "resume", &run]);
    assert!(resume.status.success(), "{}", stderr(&resume));
    h.wait_run(
        &run,
        |r| r.orchestrator.as_ref().is_some_and(|o| o.live),
        ORCH_WAIT,
    );
    assert_eq!(
        h.run_json(&run)["orch"]["orchestrator"]["otlp_token"],
        token.as_str()
    );
    wait_until("the restarted role keeps the header", persisted);
}

/// The OTLP endpoint in the persisted role of the orchestrator window `window`.
fn persisted_endpoint(h: &RunHarness, window: u32) -> Option<String> {
    let text = std::fs::read_to_string(h.data().join("state.json")).ok()?;
    let state: Value = serde_json::from_str(&text).ok()?;
    let windows = state["windows"].as_array()?;
    let w = windows.iter().find(|w| w["id"] == json!(window))?;
    let env = w["run"]["role_launch"]["env"].as_array()?;
    env.iter()
        .find(|pair| pair[0] == "OTEL_EXPORTER_OTLP_ENDPOINT")
        .and_then(|pair| pair[1].as_str())
        .map(str::to_string)
}

/// The receiver's address as the daemon now publishes it.
fn otlp_addr(h: &RunHarness) -> String {
    let path = h.data().join("otlp.addr");
    wait_until("otlp.addr", || path.exists());
    std::fs::read_to_string(path).unwrap().trim().to_string()
}

/// M9.13 review, item 1 (a correction to decision 14a's "re-passes the same
/// `RoleLaunch.env`"): after a daemon restart the receiver listens on a new port, and
/// `run resume` restarts the orchestrator with the endpoint of the receiver that is up
/// now, not the old one, and the same token.
#[test]
fn a_restarted_orchestrator_names_the_receiver_that_is_up_now() {
    let (mut h, run, window) = waiting();
    let first = otlp_addr(&h);
    wait_until("the role names the first receiver", || {
        persisted_endpoint(&h, window).as_deref() == Some(first.as_str())
    });
    let token = h.run_json(&run)["orch"]["orchestrator"]["otlp_token"]
        .as_str()
        .unwrap()
        .to_string();

    // The old address file goes, so the one read below is the new daemon's.
    let _ = std::fs::remove_file(h.data().join("otlp.addr"));
    h.restart_daemon(&[]);
    let second = otlp_addr(&h);
    assert_ne!(first, second, "the receiver took the same port again");
    h.wait_run(&run, |r| r.state == RunState::Paused, REQUEST_WAIT);
    let resume = h.anthrex(&["run", "resume", &run]);
    assert!(resume.status.success(), "{}", stderr(&resume));
    h.wait_run(
        &run,
        |r| r.orchestrator.as_ref().is_some_and(|o| o.live),
        ORCH_WAIT,
    );
    wait_until("the restarted role names the new receiver", || {
        persisted_endpoint(&h, window).as_deref() == Some(second.as_str())
    });
    let state = std::fs::read_to_string(h.data().join("state.json")).unwrap();
    assert!(state.contains(&format!("authorization=Bearer {token}")));
    assert!(
        !state.contains(&first),
        "the old endpoint is still persisted"
    );
}

/// M9.13 re-review, item 1: the user's own `anthrex restart <n>` of an orchestrator
/// restored after a daemon restart launches it with the receiver that is up now, not
/// the one it had before. The daemon's `claude` is a wrapper that records the endpoint
/// in its real environment, then runs `fake-agent`.
#[test]
fn a_manual_restart_after_a_daemon_restart_names_the_receiver_that_is_up_now() {
    let (mut h, run, window) = waiting();
    let first = otlp_addr(&h);
    let seen = h.data().join("orch-endpoints.txt");
    let wrapper = h.data().join("claude-wrapper.sh");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$OTEL_EXPORTER_OTLP_ENDPOINT\" >> '{}'\nexec '{}' \"$@\"\n",
            seen.display(),
            support::fake_agent_bin().display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();

    let _ = std::fs::remove_file(h.data().join("otlp.addr"));
    h.restart_daemon(&[("ANTHREX_CLAUDE_BIN", wrapper.to_str().unwrap())]);
    let second = otlp_addr(&h);
    assert_ne!(first, second, "the receiver took the same port again");
    h.wait_run(&run, |r| r.state == RunState::Paused, REQUEST_WAIT);

    let out = h.anthrex(&["restart", &window.to_string()]);
    assert!(out.status.success(), "{}", stderr(&out));
    wait_until("the restarted orchestrator started", || seen.exists());
    let endpoints = std::fs::read_to_string(&seen).unwrap();
    assert_eq!(endpoints.lines().collect::<Vec<_>>(), vec![second.as_str()]);
}

fn wait_until(what: &str, pred: impl Fn() -> bool) {
    let deadline = std::time::Instant::now() + REQUEST_WAIT;
    while !pred() {
        assert!(std::time::Instant::now() < deadline, "{what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn start_scout_op_runs_a_scout_and_sends_scout_ended() {
    let h = RunHarness::orch("", &[]);
    h.script(
        ORCH,
        &[
            json!({"mcp_call": {"tool": "spawn_scout", "args": {
                "id": "api", "question": "Where is the check?", "area": ["tests/**"]}}}),
            read_message(),
        ],
    );
    h.script(
        "scout-api-1",
        &[json!({"mcp_call": {"tool": "submit_scout_report", "args": {
            "summary": "check.sh runs the check.",
            "files": [{"path": "check.sh", "why": "the check"}]}}})],
    );
    let run = h.start_goal_id("rework storage", &[]);
    let scout = format!("{}-api", &run[run.len() - 4..]);
    let json = wait_json(&h, &run, |j| {
        j["scout_reports"].as_array().is_some_and(|r| !r.is_empty())
    });
    assert_eq!(json["scout_reports"], json!([scout]));
    assert_eq!(
        json["orch"]["run_scouts"][0]["state"], "reported",
        "{}",
        json["orch"]
    );
    let info = h.run(&run).unwrap();
    assert_eq!(info.scouts.len(), 1, "{:?}", info.scouts);
    assert_eq!(info.scouts[0].state, ScoutState::Reported);
    let o = info.orchestrator.unwrap();
    assert!(
        o.notes
            .iter()
            .any(|n| n == &format!("scout {scout} reported"))
            || o.wakes > 0,
        "{o:?}"
    );
}

#[test]
fn start_planner_op_runs_a_planner_and_sends_planner_ended() {
    let h = RunHarness::orch("", &[]);
    h.stored_onboarding_report(json!({"summary": "The mail module lives in mail/."}));
    h.script(
        ORCH,
        &[
            json!({"mcp_call": {"tool": "spawn_subplanner", "args": {
                "epic": "mail", "title": "Mail", "area": ["mail/**"],
                "brief": "Plan the mail work.", "scout_refs": ["onboarding"]}}}),
            read_message(),
        ],
    );
    let task = json!({
        "id": "m1", "title": "Mail one", "brief": "Write it.", "acceptance": ["done"],
        "owns": ["mail/one.txt"], "size": "S", "test_mode": "check",
        "test_mode_reason": "a text file", "scout_refs": ["onboarding"],
    });
    h.script(
        "planner-mail-1",
        &[json!({"mcp_call": {"tool": "submit_epic", "args": {
            "edits": [{"op": "add_task", "task": task}]}}})],
    );
    let run = h.start_goal_id("rework mail", &[]);
    let info = h.wait_run(
        &run,
        |r| {
            r.planners
                .first()
                .is_some_and(|p| p.state == PlannerState::Finished)
        },
        ORCH_WAIT,
    );
    assert!(info.tasks.iter().any(|t| t.id == "m1"), "{:?}", info.tasks);
    // Its session ended (`PlannerEnded`), after its start's result.
    let json = wait_json(&h, &run, |j| {
        !j["orch"]["epics"][0]["sessions"][0]["ended_at"].is_null()
    });
    assert!(json["orch"]["epics"][0]["sessions"][0]["window_id"].is_u64());
    // Decision 34: its first turn carried the epic's scout extract.
    let first = h.io_lines("planner-mail-1", "stdin").join("\n");
    assert!(first.contains("Scout report onboarding:"), "{first}");
    assert!(first.contains("The mail module lives in mail/."), "{first}");
}

/// Polls `run.json` until `pred` holds, at most [`ORCH_WAIT`].
fn wait_json(h: &RunHarness, run: &str, pred: impl Fn(&Value) -> bool) -> Value {
    let deadline = std::time::Instant::now() + ORCH_WAIT;
    loop {
        let json = h.run_json(run);
        if pred(&json) {
            return json;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "run.json never got there: {}\n{}",
            json["orch"],
            h.log_tail()
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}
