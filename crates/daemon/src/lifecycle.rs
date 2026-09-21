//! Socket setup, logging, pid file, signals, and the top-level daemon loop. Spec section 3.7.

use crate::lockfile::DaemonLock;
use crate::manager::{ManagerConfig, WindowManager};

mod codex_version;
use crate::server;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::net::UnixListener;
use tokio_util::sync::CancellationToken;

pub struct DaemonOptions {
    pub socket_path: PathBuf,
    pub data_dir: PathBuf,
    /// Not read by this milestone's task yet; carried so a later task can load
    /// `config.toml` at the same point `run` already reads everything else it needs.
    pub config_path: PathBuf,
    /// How long [`DaemonLock::acquire`] retries before giving up. The CLI passes
    /// [`LOCK_WAIT`]; tests pass [`Duration::ZERO`] so a locked-out daemon fails fast.
    pub lock_wait: Duration,
}

/// How long the CLI's own daemon start waits for another daemon's lock to clear — long
/// enough to cover a `daemon stop` immediately followed by a start (decision 24).
pub const LOCK_WAIT: Duration = Duration::from_secs(5);

/// Creates the socket directory (mode 0700) and removes a stale socket file.
/// Fails if a live daemon answers on the socket, or if the directory belongs to
/// somebody else.
pub fn prepare_socket(path: &Path) -> anyhow::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
        if let Err(e) = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)) {
            // SAFETY: getuid has no preconditions and cannot fail.
            let current_uid = unsafe { libc::getuid() };
            let meta = std::fs::metadata(dir)?;
            let owner_uid = meta.uid();
            if owner_uid == current_uid {
                // We own this directory: the 0700 guarantee must hold, so a chmod
                // failure here is a real problem and must be fatal.
                return Err(e.into());
            }
            // A directory owned by someone else is the attack case, not a nuisance:
            // without XDG_RUNTIME_DIR the default path is /tmp/anthrex-<uid>, which
            // another local user can create first and then read every keystroke we
            // send through the socket inside it. The one safe exception is a
            // root-owned sticky directory (/tmp, /var/tmp): sticky means no one but
            // the owner can remove or rename what we create there, so the socket we
            // bind and chmod 0600 below is still ours alone.
            let sticky = meta.mode() & 0o1000 != 0;
            if !(owner_uid == 0 && sticky) {
                anyhow::bail!(
                    "refusing to use the socket directory {}: it is owned by uid {owner_uid}, not by you (uid {current_uid}), \
                     and is not a root-owned sticky directory; set ANTHREX_SOCKET to a path you own ({e})",
                    dir.display()
                );
            }
            tracing::warn!(dir = %dir.display(), error = %e, "socket directory is a shared sticky directory, not 0700");
        }
    }
    if path.exists() {
        match std::os::unix::net::UnixStream::connect(path) {
            Ok(_) => anyhow::bail!("another daemon is already listening on {}", path.display()),
            Err(_) => std::fs::remove_file(path)?,
        }
    }
    Ok(())
}

/// Binds the listening socket and restricts it to its owner.
///
/// The socket directory can legitimately be a shared sticky directory (/tmp), so the
/// socket's own mode is what stops another local user from connecting and driving every
/// agent the daemon owns. Decision 25: the umask is tightened to 0o077 for the bind call
/// itself, then restored, so the socket is born 0600 without depending on whatever mask
/// the process happened to start with; the explicit chmod below is a second, independent
/// guard for the same property.
///
/// `libc::umask` is process-global, not per-thread, so every caller in this process must
/// serialize around it or one caller's restore can clobber another's — the daemon itself
/// only ever binds one socket at startup, but the test suite calls this from many
/// concurrent test threads. [`UMASK_LOCK`] is that serialization; [`bind_socket_locked`]
/// is the lock-free implementation it wraps, exposed separately so
/// `bind_restores_the_umask` below can hold the lock across its own umask manipulation
/// *and* this call without deadlocking on a lock it already holds.
static UMASK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub fn bind_socket(path: &Path) -> anyhow::Result<UnixListener> {
    let _guard = crate::lock(&UMASK_LOCK);
    bind_socket_locked(path)
}

