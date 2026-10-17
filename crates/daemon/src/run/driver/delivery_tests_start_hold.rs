//! `local_mode_calls_no_host`'s forced in-flight checkout op, split out of
//! `delivery_tests_start.rs` for the 600-line rule: the run branch's op takes the
//! project's write turn as it ends (the driver's `after_op` hook), so the checkout op the
//! engine starts next (`PrepareWorktree`, which makes the task's temporary directory) is
//! parked behind it until the test lets it go, after its teardown has begun.

use std::sync::{Arc, Mutex, mpsc};
use std::time::{Duration, Instant};

use crate::run::driver::RunService;

/// The op that makes a task's checkout and its task temporary directory.
pub(super) const PREPARE: &str = "PrepareWorktree";

/// The op whose end takes the write turn.
const RUN_BRANCH: &str = "CreateRunBranch";

/// The longest the test waits for the turn to be taken, for the checkout op to wait
/// for it or to end, and the turn is held should it never be released: a hang guard
/// only.
pub(super) const HOLD_MAX: Duration = Duration::from_secs(60);

/// What the hook saw, and the sender that lets the held write turn go.
pub(super) struct Held {
    ended: Arc<Mutex<Vec<&'static str>>>,
    release: Mutex<Option<mpsc::Sender<()>>>,
}

impl Held {
    pub(super) fn has_ended(&self, name: &str) -> bool {
        crate::lock(&self.ended).contains(&name)
    }

    pub(super) async fn wait_ended(&self, name: &str) {
        let deadline = Instant::now() + HOLD_MAX;
        while !self.has_ended(name) {
            assert!(Instant::now() < deadline, "{name} never ended");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Lets the held write turn go (once; later calls do nothing).
    pub(super) fn release(&self) {
        if let Some(tx) = crate::lock(&self.release).take() {
            let _ = tx.send(());
        }
    }
}

/// Installs `service`'s `after_op` hook: it records each op that ends, and when the run
/// branch's op ends it takes the write turn of the service's one run's project, and
/// returns only once the turn is held, so the op the engine starts next waits for it.
/// The hook's wait is on `block_in_place` (the test's runtime is multi-threaded).
pub(super) fn hold_after_run_branch(service: &RunService) -> Arc<Held> {
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let release_rx = Arc::new(Mutex::new(Some(release_rx)));
    let held = Arc::new(Held {
        ended: Arc::default(),
        release: Mutex::new(Some(release_tx)),
    });
    let ended = held.ended.clone();
    *crate::lock(&service.after_op) = Some(Box::new(move |s: &RunService, name| {
        crate::lock(&ended).push(name);
        if name != RUN_BRANCH {
            return;
        }
        let Some(release_rx) = crate::lock(&release_rx).take() else {
            return;
        };
        let project = {
            let state = crate::lock(&s.state);
            let mut runs = state.runs.values();
            let run = runs.next().expect("the run");
            assert!(runs.next().is_none(), "one run");
            run.project.clone()
        };
        let (taken_tx, taken_rx) = mpsc::channel::<()>();
        let queue = s.queue.clone();
        let release_rx = Mutex::new(release_rx);
        let taken_tx = Mutex::new(taken_tx);
        tokio::spawn(async move {
            queue
                .write(&project, move || {
                    let _ = crate::lock(&taken_tx).send(());
                    let _ = crate::lock(&release_rx).recv_timeout(HOLD_MAX);
                    Ok(())
                })
                .await
        });
        let taken = tokio::task::block_in_place(|| taken_rx.recv_timeout(HOLD_MAX));
        assert!(taken.is_ok(), "the write turn was never taken");
    }));
    held
}

/// Waits until a write waits for the held turn: the checkout op, parked.
pub(super) async fn wait_parked(service: &RunService) {
    let deadline = Instant::now() + HOLD_MAX;
    while service.queue.waiters() == 0 {
        assert!(
            Instant::now() < deadline,
            "the run's checkout op never waited for the write turn"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
