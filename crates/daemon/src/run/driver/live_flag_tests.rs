//! M9.13a review, item 8 (decisions 11 and 30): a run that has ended frees its
//! orchestrator window on the engine step that ended it, under the engine lock, not
//! when the step's saves are done and its snapshot published. The e2e
//! `create_orchestrator_sets_the_run_live_flag_and_a_terminal_run_clears_it` saw the
//! run `discarded` through `run list` and was still refused the kill.
//!
//! A real manager and a real PTY window whose `claude` is a stand-in that sleeps;
//! the test kills only that window, which it made.

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
        },
        mcp: McpTarget {
            role: AgentRole::Orchestrator,
            run_id: run_id.into(),
            task_id: None,
            scout_id: None,
            epic: None,
        },
        instructions: "the orchestrator contract".into(),
        effort: Effort::High,
        claude_allowed_tools: Vec::new(),
        claude_disallowed_tools: Vec::new(),
        env: Vec::new(),
        remove_env: Vec::new(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_ended_run_frees_its_orchestrator_before_its_saves_finish() {
    let dir = tempfile::Builder::new()
        .prefix("anthrex-live-flag-")
        .tempdir_in("/tmp")
        .unwrap();
    let claude = dir.path().join("claude");
    std::fs::write(&claude, "#!/bin/sh\nexec sleep 300\n").unwrap();
    std::fs::set_permissions(&claude, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
    let mut config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    config.claude_bin = claude.to_str().unwrap().into();
    config.worktrees_root = dir.path().join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    let (manager, _events) = WindowManager::new(config);
    let git = crate::server::GitWiring::new(config::Git {
        enabled: false,
        ..config::Git::default()
    });
    let data = dir.path().join("data");
    let ctx = RunContext::new(
        data.clone(),
        manager.config(),
        config::Orchestrator::default(),
        git.registry.clone(),
    );
    let runs = RunService::new(manager.clone(), ctx);

    let mut run = run_of(1);
    run.data_dir = data;
    run.root = dir.path().to_path_buf();
    let spec = WindowSpec {
        name: Some("orchestrator".into()),
        runtime: Runtime::Claude,
        cwd: dir.path().to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    let window = manager
        .create_run_window(spec, dir.path().to_path_buf(), role(&run.id))
        .await
        .expect("the stand-in's window")
        .id;
    manager.set_run_window_live(window, true);
    // The run has just ended: the step that ended it is the next one the engine takes.
    run.state = proto::RunState::Discarded;
    let mut record = orchestrator();
    record.window_id = Some(window);
    run.orch.orchestrator = Some(record);
    let run_id = run.id.clone();
    crate::lock(&runs.state).runs.insert(run_id.clone(), run);

    // Its saves stall: whatever waits on them, the step itself does not.
    let slot = runs.writes.slot(&run_id);
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
    let deadline = Instant::now() + Duration::from_secs(10);
    while manager.run_window_live(window).is_some() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let live = manager.run_window_live(window);

    let _ = release.send(());
    holder.join().unwrap();
    shutdown.cancel();
    runs.stop().await;
    let _ = service.await;
    let _ = manager.kill(window);
    assert_eq!(live, None, "the ended run's orchestrator is a plain window");
}
