//! Task M9.6: decision 20's `RunInfo.scouts` overlay, against a real manager, a real
//! `ScoutService` and a `/bin/sh` stand-in for Claude that reads its prompt and exits
//! when its stdin closes (never a real agent: Codex is a path that does not exist).

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use proto::{ScoutKind, ScoutState};

use crate::launch::LaunchGate;
use crate::run::driver::*;
use crate::scout::spec::ScoutSpec;

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

#[tokio::test(flavor = "multi_thread")]
async fn run_scouts_appear_in_the_snapshot() {
    let dir = tempfile::tempdir().unwrap();
    let stand_in = dir.path().join("claude.sh");
    std::fs::write(&stand_in, "#!/bin/sh\ncat > /dev/null\n").unwrap();
    std::fs::set_permissions(&stand_in, std::fs::Permissions::from_mode(0o755)).unwrap();
    let repo = dir.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    let socket = dir.path().join("d.sock");
    let mut config = ManagerConfig::new(socket.clone(), "/bin/sh".into());
    config.claude_bin = stand_in.display().to_string();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    config.worktrees_root = dir.path().join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    let (manager, mut events) = WindowManager::new(config);
    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, event)) = events.recv().await {
            pump.handle_event(id, event);
        }
    });
    let data = dir.path().join("data");
    let runs = RunService::for_manager(&manager, data.clone(), Arc::new(NoRoots));
    let orchestrator = config::Orchestrator {
        default_runtime: proto::Runtime::Claude,
        ..config::Orchestrator::default()
    };
    crate::profile::service::wire(&manager, &runs, &data, &socket, &orchestrator);
    let run = crate::run::test_support::run_ok(&crate::run::test_support::plan_with(
        crate::run::test_support::PROFILE,
        &[crate::run::test_support::task_toml(
            "t1",
            "S",
            "[\"crates/a/**\"]",
            "",
        )],
    ));
    let run_id = run.id.clone();
    crate::lock(&runs.state).runs.insert(run_id.clone(), run);
    assert!(runs.current().runs[0].scouts.is_empty());

    let scouts = runs.adaptation.get().unwrap().scouts.clone();
    let handle = scouts
        .start(ScoutSpec {
            id: "3f9a-daemon".into(),
            kind: ScoutKind::Area,
            run_id: Some(run_id.clone()),
            question: "Where are hooks parsed?".into(),
            first_turn: "[anthrex] Answer the question.".into(),
            cwd: repo.clone(),
            project: repo.clone(),
            web: false,
            codex_config: Vec::new(),
            base_sha: String::new(),
            repo_paths: Vec::new(),
        })
        .await
        .expect("the scout starts");

    // `current` (a `List` answer, a subscription's first snapshot) carries it...
    let snap = runs.current();
    let shown = &snap.runs[0].scouts;
    assert_eq!(shown.len(), 1, "{shown:?}");
    assert_eq!(shown[0].id, "3f9a-daemon");
    assert_eq!(shown[0].kind, ScoutKind::Area);
    assert_eq!(shown[0].window_id, Some(handle.window_id));
    assert!(
        matches!(shown[0].state, ScoutState::Starting | ScoutState::Working),
        "{:?}",
        shown[0].state
    );
    // ... and so does every push.
    let mut pushes = runs.pushes();
    let raw = snapshot(&crate::lock(&runs.state), unix_now());
    assert!(raw.runs[0].scouts.is_empty(), "the pure snapshot has none");
    runs.publish(raw);
    let pushed = tokio::time::timeout(Duration::from_secs(10), pushes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pushed.runs[0].scouts.len(), 1);
    assert_eq!(pushed.runs[0].scouts[0].id, "3f9a-daemon");

    // The window this test started, and nothing else.
    manager.headless_kill(handle.window_id).unwrap();
}
