//! Milestone 9.3 task 6b, decision 23: a next goal adopts its chain's orchestrator
//! window (`Effect::AdoptOrchestrator`) off every lock: a real manager and a real PTY
//! run window whose `claude` is a stand-in that sleeps (`chain_goal_tests.rs`'
//! helpers); the test kills only the window it made.

use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::{AgentRole, RunRef, RunState, Runtime, WindowSpec};

use super::super::chain_goal::tests::{PREV, role, sleeping_claude};
use crate::launch::LaunchGate;
use crate::manager::{ManagerConfig, WindowManager};
use crate::run::driver::RunService;
use crate::run::orch::test_support::{orchestrator, run_of};

/// Decision 23 and AGENTS.md rules 2 and 10: the adoption's rename and rebind hold
/// neither the engine's lock nor the manager's, nor the runtime's one thread, while
/// they wait (the M9.13 lock-probe shape: a held step, both locks probed meanwhile).
#[tokio::test(flavor = "current_thread")]
async fn adopt_runs_off_the_manager_lock() {
    let dir = tempfile::Builder::new()
        .prefix("anthrex-adopt-")
        .tempdir_in("/tmp")
        .unwrap();
    let mut config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    config.claude_bin = sleeping_claude(dir.path());
    config.worktrees_root = dir.path().join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    let (manager, _events) = WindowManager::new(config);
    let s = RunService::for_manager(
        &manager,
        dir.path().join("data"),
        Arc::new(crate::run::driver::host_ops::tests::NoRoots),
    );
    let spec = WindowSpec {
        name: Some("3f9a/orchestrator".into()),
        runtime: Runtime::Claude,
        cwd: dir.path().to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    let window = manager
        .create_run_window(spec, dir.path().to_path_buf(), role(PREV))
        .await
        .expect("the stand-in's window")
        .id;
    let mut next = run_of(1);
    next.id = "next-goal-4c1d".into();
    next.state = RunState::Planning;
    let mut record = orchestrator();
    record.window_id = Some(window);
    record.session = 3;
    next.orch.orchestrator = Some(record);
    crate::lock(&s.state).runs.insert(next.id.clone(), next);

    let (entered_tx, entered) = std::sync::mpsc::channel();
    let (release, released) = std::sync::mpsc::channel::<()>();
    // Bounded, so a failing assertion below cannot leave the blocking step held.
    let hold = move || {
        entered_tx.send(()).unwrap();
        let _ = released.recv_timeout(Duration::from_secs(30));
    };
    let adopting = s.clone();
    let adopt = tokio::spawn(async move {
        adopting
            .adopt_window("next-goal-4c1d", window, "4c1d/orchestrator".into(), hold)
            .await;
    });
    let ticks = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let counter = ticks.clone();
    let ticker = tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_millis(10)).await;
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }
    });
    // The adoption's blocking step is held.
    let deadline = Instant::now() + Duration::from_secs(10);
    while entered.try_recv().is_err() {
        assert!(Instant::now() < deadline, "the adoption never started");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    // Both locks answer at once, from another thread, and this one keeps running.
    let probe = {
        let (manager, s) = (manager.clone(), s.clone());
        std::thread::spawn(move || {
            let asked = Instant::now();
            let listed = manager.list().iter().any(|w| w.id == window);
            drop(crate::lock(&s.state));
            (listed, asked.elapsed())
        })
    };
    let seen = ticks.load(std::sync::atomic::Ordering::SeqCst);
    while ticks.load(std::sync::atomic::Ordering::SeqCst) < seen + 5 {
        assert!(Instant::now() < deadline, "the runtime thread was blocked");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let (listed, took) = probe.join().unwrap();
    assert!(listed);
    assert!(took < Duration::from_millis(500), "{took:?}");
    assert!(!adopt.is_finished(), "the adoption was not held");
    release.send(()).unwrap();
    adopt.await.unwrap();
    ticker.abort();

    let live = manager.run_window_live(window);
    let name = manager
        .list()
        .into_iter()
        .find(|w| w.id == window)
        .map(|w| w.name);
    let _ = manager.kill(window);
    assert_eq!(
        live,
        Some(RunRef {
            run_id: "next-goal-4c1d".into(),
            task_id: None,
            role: AgentRole::Orchestrator,
            session: 3,
        })
    );
    assert_eq!(name.as_deref(), Some("4c1d/orchestrator"));
}
