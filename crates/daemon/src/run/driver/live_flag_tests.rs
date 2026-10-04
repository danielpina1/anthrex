//! M9.13a review, item 8 (decisions 11 and 30): a run that has ended frees its
//! orchestrator window on the engine step that ended it, under the engine lock, not
//! when the step's saves are done and its snapshot published. The e2e
//! `create_orchestrator_sets_the_run_live_flag_and_a_terminal_run_clears_it` saw the
//! run `discarded` through `run list` and was still refused the kill.
//!
//! A real manager and a real PTY window whose `claude` is a stand-in that sleeps;
//! the test kills only that window, which it made.

use std::path::Path;
use std::time::{Duration, Instant};

use proto::{AgentRole, Effort, RunRef, Runtime, WindowSpec};
use tokio_util::sync::CancellationToken;

use crate::headless::McpTarget;
use crate::launch::LaunchGate;
use crate::launch::role::RoleLaunch;
use crate::run::driver::*;
use crate::run::orch::test_support::{orchestrator, run_of};

fn role(run_id: &str) -> RoleLaunch {
    RoleLaunch {
        run_ref: RunRef {
            run_id: run_id.into(),
            task_id: None,
            role: AgentRole::Orchestrator,
            session: 1,
            lane: None,
        },
        mcp: McpTarget {
            role: AgentRole::Orchestrator,
            run_id: run_id.into(),
            task_id: None,
            scout_id: None,
            epic: None,
            chain: None,
            lane: None,
        },
        instructions: "the orchestrator contract".into(),
        effort: Effort::High,
        claude_allowed_tools: Vec::new(),
        claude_disallowed_tools: Vec::new(),
        env: Vec::new(),
        remove_env: Vec::new(),
    }
}

/// A manager whose `claude` is a stand-in that sleeps, and a run service over it.
fn service(dir: &Path) -> (Arc<WindowManager>, Arc<RunService>) {
    let claude = dir.join("claude");
    std::fs::write(&claude, "#!/bin/sh\nexec sleep 300\n").unwrap();
    std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut config = ManagerConfig::for_tests(dir.join("d.sock"), "/bin/sh".into());
    config.claude_bin = claude.to_str().unwrap().into();
    config.worktrees_root = dir.join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    let (manager, _events) = WindowManager::new(config);
    let git = crate::server::GitWiring::new(config::Git {
        enabled: false,
        ..config::Git::default()
    });
    let ctx = RunContext::new(
        dir.join("data"),
        manager.config(),
        config::Orchestrator::default(),
        git.registry.clone(),
    );
    let runs = RunService::new(manager.clone(), ctx);
    (manager, runs)
}

/// Run `run_id`'s orchestrator window, live.
async fn live_window(manager: &WindowManager, dir: &Path, run_id: &str) -> u32 {
    let spec = WindowSpec {
        name: Some(format!("{run_id}/orchestrator")),
        runtime: Runtime::Claude,
        cwd: dir.to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    let window = manager
        .create_run_window(spec, dir.to_path_buf(), role(run_id))
        .await
        .expect("the stand-in's window")
        .id;
    manager.set_run_window_live(window, true);
    window
}

/// A run `id` that has just ended, whose orchestrator record names `window`: the step
/// that ended it is the next one the engine takes.
fn ended(runs: &RunService, dir: &Path, id: &str, window: u32) {
    let mut run = run_of(1);
    run.id = id.into();
    run.data_dir = dir.join("data");
    run.root = dir.to_path_buf();
    run.state = proto::RunState::Discarded;
    let mut record = orchestrator();
    record.window_id = Some(window);
    run.orch.orchestrator = Some(record);
    crate::lock(&runs.state).runs.insert(id.into(), run);
}

async fn wait_freed(manager: &WindowManager, window: u32) -> Option<RunRef> {
    let deadline = Instant::now() + Duration::from_secs(10);
    while manager.run_window_live(window).is_some() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    manager.run_window_live(window)
}

fn tempdir() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("anthrex-live-flag-")
        .tempdir_in("/tmp")
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ended_run_frees_its_orchestrator_before_its_saves_finish() {
    let dir = tempdir();
    let (manager, runs) = service(dir.path());
    let window = live_window(&manager, dir.path(), "ended-run").await;
    ended(&runs, dir.path(), "ended-run", window);

    // Its saves stall: whatever waits on them, the step itself does not.
    let slot = runs.writes.slot("ended-run");
    let (locked_tx, locked) = std::sync::mpsc::channel();
    let (release, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let _held = crate::lock(&slot);
        locked_tx.send(()).unwrap();
        let _ = release_rx.recv();
    });
    locked.recv().unwrap();

    let shutdown = CancellationToken::new();
    let service = runs.spawn(shutdown.clone());
    // The service's first step (its tick) sees the ended run.
    let live = wait_freed(&manager, window).await;

    let _ = release.send(());
    holder.join().unwrap();
    shutdown.cancel();
    runs.stop().await;
    let _ = service.await;
    let _ = manager.kill(window);
    assert_eq!(live, None, "the ended run's orchestrator is a plain window");
}

