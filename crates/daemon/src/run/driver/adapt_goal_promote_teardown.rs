//! The promote test's teardown (`run_promote_records_what_its_check_found`), split out
//! of `adapt_goal_installed_tests.rs` for the 600-line rule: the ops that ended (the
//! driver's `after_op` hook), the project's write turn held, and the wait for the
//! checkout op before the test's directory goes (the leak of main 636f0a40).

use std::sync::Arc;

use crate::run::driver::RunService;

/// The op that makes a task's checkout and its task temporary directory.
pub(super) const PREPARE: &str = "PrepareWorktree";

/// The longest the promote test waits for the write turn's waiter or for an op to
/// end, and holds the turn should it never be released: a hang guard only.
pub(super) const HOLD_MAX: std::time::Duration = std::time::Duration::from_secs(60);

pub(super) type Ended = Arc<std::sync::Mutex<Vec<&'static str>>>;

/// The names of the ops of `service` that have ended, in order (the driver's
/// `after_op` hook).
pub(super) fn ended_ops(service: &RunService) -> Ended {
    let ended = Ended::default();
    let record = ended.clone();
    *crate::lock(&service.after_op) = Some(Box::new(move |_: &RunService, name| {
        crate::lock(&record).push(name);
    }));
    ended
}

pub(super) fn has_ended(ended: &Ended, name: &str) -> bool {
    crate::lock(ended).contains(&name)
}

pub(super) async fn wait_ended(ended: &Ended, name: &str) {
    let deadline = std::time::Instant::now() + HOLD_MAX;
    while !has_ended(ended, name) {
        assert!(std::time::Instant::now() < deadline, "{name} never ended");
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}

/// Holds `project`'s write turn until the returned sender sends (or [`HOLD_MAX`]).
pub(super) async fn hold_write_turn(
    service: &RunService,
    project: std::path::PathBuf,
) -> (
    std::sync::mpsc::Sender<()>,
    tokio::task::JoinHandle<Result<(), String>>,
) {
    let (held_tx, held_rx) = tokio::sync::oneshot::channel();
    let held_tx = std::sync::Mutex::new(Some(held_tx));
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let release_rx = std::sync::Mutex::new(release_rx);
    let queue = service.queue.clone();
    let holder = tokio::spawn(async move {
        queue
            .write(&project, move || {
                if let Some(held) = crate::lock(&held_tx).take() {
                    let _ = held.send(());
                }
                let _ = crate::lock(&release_rx).recv_timeout(HOLD_MAX);
                Ok(())
            })
            .await
    });
    held_rx.await.expect("the write turn is held");
    (release_tx, holder)
}

/// The promote test's teardown, as the daemon stops (`lifecycle.rs`): the run service
/// (its last `run.json` written) and its loop; then the wait for the checkout op the
/// loop started, which runs on its own task that neither awaits (the daemon's process
/// exit ends it; a test's directory must outlive it); only then the directory
/// (`RunService::stop_and_settle`). The orchestrator's launch, parked for good at the
/// closed gate, is not waited for: it never passes the gate, and the runtime drops it.
/// No window is ever started, so none is killed (`SETTLE_BOUND`).
pub(super) async fn teardown(
    service: &Arc<RunService>,
    run_loop: tokio::task::JoinHandle<()>,
    shutdown: tokio_util::sync::CancellationToken,
    tmp: crate::run::git::TestDir,
) {
    service
        .stop_and_settle(run_loop, &shutdown, crate::run::driver::stop::SETTLE_BOUND)
        .await;
    drop(tmp);
}
