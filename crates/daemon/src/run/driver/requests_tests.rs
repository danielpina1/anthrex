//! Milestone 9.3 task 4a: `RunRequest::Iterate` reaches the engine's `EventKind::Iterate`
//! and is answered under `request::ITERATE`, against a real run loop (no window, no
//! agent, no host).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use proto::run_wire::request;
use proto::{RunReply, RunRequest};
use tokio_util::sync::CancellationToken;

use super::RunService;
use crate::manager::{GitRoots, ManagerConfig, WindowManager};

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

#[tokio::test(flavor = "multi_thread")]
async fn run_iterate_reaches_the_engine() {
    let dir = tempfile::tempdir().unwrap();
    let config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let s = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    let handle = s.spawn(CancellationToken::new());
    let iterate = RunRequest::Iterate {
        run: "nope".into(),
        goal: "more".into(),
        design: None,
    };
    let reply = tokio::time::timeout(Duration::from_secs(10), s.request(iterate))
        .await
        .expect("answered within one engine step");
    assert_eq!(
        reply,
        RunReply::refused(request::ITERATE, "unknown run nope")
    );
    s.stop().await;
    drop(handle);
}

/// Task M9.6.15 (decision 28): the request's `design` reaches the engine: `amend` on a
/// complete run without the design flow is refused with decision 28's exact text.
#[tokio::test(flavor = "multi_thread")]
async fn run_iterate_carries_its_design() {
    let dir = tempfile::tempdir().unwrap();
    let config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let s = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    let mut run = crate::run::orch::test_support::run_of(1);
    run.orch.orchestrator = Some(crate::run::orch::test_support::orchestrator());
    run.state = proto::RunState::Complete;
    let id = run.id.clone();
    crate::lock(&s.state).runs.insert(id.clone(), run);
    let handle = s.spawn(CancellationToken::new());
    let iterate = RunRequest::Iterate {
        run: id,
        goal: "more".into(),
        design: Some(proto::RoundDesign::Amend),
    };
    let reply = tokio::time::timeout(Duration::from_secs(10), s.request(iterate))
        .await
        .expect("answered within one engine step");
    let text = "this run has no spec to amend; iterate with --design off";
    assert_eq!(reply, RunReply::refused(request::ITERATE, text));
    s.stop().await;
    drop(handle);
}

/// Task M9.6.3 (DF §1): `run start --plan` with `--design full` is refused with the
/// exact text before anything is read (the directory does not exist); without it, or
/// with `off`, the start goes on to its first check.
#[tokio::test(flavor = "multi_thread")]
async fn a_plan_file_start_asking_for_the_design_flow_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let config = ManagerConfig::for_tests(dir.path().join("d.sock"), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    let s = RunService::for_manager(&manager, dir.path().join("data"), Arc::new(NoRoots));
    let start = |design| RunRequest::Start {
        plan_toml: "not a plan".into(),
        dir: dir.path().join("nowhere"),
        yes: false,
        trust_project: false,
        unconfined_checks: true,
        delivery: None,
        design,
    };
    let reply = s.request(start(Some(proto::DesignMode::Full))).await;
    assert_eq!(
        reply,
        RunReply::refused(
            request::START,
            "the design flow runs only for planned code or docs goals; this goal is a plan file"
        )
    );
    for design in [None, Some(proto::DesignMode::Off)] {
        let reply = s.request(start(design)).await;
        let RunReply::Refused { message, .. } = &reply else {
            panic!("{reply:?}");
        };
        assert!(!message.contains("design flow"), "{design:?}: {message}");
    }
}
