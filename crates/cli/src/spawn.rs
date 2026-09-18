//! Starts the daemon in the background when none is running.

use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::net::UnixStream;

async fn is_up(socket: &Path) -> bool {
    UnixStream::connect(socket).await.is_ok()
}

/// Re-executes this binary as `anthrex daemon start --foreground` in its own session,
/// with stdio detached, so it outlives the calling process.
fn spawn_detached() -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["daemon", "start", "--foreground"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: setsid is async-signal-safe and touches no Rust state between fork and exec.
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    let mut child = cmd.spawn()?;
    // Reap the detached child so it never lingers as a zombie once it exits —
    // this process (e.g. a long-lived TUI) may outlive the daemon by a long margin.
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

/// Connects if a daemon is running; otherwise starts one and waits up to 3 s for its socket.
pub async fn ensure_daemon(socket: &Path) -> anyhow::Result<()> {
    if is_up(socket).await {
        return Ok(());
    }
    spawn_detached()?;
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        if is_up(socket).await {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    anyhow::bail!(
        "the daemon did not start within 3 s; check {}",
        proto::paths::log_path().display()
    )
}
