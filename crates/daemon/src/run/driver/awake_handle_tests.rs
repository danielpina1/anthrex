//! Review round 1, I1: `RunService::handle`'s wiring. A host sleep the awake clock
//! measured is stepped as `EventKind::Slept` before the event that noticed it, so the
//! first tick after a wake finds the open turn's silence excused and interrupts nothing.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use super::super::effects::Ready;
use super::super::{GitRoots, RunService, unix_now};
use crate::manager::{ManagerConfig, WindowManager};
use crate::run::engine::{Effect, EventKind};
use crate::run::orch::test_support::{round, run_of, task_mut};

/// A sleep longer than the run's stall bound.
const SLEPT: u64 = 1_000;
/// The worker's window: one that does not exist, so its interrupt reaches nothing.
const WORKER: u32 = 902;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_first_tick_after_a_sleep_steps_the_sleep_first() {
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path());
    let interrupts = Arc::new(AtomicUsize::new(0));
    let seen = interrupts.clone();
    *crate::lock(&s.execute_hook) = Some(Box::new(move |item: &Ready| {
        if matches!(item, Ready::Effect(Effect::Interrupt { .. })) {
            seen.fetch_add(1, Ordering::SeqCst);
        }
    }));
    // A working task's open turn, its last event just before the host slept.
    let asleep = unix_now() - SLEPT;
    let mut run = run_of(1);
    run.state = proto::RunState::Running;
    run.data_dir = dir.path().join("runs").join(&run.id);
    std::fs::create_dir_all(&run.data_dir).unwrap();
    run.limits.stall_after_secs = 300;
    let t0 = task_mut(&mut run, "t0");
    t0.state = proto::TaskState::Working;
    let mut working = round(1, 0, Default::default());
    (working.window_id, working.pid, working.turn_open) = (Some(WORKER), None, true);
    (working.started_at, working.last_event) = (asleep, asleep);
    t0.rounds.push(working);
    let run_id = run.id.clone();
    crate::lock(&s.state).runs.insert(run_id.clone(), run);
    // The awake clock last read just before the sleep: no awake time has passed since.
    *crate::lock(&s.awake.0) = Some(super::Anchor::new(
        Duration::from_secs(asleep),
        Instant::now(),
    ));

    s.handle(EventKind::Tick).await;

    assert_eq!(
        interrupts.load(Ordering::SeqCst),
        0,
        "the tick interrupted the turn"
    );
    let round = crate::lock(&s.state).runs[&run_id].tasks[0].rounds[0].clone();
    assert!(round.turn_open && !round.interrupted, "{round:#?}");
    assert!(round.last_event + 5 >= unix_now(), "{round:#?}");
    // No loop runs to acknowledge the stop.
    s.stop_wait_ms.store(200, Ordering::SeqCst);
    s.stop().await;
}

/// fix/clock-step-back: a wall clock set back (here: corrected after it ran a year
/// ahead, where the turn's last event was stamped) is stepped as
/// `EventKind::SteppedBack` before the event that noticed it, so the open turn's
/// silence is measured in awake time and the tick stalls it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_first_tick_after_a_step_back_steps_it_first() {
    const AHEAD: u64 = 365 * 24 * 3_600;
    let dir = tempfile::tempdir().unwrap();
    let s = service(dir.path());
    let interrupts = Arc::new(AtomicUsize::new(0));
    let seen = interrupts.clone();
    *crate::lock(&s.execute_hook) = Some(Box::new(move |item: &Ready| {
        if matches!(item, Ready::Effect(Effect::Interrupt { .. })) {
            seen.fetch_add(1, Ordering::SeqCst);
        }
    }));
    // A working task's open turn, silent for longer than its stall bound on a clock a
    // year ahead.
    let ahead = unix_now() + AHEAD;
    let quiet = ahead - 400;
    let mut run = run_of(1);
    run.state = proto::RunState::Running;
    run.data_dir = dir.path().join("runs").join(&run.id);
    std::fs::create_dir_all(&run.data_dir).unwrap();
    run.limits.stall_after_secs = 300;
    let t0 = task_mut(&mut run, "t0");
    t0.state = proto::TaskState::Working;
    let mut working = round(1, 0, Default::default());
    (working.window_id, working.pid, working.turn_open) = (Some(WORKER), None, true);
    (working.started_at, working.last_event) = (quiet, quiet);
    t0.rounds.push(working);
    let run_id = run.id.clone();
    crate::lock(&s.state).runs.insert(run_id.clone(), run);
    // The awake clock last read on the clock a year ahead; no awake time since.
    *crate::lock(&s.awake.0) = Some(super::Anchor::new(
        Duration::from_secs(ahead),
        Instant::now(),
    ));

    s.handle(EventKind::Tick).await;

    let round = crate::lock(&s.state).runs[&run_id].tasks[0].rounds[0].clone();
    assert!(round.last_event <= unix_now(), "{round:#?}");
    assert_eq!(
        interrupts.load(Ordering::SeqCst),
        1,
        "the tick did not stall the turn: {round:#?}"
    );
    s.stop_wait_ms.store(200, Ordering::SeqCst);
    s.stop().await;
}
