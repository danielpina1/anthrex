//! Starts the daemon in the background when none is running.
//!
//! Moved here from `crates/cli/src/spawn.rs` by task M6.11's `git mv`: `ensure_daemon`
//! is now also the second half of `reconnect::attempt` (decision 32's `C-b r`, which
//! may start a daemon the same way a cold `anthrex attach` does), so it belongs next
//! to `reconnect.rs` and `connection.rs` rather than behind the CLI crate boundary.

use std::fs::OpenOptions;
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};
use tokio::net::UnixStream;

async fn is_up(socket: &Path) -> bool {
    UnixStream::connect(socket).await.is_ok()
}

/// How long `ensure_daemon` holds its transient startup-lock claim after spawning a
/// detached child, before releasing it so that child can make its own, real claim. See
/// `ensure_daemon`'s doc comment for what this does and does not close.
const SPAWN_HANDOFF_GRACE: Duration = Duration::from_millis(250);

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

/// Re-executes `exe` as `anthrex daemon start --foreground` in its own session, with
/// stdio detached, so it outlives the calling process.
fn spawn_detached(exe: &Path) -> anyhow::Result<()> {
    use std::os::unix::process::CommandExt;
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

/// What deciding whether to spawn found (see [`ensure_daemon`]'s doc comment).
enum Claim {
    /// Nobody held `daemon.lock`: this caller claimed it and must keep holding it
    /// until the detached child has actually been launched.
    Won(daemon::lockfile::DaemonLock),
    /// Somebody else holds it right now.
    Contended,
    /// The probe itself failed for a reason other than contention (most likely: the
    /// data directory did not exist yet). Tells us nothing about contention.
    Unknown,
}

/// Connects if a daemon is running; otherwise starts one and waits up to 3 s for its
/// socket. `exe` is the binary to re-exec as the detached daemon — task M6.11 moved
/// this out of an internal `std::env::current_exe()` call so a reconnect attempt
/// (decision 32) can be driven with a fixed, test-supplied path instead of whatever
/// binary happens to be running the test harness.
///
/// Fix wave 10, item 1's other half (`daemon::lockfile::DaemonLock::acquire_or_yield`
/// is the first): two `ensure_daemon` calls that each see "not up" at nearly the same
/// instant used to each `spawn_detached` their own competing daemon-start child — the
/// exact shape of race the review reproduced (a loser left racing the winner's own
/// lock long after this function's caller has stopped caring). Rather than only
/// detecting that after the fact once both children are already running, the `Claim`
/// below removes the redundant child at the source: only the caller that can promptly
/// claim the daemon's own startup lock (`daemon::lockfile::try_claim`) spawns
/// anything. Critically, a winning claim is *held* across the `spawn_detached` call
/// itself, not released the instant it is taken — an earlier version of this fix
/// released it immediately, which meant a sibling caller checking even a few
/// microseconds later still saw the lock as free and made the exact same "nobody has
/// this" decision, spawning its own redundant child anyway. Holding it across the
/// spawn closes that: any sibling that checks while this caller is still inside
/// `spawn_detached` sees genuine contention and does not spawn.
///
/// A caller that loses the claim spawns nothing and simply waits for the winner's
/// socket in the loop below — exactly how it already waits for a daemon started for
/// any other reason. A probe that fails for a reason *other* than contention falls
/// back to the old, unconditional behaviour — an unrelated I/O hiccup must never be
/// read as "someone else has this covered," or nobody would ever spawn anything.
/// `spawn_blocking`: this touches the filesystem (AGENTS.md hard rule 2), even though
/// the check itself never waits.
///
/// `SPAWN_HANDOFF_GRACE` (below): `spawn_detached` returning only means fork/exec has
/// been *initiated* — the detached child still has its own real startup ahead of it
/// (dynamic linking, the tokio runtime, argument parsing) before it reaches its own
/// [`daemon::lockfile::DaemonLock::acquire`] call and re-claims this lock for real.
/// Dropping this caller's transient claim the instant `spawn_detached` returns reopens
/// almost exactly the same gap this whole mechanism exists to close: a sibling
/// `ensure_daemon` call's own `try_claim`, checked in that narrow window, would see
/// "uncontended" and spawn a redundant child of its own. Holding the claim a bit
/// longer narrows that window a great deal; it does not close it — there is no signal
/// available here for "the child has now made its own claim," short of a real
/// parent/child handoff protocol (e.g. passing the already-locked fd across `exec`),
/// which is a bigger change than this fix wave makes. See
/// `.superpowers/sdd/M6-persistence/fix-wave-10-report.md` for the residual this
/// leaves, measured against a real binary.
pub async fn ensure_daemon(exe: &Path, socket: &Path) -> anyhow::Result<()> {
    if is_up(socket).await {
        return Ok(());
    }
    let data_dir = proto::paths::data_dir();
    let claim = tokio::task::spawn_blocking(move || {
        if std::fs::create_dir_all(&data_dir).is_err() {
            return Claim::Unknown;
        }
        match daemon::lockfile::try_claim(&data_dir) {
            Ok(Some(lock)) => Claim::Won(lock),
            Ok(None) => Claim::Contended,
            Err(_) => Claim::Unknown,
        }
    })
    .await
    .unwrap_or(Claim::Unknown);
    match claim {
        Claim::Contended => {}
        Claim::Unknown => {
            spawn_detached(exe)?;
        }
        Claim::Won(lock) => {
            spawn_detached(exe)?;
            // Give the just-spawned child a real chance to reach its own acquire
            // before releasing: `docs/timing-budgets.md` measured a plain `anthrex`
            // binary spawn at a 39ms median / 85ms max (idle) — this is several times
            // that, not a tight bound, since the cost of being wrong (a redundant
            // second daemon) is much higher than the cost of a slightly slower start
            // for the one caller that won the race.
            tokio::time::sleep(SPAWN_HANDOFF_GRACE).await;
            drop(lock);
        }
    }
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
    use crate::ENV_LOCK;

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
        let _guard = ENV_LOCK.blocking_lock();
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
        let _guard = ENV_LOCK.blocking_lock();
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
