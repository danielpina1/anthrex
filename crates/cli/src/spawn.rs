//! Starts the daemon in the background when none is running.

use std::fs::OpenOptions;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::net::UnixStream;

async fn is_up(socket: &Path) -> bool {
    UnixStream::connect(socket).await.is_ok()
}

/// Opens `proto::paths::stderr_path()` for append, creating `data_dir` first if needed.
/// A detached daemon has no terminal, and `logfile::RotatingFile` can only report a
/// failed log rotation via `eprintln!` (task-3 review; nothing else on that path can
/// surface it) — so that message needs this file to exist and be writable *before* the
/// daemon does anything else, not `/dev/null`. Falls back to `None` (the caller then
/// uses `Stdio::null()`, today's behaviour) rather than failing the whole daemon start
/// over a diagnostic sink that could not be opened: losing that one message is better
/// than refusing to start the daemon at all.
fn open_stderr_sink() -> Option<std::fs::File> {
    let path = proto::paths::stderr_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).ok()?;
    }
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
}

/// Re-executes this binary as `anthrex daemon start --foreground` in its own session,
/// with stdio detached, so it outlives the calling process.
fn spawn_detached() -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
    let exe = std::env::current_exe()?;
    let mut cmd = std::process::Command::new(exe);
    let stderr = match open_stderr_sink() {
        Some(file) => Stdio::from(file),
        None => Stdio::null(),
    };
    cmd.args(["daemon", "start", "--foreground"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr);
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
