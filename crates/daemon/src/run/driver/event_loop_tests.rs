//! Decision 8 (M9.11.8, B1): a panic in an event's handling after its step, in the
//! effects (`execute`) or in the tick's own work (`on_tick`), costs that event only.
//! The event loop keeps receiving, a waiting caller is answered, and ticks go on.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use tokio_util::sync::CancellationToken;

use super::super::effects::Ready;
use super::super::guard::EFFECTS_PANICKED;
use super::super::{GitRoots, RunService};
use crate::manager::{ManagerConfig, WindowManager};
use crate::run::engine::EventKind;
use crate::run::test_support::{PROFILE, plan_with, run_ok, task_toml};

/// The driver's ticker period (`RunService::spawn`'s `interval`).
const TICK: Duration = Duration::from_secs(1);
/// The workspace's allowance for a loaded machine (`docs/timing-budgets.md`).
const WALL_CLOCK_SLACK: Duration = Duration::from_secs(5);
/// Every wait below: an answered request costs one step in memory and at most one
/// `run.json` save on a blocking thread, and a tick is due at most one `TICK` after the
/// last; the failure these tests catch is a hang (no answer, no tick, ever), so the
/// bound only has to exceed that legal worst case.
const REQUEST_WAIT: Duration = Duration::from_secs(TICK.as_secs() + WALL_CLOCK_SLACK.as_secs());

struct NoRoots;
impl GitRoots for NoRoots {
    fn register(&self, _: std::path::PathBuf) {}
    fn unregister(&self, _: &std::path::Path) {}
}

fn service(data: &std::path::Path) -> Arc<RunService> {
    let mut config = ManagerConfig::for_tests(data.join("d.sock"), "/bin/sh".into());
    config.claude_bin = "/nonexistent/anthrex-test/claude".into();
    config.codex_bin = "/nonexistent/anthrex-test/codex".into();
    config.decider_bin = Some("/nonexistent/anthrex-test/decider".into());
    let (manager, _events) = WindowManager::new(config);
    RunService::for_manager(&manager, data.join("data"), Arc::new(NoRoots))
}

/// Counts the ticks that reach `check_orchestrators` (its `between_reads` hook), and
/// panics on the first when `panic_first` is set.
fn count_ticks(s: &RunService, panic_first: bool) -> Arc<AtomicUsize> {
    let ticks = Arc::new(AtomicUsize::new(0));
    let seen = ticks.clone();
    *crate::lock(&s.wakes.between_reads) = Some(Box::new(move || {
        if seen.fetch_add(1, Ordering::SeqCst) == 0 && panic_first {
            panic!("injected: the tick's work panicked");
        }
    }));
    ticks
}

async fn wait_for_more_ticks(ticks: &AtomicUsize, than: usize) {
    let deadline = Instant::now() + REQUEST_WAIT;
    while ticks.load(Ordering::SeqCst) <= than {
        assert!(Instant::now() < deadline, "no tick ran after the panic");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A request that goes through the loop: cancelling a run that does not exist answers
/// with a refusal, but only once the loop has stepped it.
async fn loop_answers(s: &Arc<RunService>) {
    let answer = tokio::time::timeout(
        REQUEST_WAIT,
        s.ask(|reply| EventKind::Cancel {
            reply,
            run_id: "no-such-run".into(),
        }),
    )
    .await
    .expect("the loop answers a request after the panic");
    assert!(answer.is_err(), "{answer:?}");
    assert_ne!(answer.unwrap_err(), "the daemon is shutting down");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panic_in_execute_keeps_the_loop() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path());
    let ticks = count_ticks(&s, false);
    let fired = Arc::new(AtomicBool::new(false));
    let once = fired.clone();
    *crate::lock(&s.execute_hook) = Some(Box::new(move |item: &Ready| {
        let panicky = match item {
            Ready::Save(run) => run.id == "panicky",
            Ready::Dirty(id) => id == "panicky",
            _ => false,
        };
        if panicky && !once.swap(true, Ordering::SeqCst) {
            panic!("injected: the first Persist of run panicky");
        }
    }));
    let shutdown = CancellationToken::new();
    s.spawn(shutdown.clone());

    let mut run = run_ok(&plan_with(
        PROFILE,
        &[task_toml("t1", "S", "[\"crates/a/**\"]", "")],
    ));
    run.id = "panicky".into();
    run.data_dir = dir.path().join("runs").join("panicky");
    std::fs::create_dir_all(&run.data_dir).unwrap();
    let started = tokio::time::timeout(
        REQUEST_WAIT,
        s.ask(|reply| EventKind::Start {
            reply,
            run: Box::new(run),
        }),
    )
    .await
    .expect("the start's caller is answered, not left waiting");
    assert!(fired.load(Ordering::SeqCst), "the hook never fired");
    assert_eq!(started, Err(EFFECTS_PANICKED.to_string()));

    // The step was committed before its effects panicked: the list shows the run.
    let listed = s.current();
    assert!(
        listed.runs.iter().any(|r| r.run_id == "panicky"),
        "{listed:?}"
    );
    loop_answers(&s).await;
    let now = ticks.load(Ordering::SeqCst);
    wait_for_more_ticks(&ticks, now).await;
    s.stop().await;
    shutdown.cancel();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_panic_in_on_tick_keeps_the_loop() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path());
    let ticks = count_ticks(&s, true);
    let shutdown = CancellationToken::new();
    s.spawn(shutdown.clone());
    // The first tick panics inside `on_tick`.
    wait_for_more_ticks(&ticks, 0).await;
    loop_answers(&s).await;
    wait_for_more_ticks(&ticks, 1).await;
    s.stop().await;
    shutdown.cancel();
}
