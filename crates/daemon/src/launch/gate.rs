//! The one-way latch a window launch passes through while the startup probe runs.
//!
//! `lifecycle::run` has exactly one thing it must finish before an agent process may be
//! launched: the Codex `--version` probe (`lifecycle/codex_version.rs`). That ordering
//! used to be produced by awaiting the probe immediately before `server::serve`, which
//! got it by holding up *everything* the daemon serves — the handshake included. A client
//! that connected while the probe ran was accepted into the listen backlog and then left
//! unanswered, racing its own `proto::HANDSHAKE_TIMEOUT` (5 s) against
//! `lifecycle::CODEX_PROBE_TIMEOUT` (5 s), two independent constants with nothing
//! separating them. This gate is what lets `serve` start at once and still keep the
//! ordering: the probe holds the gate, and the two places that actually launch a process
//! — [`crate::manager::WindowManager::create`] and
//! [`crate::manager::WindowManager::restart`] — wait on it.
//!
//! It is deliberately one-way and idempotent. There is one probe per daemon and it never
//! runs again, so "open" is a terminal state: [`LaunchGate::open`] may be called from any
//! number of places (the probe finishing, the probe's budget elapsing, a panic unwinding
//! through [`LaunchGate::open_on_drop`]) without any of them having to know whether
//! another already did. A gate that could close again would need every launch to re-check
//! it after every await point, which is exactly the "a guard at admission does not
//! constrain work already in flight" shape this module exists to avoid.
//!
//! [`CancellationToken`] is the implementation because it is precisely a clonable,
//! idempotent, many-waiter one-way latch, and it is already a dependency of this crate.
//! Its vocabulary is inverted here — *cancelled* means *open* — so it is wrapped rather
//! than used directly: nothing outside this file should have to hold that inversion in
//! its head, and `gate.wait().await` at a launch site should not read as if it were
//! waiting for a cancellation.

use tokio_util::sync::{CancellationToken, DropGuard};

/// A latch that starts closed and, once opened, stays open forever. Cloning shares the
/// same latch.
#[derive(Debug, Clone)]
pub struct LaunchGate(CancellationToken);

impl LaunchGate {
    /// A gate that is already open: every `wait` returns immediately.
    ///
    /// This is what [`crate::manager::ManagerConfig`] is built with, so that every
    /// manager outside `lifecycle::run` — the whole test suite, in practice — launches
    /// windows with no gate at all. Only `lifecycle::run` replaces it with a closed one
    /// and gives the probe the job of opening it.
    pub fn open_already() -> Self {
        let token = CancellationToken::new();
        token.cancel();
        Self(token)
    }

    /// A gate that holds every launch until [`LaunchGate::open`] is called.
    pub fn closed() -> Self {
        Self(CancellationToken::new())
    }

    /// Opens the gate, releasing every current and future waiter. Idempotent: calling it
    /// on an already-open gate does nothing.
    pub fn open(&self) {
        self.0.cancel();
    }

    /// Opens the gate when the returned guard is dropped — on a normal return, on an
    /// early `?`, and on a panic unwinding past it.
    ///
    /// This is how the probe holds the gate rather than remembering to open it: a probe
    /// that panics, or that gains an early return in some later change, must not leave
    /// every window launch for the rest of the daemon's life waiting on a gate nobody is
    /// going to open. The failure mode of forgetting to call [`LaunchGate::open`] is a
    /// permanently wedged daemon, so it is not left to a call site to remember.
    pub fn open_on_drop(&self) -> DropGuard {
        self.0.clone().drop_guard()
    }

    /// Returns once the gate is open. Returns immediately if it already is.
    pub async fn wait(&self) {
        self.0.cancelled().await;
    }

    pub fn is_open(&self) -> bool {
        self.0.is_cancelled()
    }
}

impl Default for LaunchGate {
    fn default() -> Self {
        Self::open_already()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[tokio::test]
    async fn an_open_gate_never_waits() {
        let gate = LaunchGate::open_already();
        assert!(gate.is_open());
        tokio::time::timeout(Duration::from_secs(1), gate.wait())
            .await
            .expect("an already-open gate must not hold a waiter");
    }

    /// The property both launch sites depend on: a closed gate holds, and *one* open
    /// releases every waiter, however many there are and whenever they arrived.
    #[tokio::test]
    async fn a_closed_gate_holds_every_waiter_until_one_open_releases_them_all() {
        let gate = LaunchGate::closed();
        assert!(!gate.is_open());
        assert!(
            tokio::time::timeout(Duration::from_millis(100), gate.wait())
                .await
                .is_err(),
            "a closed gate must hold"
        );

        let waiters: Vec<_> = (0..8)
            .map(|_| {
                let gate = gate.clone();
                tokio::spawn(async move { gate.wait().await })
            })
            .collect();
        gate.open();
        for waiter in waiters {
            tokio::time::timeout(Duration::from_secs(5), waiter)
                .await
                .expect("one open must release every waiter")
                .unwrap();
        }
        // Idempotent: a second open is a no-op, not a reset.
        gate.open();
        assert!(gate.is_open());
    }

    /// A probe that panics must not wedge every later launch. `open_on_drop` is what
    /// makes that structural rather than a call site's responsibility.
    #[tokio::test]
    async fn a_panicking_holder_still_opens_the_gate() {
        let gate = LaunchGate::closed();
        let held = gate.open_on_drop();
        let panicked = tokio::spawn(async move {
            let _held = held;
            panic!("the probe blew up");
        });
        assert!(panicked.await.is_err(), "the task was supposed to panic");
        tokio::time::timeout(Duration::from_secs(5), gate.wait())
            .await
            .expect("a panicking holder must still open the gate");
    }
}
