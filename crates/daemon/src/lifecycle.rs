//! Socket setup, logging, pid file, signals, and the top-level daemon loop. Spec section 3.7.

use crate::lockfile::{Acquired, DaemonLock};
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
    /// Where `config.toml` lives. Loaded by `run` with `config::load` before the manager
    /// is built, so `runtimes.*` can resolve `ManagerConfig.claude_bin`/`codex_bin`.
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

/// Synchronous counterpart to `spawn::is_up` (`crates/tui/src/spawn.rs`): a plain,
/// blocking connect attempt, usable as the `already_running` probe inside
/// `DaemonLock::acquire_or_yield`'s own blocking retry loop below — that loop runs
/// synchronously on whatever thread is executing this async fn (not on
/// `spawn_blocking`; see the comment at its call site), so the probe it takes must be
/// synchronous too.
fn daemon_is_reachable(socket: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(socket).is_ok()
}

/// Runs the daemon in the current process until a signal or a client asks it to stop.
///
/// Decision 24: the lifetime lock is acquired first, right after the data directory
/// exists and before anything else touches it — logging, the socket, `daemon.pid` — so
/// two daemons can never share a data directory. `_lock` is otherwise unused: dropping it
/// at the end of this function, after every other cleanup below has run, is what releases
/// it (decision 24, decision 26).
///
/// Fix wave 10, item 1: acquiring the lock is not actually this function's goal — a
/// live daemon on `opts.socket_path` is. `DaemonLock::acquire_or_yield` is given a
/// probe for exactly that, so a caller that loses the race to become the daemon (the
/// two-`ensure_daemon`-calls scenario the review reproduced) stops waiting the moment
/// the winner's socket answers, rather than treating "the lock is now free" — which can
/// become true moments after the winner *stops*, e.g. via `anthrex daemon stop` — as
/// its cue to become a second, unrequested daemon.
pub async fn run(opts: DaemonOptions) -> anyhow::Result<()> {
    // Test-only: `crates/cli/tests/daemon_spawn_stderr.rs`'s
    // `detached_daemon_captures_its_stderr_to_a_file` needs to assert that a detached
    // daemon's *actual* stderr writes reach `daemon.stderr.log`, not just that the file
    // exists — `open_stderr_sink` (`crates/tui/src/spawn.rs`) creates the file as a
    // side effect in the CLI parent regardless of whether the child's stdio is ever
    // wired to it, which is exactly what let that test stay green after a mutation
    // that discarded the child's stderr outright. Forcing a real rotation failure to
    // get content onto this path needs a read-only directory or a full disk, so this
    // writes one known, harmless line to the real stderr fd instead, gated behind an
    // env var no real deployment sets.
    if std::env::var_os("ANTHREX_TEST_STDERR_PROBE").is_some() {
        let _ = std::io::Write::write_all(&mut std::io::stderr(), b"anthrex-test-stderr-probe\n");
    }
    std::fs::create_dir_all(&opts.data_dir)?;
    // Whole-branch-review Minor m5: `acquire_or_yield` runs its retry loop
    // synchronously on whatever thread calls it (`std::thread::sleep`, not
    // `tokio::time::sleep` — see its own doc comment) and is called directly from this
    // async fn, not via `spawn_blocking`, which is AGENTS.md hard rule 2 as literally
    // written ("no blocking work under ... a tokio worker thread"). The exception is
    // safe *here* specifically: `run` is called once, at the very top of the daemon's
    // startup, before the socket is bound, before the manager exists, and before
    // anything else is scheduled on this runtime — there is nothing else this blocked
    // worker thread could be starving. It would not be safe to call this anywhere
    // else, including a later refactor of this same function, without moving it back
    // onto `spawn_blocking` first.
    let _lock = match DaemonLock::acquire_or_yield(&opts.data_dir, opts.lock_wait, || {
        daemon_is_reachable(&opts.socket_path)
    })? {
        Acquired::Locked(lock) => lock,
        Acquired::AlreadyRunning => {
            // Nothing has been created yet beyond `opts.data_dir` itself (idempotent to
            // recreate), so there is nothing to unwind here.
            //
            // Whole-branch-review Major 4: this used to `eprintln!` and `return Ok(())`
            // — fix wave 10 needed `B` (the loser of a race between two `ensure_daemon`
            // callers) to stop waiting and return the instant it saw a live daemon,
            // rather than possibly winning the lock later and becoming a second,
            // unrequested one, but that fix accidentally ate decision 24's own refusal
            // along with it: `anthrex daemon start --foreground` against a live daemon
            // started exiting 0 instead of failing with "another anthrex daemon is
            // running", failing the brief's own manual check 10. The two are separable:
            // not becoming a phantom daemon only needs this arm to bind nothing and
            // return promptly, which an `Err` does exactly as well as an `Ok` did — nothing
            // downstream of `ensure_daemon`'s `spawn_detached` inspects this process's
            // exit code (the phantom-avoidance guarantee lives entirely in
            // `acquire_or_yield`'s lock-and-probe mechanism above), so restoring the
            // refusal here is free for that caller and restores the intended failure
            // for `--foreground`'s.
            anyhow::bail!(
                "another anthrex daemon is running with data directory {} (pid {})",
                opts.data_dir.display(),
                crate::lockfile::holder_pid(&opts.data_dir)
            );
        }
    };
    let _log_guard = init_logging(&opts.data_dir)?;

    // Decision 12: loaded once, on `spawn_blocking`, before the socket is bound, so the
    // first client's `Welcome` already lists every restored window. Every problem `load`
    // found is logged at the severity it carries (decision 12's whole reason for
    // returning `Problem` rather than a bare `String`): an unreadable file is a real,
    // if recoverable, loss of this boot's window list (`error`); a corrupt/unsupported
    // file or a skipped record is routine recovery the module already handled on its own
    // (`warn`).
    let state_path = opts.data_dir.join("state.json");
    let (loaded_state, problems) = {
        let path = state_path.clone();
        tokio::task::spawn_blocking(move || crate::state::load(&path)).await?
    };
    for problem in &problems {
        match problem.severity {
            crate::state::Severity::Error => {
                tracing::error!(problem = %problem, "state file problem");
            }
            crate::state::Severity::Warn => {
                tracing::warn!(problem = %problem, "state file problem");
            }
        }
    }

    // Config decision 7: loaded once, on `spawn_blocking` like the state file above and
    // for the same reason (AGENTS.md hard rule 2 — file I/O that can stall must not run
    // on a tokio worker thread), and before the socket bind so `runtimes.*` is already
    // resolved into `ManagerConfig` before the first client can connect. Each problem is
    // logged at `warn` (decision 7: "the daemon logs each problem at `warn` once, at
    // start"); unlike the state file's `Problem`, `config::Problem` carries no severity of
    // its own to preserve, so there is nothing to flatten here.
    let config_path = opts.config_path.clone();
    let (loaded_config, config_problems) =
        tokio::task::spawn_blocking(move || config::load(&config_path)).await?;
    for problem in &config_problems {
        tracing::warn!(problem = %problem, "config problem");
    }

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    let mut config =
        ManagerConfig::from_env(opts.socket_path.clone(), shell, &loaded_config.runtimes)?;
    config.worktrees_root = opts.data_dir.join("worktrees");
    // Fix wave 4, item 1: `codex_bin` is cloned out here, before `WindowManager::new`
    // consumes `config`, so the probe itself can run later — immediately before
    // `server::serve`, not here. Only decision 12's state-file load needs to precede the
    // bind; the probe was moved up alongside it by accident in an earlier change, and a
    // `codex` that takes longer than `spawn::ensure_daemon`'s 3 s socket-wait (well inside
    // the probe's own 5 s budget) made every auto-spawning entry point report a failed
    // start even though the daemon was starting up fine.
    let codex_bin = config.codex_bin.clone();
    let (manager, mut events) = WindowManager::new(config);
    // Decision 12/14: every restored window is listed, dormant and viewable before
    // anything can connect.
    manager.restore(loaded_state);

    prepare_socket(&opts.socket_path)?;
    let listener = bind_socket(&opts.socket_path)?;
    // Decision 26: the socket is only ever unlinked at shutdown if this is still the same
    // file — an inode match, not a path match — so a replacement daemon (or, in the test
    // that exercises this, a plain listener standing in for one) is never touched.
    let socket_id = std::fs::metadata(&opts.socket_path).map(|m| (m.dev(), m.ino()))?;
    let pid_path = opts.data_dir.join("daemon.pid");
    std::fs::write(&pid_path, std::process::id().to_string())?;
    tracing::info!(socket = %opts.socket_path.display(), pid = std::process::id(), "daemon started");

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

    // Decision 9: keeps `state.json` current for as long as the daemon runs. Started
    // only after the socket is bound (nothing for it to persist could have changed
    // before this point) and stopped, below, before decision 11's own final write —
    // there is exactly one writer of this file at any instant.
    let persister =
        crate::state::spawn_persister(manager.clone(), state_path.clone(), shutdown.clone());

    // Complete the only version probe before any window launch is accepted — `serve` is
    // what actually accepts window launches, so immediately before it is where this
    // invariant is preserved without also holding up the socket bind above (fix wave 4,
    // item 1).
    codex_version::check(codex_bin).await;

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

    // Decision 11's exact order: the persister is stopped and *awaited* — not merely
    // signalled — before the final flush runs, so the two can never both be writing
    // `state.json.tmp` at once. `shutdown` is already cancelled by construction (`serve`
    // only ever returns after it is), but cancelling it again here is free and makes the
    // invariant hold even if a future change to `serve` ever returns some other way.
    shutdown.cancel();
    let _ = persister.await;
    let final_state = manager.state_snapshot();
    let final_path = state_path.clone();
    let final_save_succeeded =
        match tokio::task::spawn_blocking(move || crate::state::save(&final_path, &final_state))
            .await
        {
            Ok(Ok(())) => true,
            Ok(Err(error)) => {
                tracing::error!(%error, "final state save failed");
                false
            }
            Err(error) => {
                tracing::error!(%error, "final state save task panicked");
                false
            }
        };

    // Decision 11's advertised guarantee is that a socket file that has disappeared means
    // the state is on disk. Unlinking unconditionally would make that false exactly when
    // it matters, so the socket is only ever removed once the final flush actually
    // succeeded (fix wave 4, item 5). Nothing in the tree waits on the socket rather than
    // the lifetime lock file today, so this changes nothing on the success path; on a
    // failed flush it leaves the socket in place rather than falsifying the guarantee —
    // cheap, since `prepare_socket` already clears a stale socket file on the next boot.
    if final_save_succeeded {
        let socket_is_still_ours = std::fs::metadata(&opts.socket_path)
            .map(|m| (m.dev(), m.ino()) == socket_id)
            .unwrap_or(false);
        if socket_is_still_ours {
            let _ = std::fs::remove_file(&opts.socket_path);
        }
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
    /// The starting mask is deliberately `0o037`, not `0o022`: `0o022` is both the value
    /// this test used to set *and* the value it asserted was restored, so a mutation that
    /// hard-coded the restore to `unsafe { libc::umask(0o022) }` instead of round-tripping
    /// `previous_umask` passed unchanged (finding 2, M6 fix wave 1). `0o037` is also not a
    /// common real-world default, unlike `0o022`, so a mutation that hard-codes some other
    /// plausible-looking default is caught too.
    ///
    /// Holds `UMASK_LOCK` for the whole sequence and calls the lock-free
    /// `bind_socket_locked` directly (not the public `bind_socket`, which would try to
    /// take the same lock and deadlock): umask is process-wide, and other tests in this
    /// binary call `bind_socket` concurrently, so without holding the lock across its own
    /// two raw `umask` calls too this test is racy against them. Whether `bind_socket`
    /// itself takes `UMASK_LOCK` is a separate property, covered by
    /// `bind_socket_serializes_concurrent_umask_use` below, which goes through the public
    /// entry point precisely because this test cannot (see that test's doc comment).
    #[tokio::test]
    async fn bind_restores_the_umask() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("d.sock");
        let _guard = crate::lock(&UMASK_LOCK);
        // SAFETY: umask has no preconditions and cannot fail; both calls here bracket the
        // temporary value this test sets so the process's real mask is restored after.
        // `UMASK_LOCK` is held for the whole bracket, so no concurrently running test can
        // observe or clobber the value in between.
        let real_mask = unsafe { libc::umask(0o037) };
        let result = bind_socket_locked(&sock);
        // SAFETY: see above.
        let mask_after_bind = unsafe { libc::umask(real_mask) };
        let _listener = result.unwrap();
        assert_eq!(
            mask_after_bind, 0o037,
            "bind_socket did not restore the umask it found"
        );
        assert_eq!(
            std::fs::metadata(&sock).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    /// Finding 1, M6 fix wave 1 (task-2 review): `bind_restores_the_umask` above never
    /// calls the *public* `bind_socket` — by construction it can't, since it holds
    /// `UMASK_LOCK` itself and `bind_socket` would deadlock trying to take the same lock.
    /// So deleting `bind_socket`'s `let _guard = crate::lock(&UMASK_LOCK);` line left
    /// every test in this module green.
    ///
    /// `libc::umask` is process-global, so without that lock, concurrent `bind_socket`
    /// calls' read-tighten-restore sequences can interleave: thread A reads the ambient
    /// mask and tightens to `0o077`; thread B, running concurrently, reads *A's* `0o077`
    /// as if it were ambient and later restores to that instead of to what was truly
    /// ambient before either started. The mask observed once every call has returned is
    /// then wrong, even though neither call did anything incorrect in isolation.
    ///
    /// This drives many concurrent calls through the public `bind_socket` and checks the
    /// ambient mask survives the burst unchanged. Reading the mask itself, before and
    /// after, is bracketed by `UMASK_LOCK` too — not part of what's under test, since
    /// `bind_socket` takes the very same lock as long as its guard line still exists; this
    /// only keeps those two reads race-free against the burst and against
    /// `bind_restores_the_umask`'s own direct umask manipulation.
    #[test]
    fn bind_socket_serializes_concurrent_umask_use() {
        let dir = tempfile::tempdir().unwrap();

        let starting = {
            let _guard = crate::lock(&UMASK_LOCK);
            // SAFETY: umask has no preconditions and cannot fail. `UMASK_LOCK` is held
            // for both calls, so this round trip is race-free.
            unsafe {
                let m = libc::umask(0o000);
                libc::umask(m);
                m
            }
        };

        // `bind_socket` calls `tokio::net::UnixListener::bind`, which needs an active
        // Tokio reactor context even though `bind_socket` itself is synchronous; each
        // plain `std::thread` below enters this runtime's context explicitly rather than
        // being spawned as a `#[tokio::test]` task, so the burst is genuine OS-thread
        // concurrency, not tasks cooperatively yielding on one or a few worker threads.
        let rt = tokio::runtime::Runtime::new().unwrap();
        let handle = rt.handle().clone();
        let threads: Vec<_> = (0..16)
            .map(|i| {
                let sock = dir.path().join(format!("umask-race-{i}.sock"));
                let handle = handle.clone();
                std::thread::spawn(move || {
                    let _guard = handle.enter();
                    let _listener = bind_socket(&sock).unwrap();
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }

        let ending = {
            let _guard = crate::lock(&UMASK_LOCK);
            // SAFETY: see above.
            unsafe {
                let m = libc::umask(starting);
                libc::umask(m);
                m
            }
        };
        assert_eq!(
            ending, starting,
            "16 concurrent bind_socket calls left the ambient umask changed: {ending:o} != {starting:o}"
        );
    }
}
