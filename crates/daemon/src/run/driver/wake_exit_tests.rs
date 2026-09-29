//! M9.13 re-review, item 3: each of the driver's two exit guards, alone. A real manager
//! and a real shell window stand for the orchestrator's (never an agent); the engine's
//! loop is not running, so what `check_orchestrators` reports is read off its channel.

use std::time::{Duration, Instant};

use proto::{Runtime, Status, WindowSpec};
use tokio::sync::mpsc::UnboundedReceiver;

use super::*;
use crate::launch::LaunchGate;
use crate::manager::{GitRoots, ManagerConfig};

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: std::path::PathBuf) {}
    fn unregister(&self, _: &std::path::Path) {}
}

struct Rig {
    _dir: tempfile::TempDir,
    manager: Arc<WindowManager>,
    runs: Arc<RunService>,
    events: UnboundedReceiver<super::super::Msg>,
    window: u32,
}

/// A run `r1` whose live orchestrator (one launch) is a shell window that has exited.
async fn exited_orchestrator() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let mut config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    config.worktrees_root = dir.path().join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    let (manager, mut pumped) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, event)) = pumped.recv().await {
            pump.handle_event(id, event);
        }
    });
    let runs = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    let events = crate::lock(&runs.rx).take().unwrap();
    let spec = WindowSpec {
        name: Some("orch-exit".into()),
        runtime: Runtime::Shell,
        cwd: dir.path().to_path_buf(),
        worktree_branch: None,
        model: None,
        initial_prompt: None,
    };
    let info = manager
        .create(spec, dir.path().to_path_buf(), None, 80, 24)
        .await
        .expect("a shell window");
    let mut run = crate::run::orch::test_support::run_of(1);
    run.id = "r1".into();
    let mut o = crate::run::orch::test_support::orchestrator();
    o.window_id = Some(info.id);
    o.launch_op = None;
    o.launches = 1;
    run.orch.orchestrator = Some(o);
    crate::lock(&runs.state).runs.insert("r1".into(), run);
    // The window this test started, and nothing else.
    manager.kill(info.id).unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while !manager
        .list()
        .iter()
        .any(|w| w.id == info.id && w.status == Status::Exited)
    {
        assert!(Instant::now() < deadline, "the shell did not exit");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    Rig {
        _dir: dir,
        manager,
        runs,
        events,
        window: info.id,
    }
}

/// The exit reports `check_orchestrators` sent since the last call.
fn exits(rig: &mut Rig) -> Vec<(u32, bool, u64)> {
    let mut seen = Vec::new();
    while let Ok(msg) = rig.events.try_recv() {
        if let super::super::Msg::Event(EventKind::Orch(OrchEvent::OrchestratorWindow {
            window_id,
            live,
            launch,
            ..
        })) = msg
        {
            seen.push((window_id, live, launch));
        }
    }
    seen
}

/// Past `EXIT_CONFIRM`, with margin.
async fn past_confirm() {
    tokio::time::sleep(EXIT_CONFIRM + Duration::from_millis(200)).await;
}

/// The confirmation guard: a window listed `Exited` is not reported the first time it
/// is seen so, only once it has stayed so for `EXIT_CONFIRM`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_exit_is_reported_only_once_it_has_lasted() {
    let mut rig = exited_orchestrator().await;
    rig.runs.check_orchestrators();
    assert_eq!(exits(&mut rig), vec![], "reported at first sight");
    past_confirm().await;
    rig.runs.check_orchestrators();
    assert_eq!(exits(&mut rig), vec![(rig.window, false, 1)]);
}

/// The restart guard: a window listed `Exited` while a restart of it is under way is
/// never reported, however long that lasts; once the restart is over the count starts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_exit_during_a_restart_is_never_reported() {
    let mut rig = exited_orchestrator().await;
    rig.manager.hold_restarting(rig.window, true);
    rig.runs.check_orchestrators();
    past_confirm().await;
    rig.runs.check_orchestrators();
    assert_eq!(exits(&mut rig), vec![], "reported during the restart");
    rig.manager.hold_restarting(rig.window, false);
    rig.runs.check_orchestrators();
    assert_eq!(exits(&mut rig), vec![]);
    past_confirm().await;
    rig.runs.check_orchestrators();
    assert_eq!(exits(&mut rig), vec![(rig.window, false, 1)]);
}
