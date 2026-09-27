//! M8b.15 review (I3, I4, I5): the run service as the OTLP receiver's sink.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use proto::{RunState, TokenUsage};
use tokio_util::sync::CancellationToken;

use super::super::{Msg, RunService};
use crate::manager::{GitRoots, ManagerConfig, WindowManager};
use crate::metering::UsageSink;
use crate::run::engine::EventKind;
use crate::run::model::Run;

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: PathBuf) {}
    fn unregister(&self, _: &Path) {}
}

fn service(data: &Path) -> Arc<RunService> {
    let config = ManagerConfig::new("/tmp/ax-unused.sock".into(), "/bin/sh".into());
    let (manager, _events) = WindowManager::new(config);
    RunService::for_manager(&manager, data.to_path_buf(), Arc::new(NoRoots))
}

/// A run in `state` that dispatches nothing: `Paused` is live and schedules no work.
fn run(id: &str, data: &Path, state: RunState) -> Run {
    use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};
    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.id = id.to_string();
    run.data_dir = data.join("runs").join(id);
    run.state = state;
    run
}

fn usage(n: u64) -> TokenUsage {
    TokenUsage {
        input: n,
        output: 2 * n,
        cache_read: 3 * n,
        cache_write: 4 * n,
    }
}

/// Waits, to a deadline, until `done` holds.
async fn until(what: &str, mut done: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !done() {
        assert!(Instant::now() < deadline, "timed out waiting until {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// A step that changes nothing, so the loop refreshes the live runs.
fn nudge(s: &RunService) {
    s.send(EventKind::OrchestratorUsage {
        run_id: "r-none".into(),
        usage: TokenUsage::default(),
    });
}

#[tokio::test(flavor = "multi_thread")]
async fn only_runs_the_daemon_has_and_that_have_not_ended_are_live() {
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    {
        let mut state = crate::lock(&s.state);
        for (id, run_state) in [("r1", RunState::Paused), ("r2", RunState::Discarded)] {
            state
                .runs
                .insert(id.into(), run(id, data.path(), run_state));
        }
    }
    let handle = s.spawn(CancellationToken::new());
    nudge(&s);
    until("r1 is live", || s.is_live("r1")).await;
    assert!(!s.is_live("r2"), "a discarded run is not live");
    assert!(!s.is_live("r-junk"));
    let generation = s.live_generation();
    // An unchanged set keeps its generation.
    nudge(&s);
    nudge(&s);
    s.post("r-none".into(), TokenUsage::default());
    until("the posts are drained", || {
        crate::lock(&s.metered.pending).is_empty()
    })
    .await;
    assert_eq!(s.live_generation(), generation);
    // A run that ends is no longer live, and the generation moves.
    crate::lock(&s.state).runs.get_mut("r1").unwrap().state = RunState::Accepted;
    nudge(&s);
    until("r1 is no longer live", || !s.is_live("r1")).await;
    assert!(s.live_generation() > generation);
    s.stop().await;
    let _ = handle.await;
}

/// Review I4: a flood of posts queues one drain, and the engine gets the latest total
/// of each run once.
#[tokio::test]
async fn a_flood_of_posts_queues_one_drain_with_the_latest_totals() {
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    let mut rx = crate::lock(&s.rx).take().expect("the loop never ran");
    for n in 1..=1000 {
        s.post("r1".into(), usage(n));
        s.post("r2".into(), usage(2 * n));
    }
    let mut drains = 0;
    while let Ok(msg) = rx.try_recv() {
        assert!(matches!(msg, Msg::Usage), "only drains are queued");
        drains += 1;
    }
    assert_eq!(drains, 1);
    let pending = s.metered.take();
    assert_eq!(
        pending.into_iter().collect::<Vec<_>>(),
        vec![
            ("r1".to_string(), usage(1000)),
            ("r2".to_string(), usage(2000))
        ]
    );
    // Once drained, the next post queues the next drain.
    s.post("r1".into(), usage(1));
    assert!(matches!(rx.try_recv(), Ok(Msg::Usage)));
    assert!(rx.try_recv().is_err());
}

/// Review I5: a restored run is live, and a total posted after the restart adds to the
/// usage it had.
#[tokio::test(flavor = "multi_thread")]
async fn a_restored_run_is_live_and_its_usage_adds_to_what_was_stored() {
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    let mut stored = run("r-back", data.path(), RunState::Running);
    stored.orchestrator_usage = usage(999);
    stored.revision = 3;
    crate::run::journal::save_run(&stored).unwrap();
    tokio::time::timeout(Duration::from_secs(60), s.restore())
        .await
        .expect("the restore returns");
    assert!(s.is_live("r-back"), "a restored run is live at once");
    let handle = s.spawn(CancellationToken::new());
    s.post("r-back".into(), usage(5));
    let total = || crate::lock(&s.state).runs["r-back"].orchestrator_usage;
    until("the total reaches the run", || total() == usage(1004)).await;
    s.stop().await;
    let _ = handle.await;
}
