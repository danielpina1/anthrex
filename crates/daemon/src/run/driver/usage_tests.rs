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

/// Review minor 2 (mutant B): a step that ends one run and starts another keeps the
/// count of live runs but changes them, and the new set is taken.
#[test]
fn live_runs_that_change_but_keep_their_count_are_refreshed() {
    let data = tempfile::tempdir().unwrap();
    let metered = super::Metered::default();
    let mut state = crate::run::engine::EngineState::default();
    state
        .runs
        .insert("r1".into(), run("r1", data.path(), RunState::Paused));
    metered.refresh_live(&state);
    let generation = metered.generation.load(std::sync::atomic::Ordering::SeqCst);
    state.runs.get_mut("r1").unwrap().state = RunState::Accepted;
    state
        .runs
        .insert("r2".into(), run("r2", data.path(), RunState::Paused));
    metered.refresh_live(&state);
    let live = crate::lock(&metered.live).clone();
    assert_eq!(live, ["r2".to_string()].into_iter().collect());
    assert!(metered.generation.load(std::sync::atomic::Ordering::SeqCst) > generation);
}

/// Review minor 2 (mutant D): one drain gives every pending run its total, not only
/// the first.
#[tokio::test(flavor = "multi_thread")]
async fn one_drain_gives_every_pending_run_its_total() {
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    {
        let mut state = crate::lock(&s.state);
        for id in ["r1", "r2", "r3"] {
            state
                .runs
                .insert(id.into(), run(id, data.path(), RunState::Paused));
        }
    }
    // Queued before the loop runs, so the three totals are in one drain.
    s.post("r1".into(), usage(1));
    s.post("r2".into(), usage(2));
    s.post("r3".into(), usage(3));
    let handle = s.spawn(CancellationToken::new());
    let total = |id: &str| crate::lock(&s.state).runs[id].orchestrator_usage;
    until("every run has its total", || {
        (total("r1"), total("r2"), total("r3")) == (usage(1), usage(2), usage(3))
    })
    .await;
    s.stop().await;
    let _ = handle.await;
}

/// Review minor 1: the base a restore sets is never stored, so a run the restore does
/// not otherwise change is not rewritten because it has orchestrator usage.
#[tokio::test(flavor = "multi_thread")]
async fn restoring_a_run_with_orchestrator_usage_writes_nothing() {
    use std::os::unix::fs::MetadataExt;
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    let mut stored = run("done", data.path(), RunState::Accepted);
    stored.orchestrator_usage = usage(999);
    stored.revision = 4;
    crate::run::journal::save_run(&stored).unwrap();
    let file = stored.data_dir.join(crate::run::journal::RUN_FILE);
    let before = std::fs::metadata(&file).unwrap().ino();
    tokio::time::timeout(Duration::from_secs(60), s.restore())
        .await
        .expect("the restore returns");
    assert_eq!(
        crate::lock(&s.state).runs["done"].orchestrator_usage,
        usage(999)
    );
    assert_eq!(
        std::fs::metadata(&file).unwrap().ino(),
        before,
        "run.json was rewritten"
    );
}

/// M8b.15 re-review 2, m1: a run the restore does change is saved, orchestrator usage
/// and all. `same_on_disk` answering "same" for it would leave `run.json` stale.
#[tokio::test(flavor = "multi_thread")]
async fn restoring_a_run_the_restore_changes_saves_it() {
    let data = tempfile::tempdir().unwrap();
    let s = service(data.path());
    let mut stored = run("live", data.path(), RunState::Running);
    stored.orchestrator_usage = usage(999);
    crate::run::journal::save_run(&stored).unwrap();
    tokio::time::timeout(Duration::from_secs(60), s.restore())
        .await
        .expect("the restore returns");
    assert_eq!(crate::lock(&s.state).runs["live"].state, RunState::Paused);
    let file = stored.data_dir.join(crate::run::journal::RUN_FILE);
    let on_disk: Run = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(
        on_disk.state,
        RunState::Paused,
        "run.json was not rewritten"
    );
    assert_eq!(on_disk.orchestrator_usage, usage(999));
}
