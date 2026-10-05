//! Milestone 9 decisions 5, 10, 11, 11a and 12 (task M9.10): the orchestrator's PTY
//! window against a real manager, real PTYs and a real socket. The runtime binary is a
//! `/bin/sh` stand-in that records its argv and environment (never a real agent).

mod support;

/// The client refusals and the client's input over a real socket.
#[path = "orchestrator_window/socket.rs"]
mod socket;

use daemon::headless::{McpTarget, credential_scrub_for};
use daemon::launch::role::{ORCHESTRATOR_ALLOWED_TOOLS, ORCHESTRATOR_DISALLOWED_TOOLS, RoleLaunch};
use daemon::manager::{RUN_WINDOW_SIZE, WindowManager, lost_role_refusal};
use daemon::state::WindowRecord;
use proto::{
    AgentRole, Effort, HookSource, RunRef, Runtime, Status, WindowInfo, WindowKind, WindowSpec,
};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use support::headless::{DEADLINE, find, manager, script, wait_until};

const RUN: &str = "r-7a2c";
/// anthrex's own receiver, as the driver puts it in the role's environment.
const ANTHREX_OTLP: &str = "http://127.0.0.1:4318";

/// Edition 2024: tests that change the process environment hold this.
static ENV: Mutex<()> = Mutex::new(());

