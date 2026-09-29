//! A daemon restart resumes the orchestrator with every role flag (decision 11), and a
//! Claude orchestrator is metered through M8b's OTLP receiver while a Codex one is not
//! (decisions 14 and 14a); the orchestrator never sees `ANTHROPIC_BASE_URL` (user
//! ruling 2026-09-29).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use proto::{RunState, TaskState};
use serde_json::json;

use crate::common::*;
use crate::support::orch_script::*;
use crate::support::run_adapt::{ADAPT_FILES, STORED_PROFILE};
use crate::support::run_harness::{REQUEST_WAIT, RunHarness};
use crate::support::run_orch::{ORCH_LINES, ORCH_WAIT, triage_plan};
use crate::support::run_plans::{approve, commit, done, report_with, until as poll};

/// The role flags of a Claude orchestrator's launch that take a value (decision 7).
const ROLE_FLAGS: &[&str] = &[
    "--settings",
    "--mcp-config",
    "--allowedTools",
    "--disallowedTools",
    "--append-system-prompt",
];

/// Each orchestrator process's argv, once `n` have started.
fn argvs(h: &RunHarness, n: usize) -> Vec<Vec<String>> {
    poll("the orchestrator's argv lines", REQUEST_WAIT, || {
        let lines: Vec<Vec<String>> = h
            .io_lines(ORCH, "args")
            .iter()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        (lines.len() >= n).then_some(lines)
    })
}

/// The value after `flag` in `argv`.
fn value<'a>(argv: &'a [String], flag: &str) -> Option<&'a str> {
    let at = argv.iter().position(|a| a == flag)?;
    argv.get(at + 1).map(String::as_str)
}

#[test]
fn e2e_restart_resumes_the_orchestrator_with_its_role_flags() {
    let mut h = harness("");
    let go = h.dir.path().join("go");
    h.script(
        "worker-t1-1",
        &[wait_file(&go), commit("a.txt", "t1\n"), done("added a.txt")],
    );
    h.script("reviewer-t1-1", &[approve()]);
    let steps = [
        prompt(),
        edit_plan(
            vec![add(plan_task("t1", &["a.txt"], json!({})))],
            json!({"submit": true}),
        ),
        until("/gate/state", json!("approved"), ORCH_WAIT),
        read(Some("the daemon restarted")),
        marker(),
        read(None),
    ];
    let (run, window) = start(&h, &steps);
    approve_plan(&h, &run);
    wait_task(&h, &run, "t1", TaskState::Working);
    // The orchestrator's turn has ended: it waits in `read_message`, with a session id
    // a restart resumes.
    h.wait_orchestrator_idle(window);
    h.wait_window(window, "a session", |w| w.session_id.is_some(), ORCH_WAIT);
    assert!(h.read_messages(ORCH).is_empty());

    h.restart_daemon(&[]);
    h.wait_run(&run, |r| r.state == RunState::Paused, REQUEST_WAIT);
    ok(&h.anthrex(&["run", "resume", &run]));
    let info = h.wait_run(
        &run,
        |r| r.orchestrator.as_ref().is_some_and(|o| o.live),
        ORCH_WAIT,
    );
    assert_eq!(info.orchestrator.unwrap().window_id, Some(window));

    // A second process, resumed, with every role flag and value of the first.
    let argvs = argvs(&h, 2);
    assert_eq!(argvs.len(), 2, "{argvs:?}");
    let (first, resumed) = (&argvs[0], &argvs[1]);
    assert!(value(resumed, "--resume").is_some(), "{resumed:?}");
    for flag in ROLE_FLAGS {
        assert!(
            value(first, flag).is_some(),
            "{flag} missing at launch: {first:?}"
        );
        assert_eq!(value(resumed, flag), value(first, flag), "{flag}");
    }
    for flag in first.iter().filter(|a| a.starts_with("--") && *a != "--") {
        assert!(resumed.contains(flag), "{flag} lost on resume: {resumed:?}");
    }
    // Its first wake-up says why (decisions 11 and 39).
    h.wait_messages(ORCH, 1, ORCH_WAIT);
    let first_wake = h.read_messages(ORCH)[0]["text"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(
        first_wake.contains("the daemon restarted and your session was resumed"),
        "{first_wake}"
    );
    wait_passed(&h, 1);
    let info = h.run(&run).unwrap();
    assert_eq!(info.state, RunState::Running);
}

/// A `claude` and `codex` stand-in that writes its environment to
/// `<dir>/env-<window id>.txt`, then runs `fake-agent`. The file is written under
/// another name and renamed, so [`env_of`] never reads it half-written.
fn env_recorder(dir: &Path) -> PathBuf {
    let wrapper = dir.join("agent-env.sh");
    std::fs::write(
        &wrapper,
        format!(
            "#!/bin/sh\nif [ -n \"$ANTHREX_WINDOW_ID\" ]; then out='{}/env-'\"$ANTHREX_WINDOW_ID\"; env > \"$out.tmp\" && mv \"$out.tmp\" \"$out.txt\"; fi\nexec '{}' \"$@\"\n",
            dir.display(),
            crate::support::fake_agent_bin().display()
        ),
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755)).unwrap();
    wrapper
}

