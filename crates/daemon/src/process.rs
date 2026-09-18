//! Bounded cleanup of the session owned by a PTY child.

use std::time::{Duration, Instant};
use tokio::sync::oneshot;

pub const HUP_GRACE: Duration = Duration::from_secs(1);
pub const KILL_GRACE: Duration = Duration::from_secs(3);

/// Returns false when the entire group is already gone.
pub(crate) fn signal_group(pid: u32, signal: i32) -> anyhow::Result<bool> {
    anyhow::ensure!(pid > 0 && pid <= i32::MAX as u32, "invalid child pid {pid}");
    // SAFETY: portable-pty starts the child in its own session, with pid == pgid.
    if unsafe { libc::killpg(pid as libc::pid_t, signal) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        return Ok(false);
    }
    anyhow::bail!("killpg({pid}, {signal}): {error}")
}

/// Signal immediately, then monitor the group on a dedicated thread. Keep checking
/// even after the leader exits: a descendant may still need TERM or KILL.
pub(crate) fn escalate(pid: u32) -> anyhow::Result<oneshot::Receiver<()>> {
    let started = Instant::now();
    let (done, receiver) = oneshot::channel();
    if !signal_group(pid, libc::SIGHUP)? {
        let _ = done.send(());
        return Ok(receiver);
    }
    std::thread::Builder::new()
        .name(format!("pty-kill-{pid}"))
        .spawn(move || {
            let mut term_sent = false;
            loop {
                if matches!(signal_group(pid, 0), Ok(false)) {
                    break;
                }
                if started.elapsed() >= KILL_GRACE {
                    let _ = signal_group(pid, libc::SIGKILL);
                    break;
                }
                if !term_sent && started.elapsed() >= HUP_GRACE {
                    let _ = signal_group(pid, libc::SIGTERM);
                    term_sent = true;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            let _ = done.send(());
        })?;
    Ok(receiver)
}
