//! The anthrex daemon: owns PTY windows and serves them over a Unix socket.

/// Locks a mutex, taking the data back even when a previous holder panicked.
///
/// Spec section 7: one window must never take the daemon down. A panic anywhere under
/// one of these locks - `vt100::process` on a reader thread is the realistic one - would
/// otherwise poison the mutex, and the next caller would panic in turn while holding the
/// manager's lock, poisoning that too and leaving every window unreachable. Everything
/// these mutexes guard (a screen mirror, the window table) stays structurally sound after
/// such a panic, so recovering is strictly better than cascading.
pub(crate) fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub mod agent_state;
pub mod conversation;
pub mod git;
pub mod hooks;
pub mod launch;
pub mod lifecycle;
pub mod lockfile;
pub mod logfile;
pub mod manager;
mod process;
pub mod project;
pub mod server;
pub mod state;
pub mod status;
pub mod subagents;
/// `#[doc(hidden)] pub` rather than private only so that `crates/daemon/tests/subprocess.rs`
/// can exercise `Captured`, `Outcome`, `run` and `run_captured` in a test binary of its own —
/// the same reason and the same treatment `worktree::run_git` gives itself. Nothing outside
/// the crate uses it otherwise: `project::detect_roots_with`, `git::probe` and
/// `worktree::run_git`, the module's real callers, all live inside this crate.
#[doc(hidden)]
pub mod subprocess;
pub mod transcript;
pub mod window;
pub mod worktree;

pub use lifecycle::{DaemonOptions, LOCK_WAIT, run};

#[cfg(test)]
mod tests {
    /// I4: a poisoned mutex must not be a second failure on top of the first.
    #[test]
    fn lock_recovers_from_a_poisoned_mutex() {
        let mutex = std::sync::Arc::new(std::sync::Mutex::new(41));
        let poisoner = std::sync::Arc::clone(&mutex);
        // The panic message this prints is expected; the point is what survives it.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _guard = poisoner.lock().unwrap();
            panic!("boom");
        }));
        assert!(mutex.is_poisoned());
        *super::lock(&mutex) += 1;
        assert_eq!(*super::lock(&mutex), 42);
    }
}
