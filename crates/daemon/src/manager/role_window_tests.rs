//! Milestone 9.3 task 6b, decision 23: a continued run adopts its chain's orchestrator
//! window, `rebind_run_window`. A real manager and a real PTY window whose `claude` is a
//! stand-in that sleeps; the test kills only the window it made.

use std::path::Path;

use proto::{AgentRole, Effort, RunRef, Runtime, WindowSpec};

use super::super::{ManagerConfig, WindowManager};
use crate::headless::McpTarget;
use crate::launch::LaunchGate;
use crate::launch::role::RoleLaunch;

fn run_ref(run_id: &str) -> RunRef {
    RunRef {
        run_id: run_id.into(),
        task_id: None,
        role: AgentRole::Orchestrator,
        session: 1,
        lane: None,
    }
}

fn role(run_id: &str) -> RoleLaunch {
    RoleLaunch {
        run_ref: run_ref(run_id),
        mcp: McpTarget {
            role: AgentRole::Orchestrator,
            run_id: run_id.into(),
            task_id: None,
            scout_id: None,
            epic: None,
            chain: Some("o-3f9a".into()),
            lane: None,
            agent_label: None,
        },
        instructions: "the orchestrator contract".into(),
        effort: Effort::HIGH,
        claude_allowed_tools: Vec::new(),
        claude_disallowed_tools: Vec::new(),
        env: Vec::new(),
        remove_env: Vec::new(),
    }
}

/// A manager whose `claude` is a stand-in that sleeps.
fn manager(dir: &Path) -> std::sync::Arc<WindowManager> {
    let claude = dir.join("claude");
    testexec::write_executable(&claude, "#!/bin/sh\nexec sleep 300\n");
    let mut config = ManagerConfig::for_tests(dir.join("d.sock"), "/bin/sh".into());
    config.claude_bin = claude.to_str().unwrap().into();
    config.worktrees_root = dir.join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    WindowManager::new(config).0
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rebind_moves_the_live_flag_to_the_new_run() {
    let dir = tempfile::Builder::new()
        .prefix("anthrex-rebind-")
        .tempdir_in("/tmp")
        .unwrap();
    let manager = manager(dir.path());
    let spec = WindowSpec {
        name: Some("3f9a/orchestrator".into()),
        runtime: Runtime::Claude,
        cwd: dir.path().to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    let id = manager
        .create_run_window(spec, dir.path().to_path_buf(), role("first-3f9a"))
        .await
        .expect("the stand-in's window")
        .id;
    manager.set_run_window_live(id, true);

    let next = run_ref("next-4c1d");
    manager.rebind_run_window(id, next.clone()).unwrap();
    let live = manager.run_window_live(id);
    // The ended run's release no longer reaches the window.
    manager.end_run_window(id, "first-3f9a");
    let after_old = manager.run_window_live(id);
    // The persisted record names the new run, its MCP target unchanged.
    let record = crate::lock(&manager.inner).entries[&id].run.clone();
    // The new run's own end frees it.
    manager.end_run_window(id, "next-4c1d");
    let after_new = manager.run_window_live(id);
    let missing = manager.rebind_run_window(id + 1000, next.clone());
    let _ = manager.kill(id);

    assert_eq!(live, Some(next.clone()));
    assert_eq!(after_old, Some(next));
    let record = record.expect("a run window's record");
    assert_eq!(
        record.pointer("/role_launch/run_ref/run_id"),
        Some(&serde_json::json!("next-4c1d")),
        "{record}"
    );
    assert_eq!(
        record.pointer("/role_launch/mcp/chain"),
        Some(&serde_json::json!("o-3f9a"))
    );
    assert_eq!(after_new, None);
    assert_eq!(
        missing.unwrap_err().to_string(),
        format!("no window with id {}", id + 1000)
    );
}

/// The `a_waiting_wake_logs_why_once` flake (Ubuntu CI on 323d2564, `(Starting, false)`
/// after 5 s): a run window's stand-in printed its one line before phase C inserted the
/// entry, `handle_event` dropped that `Output`, and the window stayed `Starting` for good.
/// `create` had the fix (b8050c87) and tests that build its phases by hand
/// (`early_events_tests`); this drives `create_run_window` itself, held between phase B
/// and phase C while the test hands every event up to the first `Output` to
/// `handle_event`. No timing luck: the `Output` is handled before `insert` every time.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_run_windows_output_before_insert_is_applied_at_insert() {
    use crate::window::WindowEvent;
    use proto::Status;
    let dir = tempfile::Builder::new()
        .prefix("anthrex-early-run-")
        .tempdir_in("/tmp")
        .unwrap();
    let claude = dir.path().join("claude");
    testexec::write_executable(&claude, "#!/bin/sh\necho starting\nexec sleep 300\n");
    let mut config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    config.claude_bin = claude.to_str().unwrap().into();
    config.worktrees_root = dir.path().join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    let (manager, mut events) = WindowManager::new(config);
    let spec = WindowSpec {
        name: Some("r1/orchestrator".into()),
        runtime: Runtime::Claude,
        cwd: dir.path().to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    let release = manager.hold_next_run_window();
    let creating = tokio::spawn({
        let manager = manager.clone();
        let project = dir.path().to_path_buf();
        async move { manager.create_run_window(spec, project, role("r1")).await }
    });
    let mut seen = Vec::new();
    loop {
        let (id, event) =
            tokio::time::timeout(super::super::early_events_tests::EVENT_WAIT, events.recv())
                .await
                .unwrap_or_else(|_| panic!("the stand-in never printed; saw {seen:?}"))
                .expect("the event channel stays open while the manager lives");
        let output = matches!(event, WindowEvent::Output);
        seen.push(format!("{event:?}"));
        manager.handle_event(id, event);
        if output {
            break;
        }
    }
    let listed_before_insert = manager.list().len();
    release.send(()).unwrap();
    let info = creating.await.unwrap().expect("the stand-in's window");
    let window = manager
        .list()
        .into_iter()
        .find(|w| w.id == info.id)
        .unwrap();
    let _ = manager.kill(info.id);

    assert_eq!(
        listed_before_insert, 0,
        "inserted before its output: {seen:?}"
    );
    assert_eq!(window.status, Status::Working, "{window:?}; saw {seen:?}");
    assert_eq!(info.status, Status::Working, "{info:?}");
}
