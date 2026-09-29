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
    let mut config = ManagerConfig::for_tests(socket.clone(), "/bin/sh".into());
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

/// M9.6 second review (Important): the tick releases the engine lock before it lays
/// the scouts over the snapshot, so a scout table held elsewhere never holds the
/// engine lock too. The scout table is held while a tick with a publish due runs; once
/// the tick has taken `publish_due` it is at, or on its way to, `with_scouts`, and the
/// engine lock must still be free. Holding the engine lock across `publish` fails this
/// test, unless the lock is taken in the few instructions between the tick's reading
/// of `publish_due` and its taking of the engine lock: a rare false green, never a
/// false red.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_tick_publishes_outside_the_engine_lock() {
    let dir = tempfile::tempdir().unwrap();
    let socket = dir.path().join("d.sock");
    let mut config = ManagerConfig::for_tests(socket.clone(), "/bin/sh".into());
    config.claude_bin = "/nonexistent/anthrex-test/claude".into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    config.worktrees_root = dir.path().join("worktrees");
    config.launch_gate = LaunchGate::open_already();
    let (manager, _events) = WindowManager::new(config);
    let data = dir.path().join("data");
    let runs = RunService::for_manager(&manager, data.clone(), Arc::new(NoRoots));
    crate::profile::service::wire(
        &manager,
        &runs,
        &data,
        &socket,
        &config::Orchestrator::default(),
    );
    let run = crate::run::test_support::run_ok(&crate::run::test_support::plan_with(
        crate::run::test_support::PROFILE,
        &[crate::run::test_support::task_toml(
            "t1",
            "S",
            "[\"crates/a/**\"]",
            "",
        )],
    ));
    crate::lock(&runs.state).runs.insert(run.id.clone(), run);
    let scouts = runs.adaptation.get().unwrap().scouts.clone();
    let mut pushes = runs.pushes();

    // The scout table, held on a thread of its own until `release` is sent.
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        scouts.with_table_held(|| {
            held_tx.send(()).unwrap();
            let _ = release_rx.recv_timeout(Duration::from_secs(30));
        })
    });
    held_rx.recv_timeout(Duration::from_secs(10)).unwrap();

    crate::lock(&runs.book).publish_due = true;
    let ticking = runs.clone();
    let tick = tokio::spawn(async move { ticking.on_tick(unix_now()).await });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while crate::lock(&runs.book).publish_due {
        assert!(std::time::Instant::now() < deadline, "the tick never ran");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let probing = runs.clone();
    let probe = tokio::task::spawn_blocking(move || {
        drop(crate::lock(&probing.state));
    });
    let engine_free = tokio::time::timeout(Duration::from_secs(3), probe).await;

    release_tx.send(()).unwrap();
    holder.join().unwrap();
    tokio::time::timeout(Duration::from_secs(10), tick)
        .await
        .expect("the tick finishes once the scout table is free")
        .unwrap();
    assert!(
        engine_free.is_ok(),
        "the engine lock was held while the tick waited on the scout table"
    );
    let pushed = tokio::time::timeout(Duration::from_secs(10), pushes.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pushed.runs.len(), 1);
}

/// Task M9.8 (decision 34): the driver reads the stored reports a first turn's slot
/// names, each resolved as `scout::report::resolve_ref` says (a run scout's under the
/// run, `onboarding` under the repository), and puts their extract in the slot. One
/// that cannot be read, or that is not a scout id, is left out.
#[test]
fn the_driver_fills_a_first_turn_with_the_stored_reports() {
    use crate::run::orch::extract::{ExtractSlot, scout_extract};
    use crate::scout::report::report_path;
    let dir = tempfile::tempdir().unwrap();
    let (run_dir, repo_dir) = (dir.path().join("run"), dir.path().join("repo"));
    let report = |id: &str, summary: &str| proto::ScoutReport {
        id: id.into(),
        kind: ScoutKind::Area,
        run_id: None,
        question: String::new(),
        summary: summary.into(),
        files: vec![proto::ScoutFile {
            path: "crates/api/src/lib.rs".into(),
            why: String::new(),
        }],
        modules: Vec::new(),
        interfaces: Vec::new(),
        risks: Vec::new(),
        profile: None,
        route: proto::Route {
            runtime: proto::Runtime::Claude,
            model: String::new(),
            strength: proto::Strength::Fast,
            effort: proto::Effort::Low,
        },
        window_id: None,
        started_at: 0,
        finished_at: 0,
        tool_calls: 0,
        usage: Default::default(),
    };
    let store = |path: PathBuf, report: &proto::ScoutReport| {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, serde_json::to_vec(report).unwrap()).unwrap();
    };
    let api = report("3f9a-api", "The API is one crate.");
    store(report_path(&repo_dir, Some(&run_dir), "3f9a-api"), &api);
    let onboarding = report("onboarding-7", "A Rust workspace.");
    store(report_path(&repo_dir, None, "onboarding-7"), &onboarding);
    let refs: Vec<String> = ["3f9a-api", "3f9a-gone", "onboarding", "../escape"]
        .map(String::from)
        .to_vec();
    let slot = ExtractSlot::new(&refs, Some("onboarding-7"), 4, "\n\n").unwrap();
    let got = super::filled(&slot, &run_dir, &repo_dir, "HEAD\n\nBRIEF");
    let extract = scout_extract(&[
        ("3f9a-api".to_string(), api),
        ("onboarding".to_string(), onboarding),
    ]);
    assert_eq!(got, format!("HEAD\n\n{extract}\n\nBRIEF"));
    assert!(
        got.contains(
            "Scout report 3f9a-api:\n  The API is one crate.\nFiles: crates/api/src/lib.rs"
        )
    );
    // Nothing readable: the turn as the engine built it.
    let slot = ExtractSlot::new(&refs[1..2], None, 4, "\n\n").unwrap();
    assert_eq!(
        super::filled(&slot, &run_dir, &repo_dir, "HEAD\n\nBRIEF"),
        "HEAD\n\nBRIEF"
    );
}