/// Variables set for one test, under [`ENV`], and put back as they were when it ends,
/// under [`ENV`] again. The lock is not held in between, so it is never held across an
/// `await`.
struct EnvVars(Vec<(&'static str, Option<std::ffi::OsString>)>);

impl EnvVars {
    fn set(vars: &[(&'static str, &str)]) -> EnvVars {
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        let saved = vars
            .iter()
            .map(|(k, _)| (*k, std::env::var_os(k)))
            .collect();
        for (key, value) in vars {
            // SAFETY: under `ENV`; std's own environment lock orders the reads the
            // spawns in other tests make.
            unsafe { std::env::set_var(key, value) };
        }
        EnvVars(saved)
    }
}

impl Drop for EnvVars {
    fn drop(&mut self) {
        let _env = ENV.lock().unwrap_or_else(|e| e.into_inner());
        for (key, value) in &self.0 {
            // SAFETY: as in `set`.
            unsafe {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}

fn role() -> RoleLaunch {
    RoleLaunch {
        run_ref: RunRef {
            run_id: RUN.into(),
            task_id: None,
            role: AgentRole::Orchestrator,
            session: 1,
            lane: None,
        },
        mcp: McpTarget {
            role: AgentRole::Orchestrator,
            run_id: RUN.into(),
            task_id: None,
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
            agent_label: None,
        },
        instructions: "the orchestrator contract".into(),
        effort: Effort::High,
        claude_allowed_tools: ORCHESTRATOR_ALLOWED_TOOLS
            .iter()
            .map(|t| t.to_string())
            .collect(),
        claude_disallowed_tools: ORCHESTRATOR_DISALLOWED_TOOLS
            .iter()
            .map(|t| t.to_string())
            .collect(),
        env: vec![
            ("ROLE_VAR".into(), "role-value".into()),
            ("OTEL_EXPORTER_OTLP_ENDPOINT".into(), ANTHREX_OTLP.into()),
        ],
        remove_env: credential_scrub_for(Runtime::Claude, config::ClaudeAuth::Login)
            .into_iter()
            .map(String::from)
            .collect(),
    }
}

fn spec(cwd: &Path) -> WindowSpec {
    WindowSpec {
        name: Some("7a2c/orchestrator".into()),
        runtime: Runtime::Claude,
        cwd: cwd.to_path_buf(),
        worktree_branch: None,
        model: Some("opus".into()),
        initial_prompt: Some("plan the goal".into()),
    }
}

/// A stand-in `claude` in `dir` that writes its argv (one per line) to `dir/args` and
/// its environment to `dir/env`, then waits.
fn stand_in(dir: &Path) -> PathBuf {
    script(
        dir,
        "claude",
        &format!(
            "printf '%s\\n' \"$@\" > '{d}/args.tmp'; mv '{d}/args.tmp' '{d}/args'\nenv > '{d}/env.tmp'; mv '{d}/env.tmp' '{d}/env'\nexec sleep 300",
            d = dir.display()
        ),
    )
}

fn lines(path: &Path) -> Vec<String> {
    std::fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .map(String::from)
        .collect()
}

async fn wait_for_args(dir: &Path, what: &str, pred: impl Fn(&[String]) -> bool) -> Vec<String> {
    let path = dir.join("args");
    wait_until(what, || pred(&lines(&path))).await;
    lines(&path)
}

fn has_pair(args: &[String], flag: &str, value: &str) -> bool {
    args.windows(2).any(|w| w[0] == flag && w[1] == value)
}

async fn run_window(m: &WindowManager, dir: &Path) -> WindowInfo {
    m.create_run_window(spec(dir), dir.to_path_buf(), role())
        .await
        .expect("create_run_window")
}

fn session_start(m: &WindowManager, id: u32, session: &str) {
    m.handle_hook(
        id,
        HookSource::Claude,
        &json!({"hook_event_name": "SessionStart", "session_id": session, "source": "startup"}),
    )
    .unwrap();
}

/// Decision 5: a PTY window naming its run and role, 200 × 50 until a client resizes
/// it, in the run's root, launched with the role's flags. (It counts toward the run's
/// `max_windows` in the engine: `engine/tests/orch.rs`, M9.7.)
#[tokio::test]
async fn run_window_is_pty_with_run_ref_and_counts_to_max_windows() {
    let dir = tempfile::tempdir().unwrap();
    let claude = stand_in(dir.path());
    let m = manager(&claude, &claude, |_| {});
    let info = run_window(&m, dir.path()).await;
    assert_eq!(info.kind, WindowKind::Pty);
    assert_eq!(info.run, Some(role().run_ref));
    assert_eq!(info.name, "7a2c/orchestrator");
    assert_eq!(info.cwd, dir.path());
    assert_eq!(find(&m, info.id).run, Some(role().run_ref));
    let attached = m.attach(info.id).unwrap();
    assert_eq!((attached.cols, attached.rows), RUN_WINDOW_SIZE);
    assert_eq!(RUN_WINDOW_SIZE, (200, 50));
    let args = wait_for_args(dir.path(), "the args file", |a| !a.is_empty()).await;
    assert!(has_pair(&args, "--permission-mode", "default"), "{args:?}");
    assert!(args.iter().any(|a| a == "--disallowedTools"), "{args:?}");
    assert_eq!(args.last().map(String::as_str), Some("plan the goal"));
    m.remove(info.id).unwrap();
}

/// Decision 10 with M9.1 ruling 5: a daemon started inside a Claude session with an API
/// key does not make the orchestrator a nested session, bill the key, or tune its MCP
/// client; its own identity and `ENABLE_TOOL_SEARCH=false` are set. A plain window in
/// the same environment keeps what it inherits. M9.10 review: an inherited
/// signal-specific `OTEL_*` variable cannot redirect the orchestrator's metrics, and
/// anthrex's own endpoint, set after the scrub, is there.
#[tokio::test]
async fn scrub_removes_agent_session_and_credential_variables() {
    let _vars = EnvVars::set(&[
        ("CLAUDECODE", "1"),
        ("CLAUDE_CODE_ENTRYPOINT", "x"),
        ("ANTHROPIC_API_KEY", "x"),
        ("ANTHROPIC_BASE_URL", "http://127.0.0.1:9"),
        ("OPENAI_BASE_URL", "http://127.0.0.1:9"),
        ("MCP_CONNECTION_NONBLOCKING", "true"),
        ("OTEL_EXPORTER_OTLP_METRICS_ENDPOINT", "http://127.0.0.1:9"),
    ]);
    let dir = tempfile::tempdir().unwrap();
    let claude = stand_in(dir.path());
    let m = manager(&claude, &claude, |_| {});
    let info = run_window(&m, dir.path()).await;
    wait_until("the env file", || dir.path().join("env").exists()).await;
    let env = lines(&dir.path().join("env"));
    for gone in [
        "CLAUDECODE=",
        "CLAUDE_CODE_ENTRYPOINT=",
        "ANTHROPIC_API_KEY=",
        "ANTHROPIC_BASE_URL=",
        "OPENAI_BASE_URL=",
        "MCP_CONNECTION_NONBLOCKING=",
        "OTEL_EXPORTER_OTLP_METRICS_ENDPOINT=",
    ] {
        // Only the offending line is printed: the environment can hold secrets.
        let found: Vec<&String> = env.iter().filter(|l| l.starts_with(gone)).collect();
        assert!(found.is_empty(), "{found:?}");
    }
    for kept in [
        format!("ANTHREX_WINDOW_ID={}", info.id),
        "ENABLE_TOOL_SEARCH=false".to_string(),
        "ROLE_VAR=role-value".to_string(),
        format!("OTEL_EXPORTER_OTLP_ENDPOINT={ANTHREX_OTLP}"),
    ] {
        assert!(env.contains(&kept), "{kept} missing");
    }
    m.remove(info.id).unwrap();

    // The user's own window is unchanged: nothing scrubbed, no tool-search pin.
    let plain_dir = tempfile::tempdir().unwrap();
    let plain = stand_in(plain_dir.path());
    let m = manager(&plain, &plain, |_| {});
    let mut plain_spec = spec(plain_dir.path());
    plain_spec.name = Some("mine".into());
    let info = m
        .create(plain_spec, plain_dir.path().into(), None, 80, 24)
        .await
        .unwrap();
    assert_eq!(info.run, None);
    wait_until("the plain env file", || {
        plain_dir.path().join("env").exists()
    })
    .await;
    let env = lines(&plain_dir.path().join("env"));
    assert!(
        env.contains(&"CLAUDECODE=1".to_string()),
        "CLAUDECODE=1 missing"
    );
    assert!(!env.iter().any(|l| l.starts_with("ENABLE_TOOL_SEARCH=")));
    m.remove(info.id).unwrap();
}

/// Decision 11: a restart re-passes every role flag, with `--resume <id>`.
#[tokio::test]
async fn restart_repasses_role_flags_with_resume() {
    let dir = tempfile::tempdir().unwrap();
    let claude = stand_in(dir.path());
    let m = manager(&claude, &claude, |_| {});
    let info = run_window(&m, dir.path()).await;
    wait_for_args(dir.path(), "the first args", |a| !a.is_empty()).await;
    session_start(&m, info.id, "sess-9");
    wait_until("the session id", || {
        find(&m, info.id).session_id.as_deref() == Some("sess-9")
    })
    .await;
    m.restart(info.id).await.expect("restart");
    let args = wait_for_args(dir.path(), "the restarted args", |a| {
        a.iter().any(|x| x == "--resume")
    })
    .await;
    assert!(has_pair(&args, "--resume", "sess-9"), "{args:?}");
    assert!(args.iter().any(|a| a == "--mcp-config"), "{args:?}");
    assert!(args.iter().any(|a| a == "--disallowedTools"), "{args:?}");
    assert!(!args.iter().any(|a| a == "plan the goal"), "{args:?}");
    assert_eq!(find(&m, info.id).run, Some(role().run_ref));
    m.remove(info.id).unwrap();
}

/// Decision 11: the role is persisted as `{"role_launch": …}` and comes back with the
/// window, so a restart after a daemon restart re-passes the same flags.
#[tokio::test]
async fn role_survives_a_daemon_restart() {
    let dir = tempfile::tempdir().unwrap();
    let claude = stand_in(dir.path());
    let m = manager(&claude, &claude, |_| {});
    let info = run_window(&m, dir.path()).await;
    wait_for_args(dir.path(), "the first args", |a| !a.is_empty()).await;
    session_start(&m, info.id, "sess-4");
    wait_until("the session id", || find(&m, info.id).session_id.is_some()).await;
    let state = m.state_snapshot();
    m.remove(info.id).unwrap();
    let record = state.windows.iter().find(|r| r.id == info.id).unwrap();
    assert_eq!(record.kind, WindowKind::Pty);
    assert_eq!(
        record.run,
        Some(json!({ "role_launch": serde_json::to_value(role()).unwrap() }))
    );

    let restored = manager(&claude, &claude, |_| {});
    restored.restore(state);
    let window = find(&restored, info.id);
    assert_eq!(window.kind, WindowKind::Pty);
    assert_eq!(window.status, Status::Exited);
    assert_eq!(window.run, Some(role().run_ref));
    std::fs::remove_file(dir.path().join("args")).unwrap();
    restored.restart(info.id).await.expect("restart");
    let args = wait_for_args(dir.path(), "the restored args", |a| !a.is_empty()).await;
    assert!(has_pair(&args, "--resume", "sess-4"), "{args:?}");
    assert!(args.iter().any(|a| a == "--mcp-config"), "{args:?}");
    assert!(has_pair(&args, "--permission-mode", "default"), "{args:?}");
    // It is saved again the same way.
    let again = restored.state_snapshot();
    let record = again.windows.iter().find(|r| r.id == info.id).unwrap();
    assert_eq!(
        record.run,
        Some(json!({ "role_launch": serde_json::to_value(role()).unwrap() }))
    );
    restored.remove(info.id).unwrap();
}

/// M9.10 review, the controller's ruling (over decision 11's plain-window fallback): a
/// Pty record whose role does not parse never runs again as a plain agent. It comes back
/// with no session to resume, refuses a restart, and launches nothing.
#[tokio::test]
async fn unparseable_role_restores_a_window_that_never_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let claude = stand_in(dir.path());
    let m = manager(&claude, &claude, |_| {});
    let info = run_window(&m, dir.path()).await;
    wait_for_args(dir.path(), "the first args", |a| !a.is_empty()).await;
    session_start(&m, info.id, "sess-5");
    wait_until("the session id", || find(&m, info.id).session_id.is_some()).await;
    let state = broken_role(m.state_snapshot(), info.id);
    m.remove(info.id).unwrap();

    let restored = manager(&claude, &claude, |_| {});
    restored.restore(state);
    let window = find(&restored, info.id);
    assert_eq!(window.kind, WindowKind::Pty);
    assert_eq!(window.status, Status::Exited);
    assert_eq!(window.session_id, None, "no session to resume");
    std::fs::remove_file(dir.path().join("args")).unwrap();
    let refused = restored
        .restart(info.id)
        .await
        .expect_err("restart is refused");
    assert_eq!(refused.to_string(), lost_role_refusal(info.id, RUN));
    assert!(!dir.path().join("args").exists(), "nothing was launched");
    // It is saved again as it came, still refused after the next restart.
    let again = restored.state_snapshot();
    let record = again.windows.iter().find(|r| r.id == info.id).unwrap();
    assert_eq!(record.session_id, None);
    assert!(
        record
            .run
            .as_ref()
            .is_some_and(|run| run.get("role_launch").is_some())
    );
    restored.remove(info.id).unwrap();
}

/// A snapshot whose window `id` has a role that no longer parses (an unknown effort),
/// with its run still named.
fn broken_role(mut state: daemon::state::StateFile, id: u32) -> daemon::state::StateFile {
    let record: &mut WindowRecord = state.windows.iter_mut().find(|r| r.id == id).unwrap();
    let run = record.run.as_mut().unwrap();
    run["role_launch"]["effort"] = json!("extreme");
    state
}

/// AGENTS.md rule 2: the run window's spawn holds no manager lock. While its launch is
/// held at the gate and while its child starts, `list()` answers at once.
#[tokio::test]
async fn create_run_window_does_not_hold_the_manager_lock_across_spawn() {
    let dir = tempfile::tempdir().unwrap();
    let claude = stand_in(dir.path());
    let gate = daemon::launch::LaunchGate::closed();
    let m: Arc<WindowManager> = manager(&claude, &claude, |c| c.launch_gate = gate.clone());
    let creating = {
        let m = m.clone();
        let dir = dir.path().to_path_buf();
        tokio::spawn(async move { run_window(&m, &dir).await })
    };
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = Instant::now();
    assert!(m.list().is_empty());
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(
        !dir.path().join("args").exists(),
        "spawned before the gate opened"
    );
    gate.open();
    let info = tokio::time::timeout(DEADLINE, creating)
        .await
        .expect("created once the gate opens")
        .unwrap();
    let started = Instant::now();
    let listed = m.list();
    assert!(started.elapsed() < Duration::from_millis(100));
    assert!(listed.iter().any(|w| w.id == info.id));
    m.remove(info.id).unwrap();
}