/// M9.13a re-review, item 5: after a daemon restart a window id can belong to another
/// run's window. An ended run whose record still names that id leaves it alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ended_run_leaves_another_runs_window_with_its_old_id_alone() {
    let dir = tempdir();
    let (manager, runs) = service(dir.path());
    let mine = live_window(&manager, dir.path(), "ended-run").await;
    let theirs = live_window(&manager, dir.path(), "live-run").await;
    ended(&runs, dir.path(), "ended-run", mine);
    // An older ended run, whose orchestrator had the id `theirs` has now.
    ended(&runs, dir.path(), "old-run", theirs);

    let shutdown = CancellationToken::new();
    let service = runs.spawn(shutdown.clone());
    // Once the ended run's own window is freed, the engine has taken a step.
    let freed = wait_freed(&manager, mine).await;
    let other = manager.run_window_live(theirs);

    shutdown.cancel();
    runs.stop().await;
    let _ = service.await;
    let _ = manager.kill(mine);
    let _ = manager.kill(theirs);
    assert_eq!(freed, None);
    assert_eq!(other.map(|r| r.run_id), Some("live-run".to_string()));
}

/// The final fix wave (review A, I3; milestone 9.3 D17, decision 19): a delivered `pr`
/// run (complete, every PR landed) is finished for its chain, so its idle
/// orchestrator's window is released as an accepted run's is, and `C-b x` or the idle
/// menu's `close` can kill it. An iterate that makes the run planning again protects
/// the window again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_delivered_runs_idle_window_is_released_until_an_iterate() {
    let dir = tempdir();
    let (manager, runs) = service(dir.path());
    let window = live_window(&manager, dir.path(), "delivered-run").await;
    ended(&runs, dir.path(), "delivered-run", window);
    {
        let mut state = crate::lock(&runs.state);
        let run = state.runs.get_mut("delivered-run").unwrap();
        run.state = proto::RunState::Complete;
        run.delivery.mode = proto::DeliveryMode::Pr;
        assert!(crate::run::chain::delivered(run));
    }

    let shutdown = CancellationToken::new();
    let service = runs.spawn(shutdown.clone());
    let freed = wait_freed(&manager, window).await;
    // The iterate's effect on the run (the engine's own path is pinned by
    // `goal_rounds_start`): planning again, its orchestrator live.
    {
        let mut state = crate::lock(&runs.state);
        let run = state.runs.get_mut("delivered-run").unwrap();
        run.state = proto::RunState::Planning;
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while manager.run_window_live(window).is_none() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let again = manager.run_window_live(window);

    shutdown.cancel();
    runs.stop().await;
    let _ = service.await;
    let _ = manager.kill(window);
    assert_eq!(
        freed, None,
        "the delivered run's idle window is a plain one"
    );
    assert_eq!(again.map(|r| r.run_id), Some("delivered-run".to_string()));
}
