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
