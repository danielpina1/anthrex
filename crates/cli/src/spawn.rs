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
/// failed log rotation via a direct write to its own stderr (task-3 review; nothing else
/// on that path can surface it) — so that message needs this file to exist and be
/// writable *before* the daemon does anything else, not `/dev/null`. Falls back to `None`
/// (the caller then uses `Stdio::null()`, today's behaviour) rather than failing the
/// whole daemon start over a diagnostic sink that could not be opened: losing that one
/// message is better than refusing to start the daemon at all.
fn open_stderr_sink() -> Option<std::fs::File> {
    let path = proto::paths::stderr_path();
    let dir = path.parent()?;
    std::fs::create_dir_all(dir).ok()?;
    // Decision 28: no file in `data_dir` may grow without bound. Every write this file
    // ever receives after this function returns lands through the detached child's own
    // inherited fd 2, not through any `Write` call this process controls, so nothing here
    // can cap growth *within* one daemon's run beyond what the rotation-failure
    // diagnostic's own latch already gives it (one line per failure episode —
    // `logfile::RotatingFile::recover_from_failed_rotation`); an unrelated panic is
    // simply not something this file's opener can bound. What this function *can* do, at
    // the one moment this process does control the file, is stop growth from carrying
    // across restarts: open it through `logfile::RotatingFile` — the same size/keep
    // policy `daemon.log` uses, not a second one invented just for this file — and force
    // its own over-the-cap check once, with nothing new to write. A file a previous run
    // pushed past the cap gets rotated out of the way before this run's fd is handed to
    // the child; a file already within the cap is left untouched.
    if let Ok(mut rotating) = daemon::logfile::RotatingFile::open(
        dir,
        "daemon.stderr.log",
        daemon::logfile::LOG_MAX_BYTES,
        daemon::logfile::LOG_KEEP,
    ) {
        rotating.rotate_if_over_cap();
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

#[cfg(test)]
mod tests {
    use super::*;

    // `ANTHREX_DATA_DIR` is process-global; serialize the tests that touch it, matching
    // `proto::paths`'s own `ENV_LOCK` convention.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Re-review Major #1: `daemon.stderr.log` is opened once, appended to forever, with
    /// no cap of its own — across many restarts of a daemon whose directory keeps going
    /// read-only and coming back (each episode contributing one latched diagnostic line;
    /// see `logfile::rotation_failure_diagnostic_is_reported_once_per_episode`), or across
    /// enough uncaught panics, the file a fresh `spawn_detached` inherits could already be
    /// arbitrarily large. `open_stderr_sink` must not hand that file to a new child
    /// unbounded: a previous run that already pushed it over `daemon.log`'s own cap must
    /// be rotated out of the way before this run starts appending.
    #[test]
    fn open_stderr_sink_rotates_away_a_previous_runs_oversized_file() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        // SAFETY: serialized by ENV_LOCK; nothing else in this process reads the variable
        // concurrently.
        unsafe { std::env::set_var("ANTHREX_DATA_DIR", &data_dir) };

        let oversized = vec![b'x'; (daemon::logfile::LOG_MAX_BYTES + 4096) as usize];
        std::fs::write(data_dir.join("daemon.stderr.log"), &oversized).unwrap();

        let file = open_stderr_sink();
        drop(file);

        // SAFETY: see above.
        unsafe { std::env::remove_var("ANTHREX_DATA_DIR") };

        let new_len = std::fs::metadata(data_dir.join("daemon.stderr.log"))
            .unwrap()
            .len();
        assert!(
            new_len < oversized.len() as u64,
            "a sink already over the cap from a previous run must be rotated at open, \
             not handed to a new child to keep growing: {new_len} bytes"
        );
        assert!(
            data_dir.join("daemon.stderr.log.1").exists(),
            "the previous run's content must survive the rotation, not be discarded: \
             expected daemon.stderr.log.1 to exist"
        );
    }

    /// An ordinary, well-behaved sink (nothing has ever pushed it over the cap) must be
    /// left exactly as it was — this is a cap check at open time, not an unconditional
    /// rotation on every daemon start.
    #[test]
    fn open_stderr_sink_leaves_a_small_file_untouched() {
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let data_dir = dir.path().join("data");
        std::fs::create_dir_all(&data_dir).unwrap();
        unsafe { std::env::set_var("ANTHREX_DATA_DIR", &data_dir) };

        std::fs::write(
            data_dir.join("daemon.stderr.log"),
            b"a small existing line\n",
        )
        .unwrap();

        let file = open_stderr_sink();
        drop(file);

        unsafe { std::env::remove_var("ANTHREX_DATA_DIR") };

        assert!(
            !data_dir.join("daemon.stderr.log.1").exists(),
            "a file well under the cap must not be rotated"
        );
        let contents = std::fs::read(data_dir.join("daemon.stderr.log")).unwrap();
        assert_eq!(contents, b"a small existing line\n");
    }
}
