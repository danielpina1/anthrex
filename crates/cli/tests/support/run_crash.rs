//! The run harness's helpers for a daemon that dies on purpose (decision 48's crash
//! injection): a request it may never answer, and forgetting the dead daemon. Moved out
//! of `run_harness.rs` (AGENTS.md rule 8), each with a bounded variant so a scenario's
//! deadline covers every wait in it (`run_e2e_crash.rs`).

use std::time::{Duration, Instant};

use proto::{ClientMsg, DaemonMsg, RunReply, RunRequest};

use super::run_daemon::DAEMON_EXIT_WAIT;
use super::run_harness::{REQUEST_WAIT, RunHarness, connect, request_wait};
use super::runtime;

impl RunHarness {
    /// Sends one raw run request whose daemon may die before answering (decision 48's
    /// crash injection); the reply, if one came.
    pub fn request_unanswered(&self, request: RunRequest) -> Option<RunReply> {
        let wait = request_wait(&request);
        self.request_unanswered_within(request, wait)
    }

    /// [`Self::request_unanswered`] bounded by `wait`, the connection and the handshake
    /// included: a daemon that accepts the connection and never sends its `Welcome`
    /// fails the request at `wait`, as one that neither answers nor dies does, instead
    /// of holding the test until nextest kills it, which skips `Drop` and leaks the
    /// daemon (`request_within`'s shape).
    pub fn request_unanswered_within(
        &self,
        request: RunRequest,
        wait: Duration,
    ) -> Option<RunReply> {
        let socket = self.socket();
        let reply = runtime().block_on(async move {
            tokio::time::timeout(wait, async move {
                let mut stream = connect(&socket).await;
                proto::write_frame(&mut stream, &ClientMsg::Run(request))
                    .await
                    .ok()?;
                loop {
                    match proto::read_frame::<_, DaemonMsg>(&mut stream).await {
                        Ok(Some(DaemonMsg::Run(reply))) => return Some(reply),
                        Ok(Some(_)) => {}
                        _ => return None,
                    }
                }
            })
            .await
        });
        reply.unwrap_or_else(|_| {
            panic!(
                "the daemon neither answered nor died within {wait:?} (the handshake included)\n--- daemon.log:\n{}",
                self.log_tail()
            )
        })
    }

    /// After the daemon died without its shutdown (decision 48's abort): waits until
    /// its socket refuses connections, then removes the socket file, so the next
    /// [`RunHarness::restart_daemon`] starts a fresh one.
    pub fn forget_dead_daemon(&self) {
        self.forget_dead_daemon_within(REQUEST_WAIT + DAEMON_EXIT_WAIT);
    }

    /// [`Self::forget_dead_daemon`], all of it within `wait`: the socket's refusal
    /// within `REQUEST_WAIT` and the process's exit within `DAEMON_EXIT_WAIT`, each cut
    /// to what is left of `wait`.
    pub fn forget_dead_daemon_within(&self, wait: Duration) {
        let end = Instant::now() + wait;
        let deadline = Instant::now() + REQUEST_WAIT.min(wait);
        while std::os::unix::net::UnixStream::connect(self.socket()).is_ok() {
            assert!(
                Instant::now() < deadline,
                "the daemon did not die\n--- daemon.log:\n{}",
                self.log_tail()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        // M8a.24 fix round 2 (N1): taken out of the lock first, and killed before the
        // assertion, so a failure neither poisons the lock nor leaks the process.
        let taken = self.daemon().take();
        if let Some(mut daemon) = taken {
            let left = end.saturating_duration_since(Instant::now());
            let exited = daemon.wait_exit(DAEMON_EXIT_WAIT.min(left));
            daemon.kill();
            assert!(exited, "the dead daemon's process was still running");
        }
        let _ = std::fs::remove_file(self.socket());
    }
}