fn bind_socket_locked(path: &Path) -> anyhow::Result<UnixListener> {
    // SAFETY: `umask` has no preconditions and cannot fail; it only ever changes this
    // process's file-creation mask, which we restore immediately below on every path.
    // Callers serialize through `UMASK_LOCK`.
    let previous_umask = unsafe { libc::umask(0o077) };
    let bound = UnixListener::bind(path);
    // SAFETY: see above. Restored before `?` so a bind failure never leaves the tighter
    // mask in place.
    unsafe { libc::umask(previous_umask) };
    let listener = bound?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// `try_init` rather than `init`: production only ever calls this once, but a test
/// process that runs `run` more than once (this milestone's `tests/lifecycle.rs`) must
/// not panic on the second global-subscriber install.
///
/// Decision 28: `daemon.log` rotates by size through [`crate::logfile::RotatingFile`]
/// rather than tracing_appender's non-rotating `never` appender, which lets the file grow
/// without bound for as long as the daemon runs.
fn init_logging(data_dir: &Path) -> anyhow::Result<tracing_appender::non_blocking::WorkerGuard> {
    let file = crate::logfile::RotatingFile::open(
        data_dir,
        "daemon.log",
        crate::logfile::LOG_MAX_BYTES,
        crate::logfile::LOG_KEEP,
    )?;
    let (writer, guard) = tracing_appender::non_blocking(file);
    let filter = tracing_subscriber::EnvFilter::try_from_env("ANTHREX_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_writer(writer)
        .try_init();
    Ok(guard)
}

/// Runs the daemon in the current process until a signal or a client asks it to stop.
///
/// Decision 24: the lifetime lock is acquired first, right after the data directory
/// exists and before anything else touches it — logging, the socket, `daemon.pid` — so
/// two daemons can never share a data directory. `_lock` is otherwise unused: dropping it
/// at the end of this function, after every other cleanup below has run, is what releases
/// it (decision 24, decision 26).
pub async fn run(opts: DaemonOptions) -> anyhow::Result<()> {
    std::fs::create_dir_all(&opts.data_dir)?;
    let _lock = DaemonLock::acquire(&opts.data_dir, opts.lock_wait)?;
    let _log_guard = init_logging(&opts.data_dir)?;
    prepare_socket(&opts.socket_path)?;
    let listener = bind_socket(&opts.socket_path)?;
    // Decision 26: the socket is only ever unlinked at shutdown if this is still the same
    // file — an inode match, not a path match — so a replacement daemon (or, in the test
    // that exercises this, a plain listener standing in for one) is never touched.
    let socket_id = std::fs::metadata(&opts.socket_path).map(|m| (m.dev(), m.ino()))?;
    let pid_path = opts.data_dir.join("daemon.pid");
    std::fs::write(&pid_path, std::process::id().to_string())?;
    tracing::info!(socket = %opts.socket_path.display(), pid = std::process::id(), "daemon started");

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut config = ManagerConfig::from_env(opts.socket_path.clone(), shell)?;
    config.worktrees_root = opts.data_dir.join("worktrees");
    // Complete the only version probe before any window launch is accepted.
    codex_version::check(config.codex_bin.clone()).await;
    let (manager, mut events) = WindowManager::new(config);
    let shutdown = CancellationToken::new();

    let pump = manager.clone();
    tokio::spawn(async move {
        while let Some((id, ev)) = events.recv().await {
            pump.handle_event(id, ev);
        }
    });

    let ticker = manager.clone();
    let tick_token = shutdown.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        loop {
            tokio::select! {
                _ = tick_token.cancelled() => break,
                _ = interval.tick() => ticker.tick(),
            }
        }
    });

    let signal_token = shutdown.clone();
    tokio::spawn(async move {
        let mut term = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
        tracing::info!("signal received, shutting down");
        signal_token.cancel();
    });

    let served = server::serve(
        listener,
        manager.clone(),
        crate::git::enabled_from_env(),
        shutdown.clone(),
    )
    .await;
    if let Err(e) = &served {
        tracing::error!(error = %e, "server exited with an error");
    }
    tracing::info!("stopping agents");
    manager.shutdown().await;
    let socket_is_still_ours = std::fs::metadata(&opts.socket_path)
        .map(|m| (m.dev(), m.ino()) == socket_id)
        .unwrap_or(false);
    if socket_is_still_ours {
        let _ = std::fs::remove_file(&opts.socket_path);
    }
    let _ = std::fs::remove_file(&pid_path);
    tracing::info!("daemon stopped");
    // `_lock` drops here, after every cleanup above, releasing the flock last.
    served
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_socket_file_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("sub").join("d.sock");
        std::fs::create_dir_all(sock.parent().unwrap()).unwrap();
        std::fs::write(&sock, b"not a socket").unwrap();
        prepare_socket(&sock).unwrap();
        assert!(!sock.exists());
        let mode = std::fs::metadata(sock.parent().unwrap())
            .unwrap()
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o700);
    }

    #[test]
    fn live_socket_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("d.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        let err = prepare_socket(&sock).unwrap_err();
        assert!(err.to_string().contains("already"));
        assert!(sock.exists());
    }

    /// I5: a root-owned sticky directory (/tmp) is the deliberate override and stays
    /// allowed - nobody but us can replace what we create inside it.
    #[test]
    fn root_owned_sticky_parent_is_tolerated() {
        let tmp = PathBuf::from("/tmp");
        // SAFETY: getuid has no preconditions and cannot fail.
        let current_uid = unsafe { libc::getuid() };
        let meta = std::fs::metadata(&tmp).unwrap();
        if meta.uid() == current_uid {
            // Running as root (or otherwise owns /tmp): the scenario this test exercises
            // (a parent directory we don't own) doesn't apply here.
            return;
        }
        assert_eq!(meta.uid(), 0, "/tmp is expected to be root-owned");
        assert_ne!(meta.mode() & 0o1000, 0, "/tmp is expected to be sticky");
        let sock = tmp.join(format!("anthrex-prep-{}.sock", std::process::id()));
        assert!(!sock.exists());
        let result = prepare_socket(&sock);
        assert!(result.is_ok(), "{result:?}");
    }

    /// I5: any other directory we do not own is refused by name, because an attacker who
    /// pre-created it would otherwise see every keystroke going through the socket.
    #[test]
    fn foreign_owned_parent_without_the_sticky_bit_is_fatal() {
        // SAFETY: getuid has no preconditions and cannot fail.
        let current_uid = unsafe { libc::getuid() };
        if current_uid == 0 {
            return; // root can chmod anything, so there is no failure to observe.
        }
        // /usr is root-owned and NOT sticky: exactly the shape of a directory another
        // user pre-created for us.
        let dir = PathBuf::from("/usr");
        let meta = std::fs::metadata(&dir).unwrap();
        assert_ne!(meta.uid(), current_uid);
        assert_eq!(meta.mode() & 0o1000, 0);
        let err = prepare_socket(&dir.join("anthrex-should-never-bind.sock"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("/usr"), "{err}");
        assert!(err.contains(&format!("uid {}", meta.uid())), "{err}");
    }

    /// I5: the socket file itself must end up 0600, since the directory may be shared.
    #[tokio::test]
    async fn the_bound_socket_is_chmod_0600() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("d.sock");
        prepare_socket(&sock).unwrap();
        let _listener = bind_socket(&sock).unwrap();
        assert_eq!(
            std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    /// Decision 25: `bind_socket` must leave the process's umask exactly as it found it,
    /// whatever that was — a mutation that forgot to restore it, or restored a hardcoded
    /// value instead of the one it read, would leak a tightened (or loosened) mask into
    /// every file this process creates afterwards.
    ///
    /// Holds `UMASK_LOCK` for the whole sequence and calls the lock-free
    /// `bind_socket_locked` directly (not the public `bind_socket`, which would try to
    /// take the same lock and deadlock): umask is process-wide, and other tests in this
    /// binary call `bind_socket` concurrently, so without holding the lock across its own
    /// two raw `umask` calls too this test is racy against them.
    #[tokio::test]
    async fn bind_restores_the_umask() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("d.sock");
        let _guard = crate::lock(&UMASK_LOCK);
        // SAFETY: umask has no preconditions and cannot fail; both calls here bracket the
        // temporary value this test sets so the process's real mask is restored after.
        // `UMASK_LOCK` is held for the whole bracket, so no concurrently running test can
        // observe or clobber the value in between.
        let real_mask = unsafe { libc::umask(0o022) };
        let result = bind_socket_locked(&sock);
        // SAFETY: see above.
        let mask_after_bind = unsafe { libc::umask(real_mask) };
        let _listener = result.unwrap();
        assert_eq!(
            mask_after_bind, 0o022,
            "bind_socket did not restore the umask it found"
        );
        assert_eq!(
            std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