/// The environment window `window`'s agent started with.
fn env_of(dir: &Path, window: u32) -> BTreeMap<String, String> {
    let path = dir.join(format!("env-{window}.txt"));
    let text = poll("the orchestrator's environment", REQUEST_WAIT, || {
        std::fs::read_to_string(&path).ok()
    });
    text.lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

const OTEL_KEYS: &[&str] = &[
    "CLAUDE_CODE_ENABLE_TELEMETRY",
    "OTEL_METRICS_EXPORTER",
    "OTEL_EXPORTER_OTLP_PROTOCOL",
    "OTEL_EXPORTER_OTLP_ENDPOINT",
    "OTEL_METRIC_EXPORT_INTERVAL",
    "OTEL_RESOURCE_ATTRIBUTES",
    "OTEL_EXPORTER_OTLP_HEADERS",
];

#[test]
fn e2e_claude_orchestrator_gets_the_otlp_environment() {
    let recorded = crate::support::tempdir();
    let wrapper = env_recorder(recorded.path());
    let wrapper = wrapper.to_str().unwrap();
    let endpoint = "http://127.0.0.1:9/never-used";
    let h = RunHarness::adapt(
        "claude",
        ORCH_LINES,
        &[
            ("ANTHREX_CLAUDE_BIN", wrapper),
            ("ANTHREX_CODEX_BIN", wrapper),
            ("ANTHROPIC_BASE_URL", endpoint),
        ],
        ADAPT_FILES,
    );
    h.stored_profile(STORED_PROFILE);
    h.decider("triage", 1, triage_plan());
    h.decider("triage", 2, triage_plan());
    for n in [1, 2] {
        h.script(&format!("orchestrator-run-{n}"), &[prompt(), read(None)]);
    }
    // M8b's receiver is up before the goal starts.
    let addr_file = h.data().join("otlp.addr");
    let addr = poll("otlp.addr", REQUEST_WAIT, || {
        std::fs::read_to_string(&addr_file).ok()
    });
    let addr = addr.trim();

    let run = h.start_goal_id("add the files", &["--orchestrator", "claude"]);
    let window = h.orchestrator_window(&run);
    let env = env_of(recorded.path(), window);
    let token = h.run_json(&run)["orch"]["orchestrator"]["otlp_token"]
        .as_str()
        .unwrap()
        .to_string();
    let expected: BTreeMap<String, String> = daemon::launch::role::otlp_env(addr, &run, &token)
        .into_iter()
        .collect();
    for (key, value) in &expected {
        assert_eq!(env.get(key), Some(value), "{key}");
    }
    assert!(
        env["OTEL_RESOURCE_ATTRIBUTES"].contains("anthrex.role=orchestrator"),
        "{:?}",
        env.keys()
    );
    assert_eq!(
        env["OTEL_EXPORTER_OTLP_HEADERS"],
        format!("authorization=Bearer {token}")
    );
    assert_eq!(
        env.get("ENABLE_TOOL_SEARCH").map(String::as_str),
        Some("false")
    );
    // The daemon's own environment reached the window, but not its API endpoint.
    assert!(env.contains_key("FAKE_AGENT_MCP_LOG"), "{:?}", env.keys());
    assert!(!env.contains_key("ANTHROPIC_BASE_URL"), "{:?}", env.keys());

    // A Codex orchestrator gets none of it, and is not metered.
    let codex = h.start_goal_id("add the files", &["--orchestrator", "codex"]);
    let codex_window = h.orchestrator_window(&codex);
    let env = env_of(recorded.path(), codex_window);
    for key in OTEL_KEYS {
        assert!(!env.contains_key(*key), "{key} in {:?}", env.keys());
    }
    assert!(!env.contains_key("ANTHROPIC_BASE_URL"), "{:?}", env.keys());
    let status = h.status_json(&codex);
    assert_eq!(status["orchestrator"]["route"]["runtime"], "codex");
    let usage = &status["usage"]["by_role"]["orchestrator"];
    assert!(
        usage.is_null() || usage.as_object().unwrap().values().all(|v| v == 0),
        "{usage}"
    );
    // The report says so (decision 14); the Claude run's does not. A planning run's
    // report is written when the run ends, so both are rejected first.
    for id in [&codex, &run] {
        ok(&h.anthrex(&["run", "reject", id, "--confirm", id]));
        h.wait_run(id, |r| r.state == proto::RunState::Discarded, REQUEST_WAIT);
    }
    let line = "\norchestrator usage: not metered (codex)\n";
    report_with(&h.run(&codex).unwrap(), line);
    let claude_report = report_with(&h.run(&run).unwrap(), "# anthrex run");
    assert!(
        !claude_report.contains("orchestrator usage"),
        "{claude_report}"
    );
}
