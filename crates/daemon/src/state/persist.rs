//! `save` and `spawn_persister` — the write half of this module, split from `load`
//! (the read half) and from `mod.rs` (the data model both sides share). A submodule of
//! `state` (AGENTS.md rule 8), pulling `StateFile` in via `use super::*`.

use super::*;

/// Serializes `state` and atomically replaces `path` with it (decision 10). Blocking.
///
/// Writes `<path>.tmp` created with mode `0o600` (prompts in `initial_prompt` can be
/// private), calls `sync_all` on it, renames it over `path`, then opens `path`'s parent
/// directory and calls `sync_all` on *that* — the step most often skipped, and the one
/// that makes the rename itself durable across a power failure. A leftover `.tmp` from an
/// earlier crashed run is overwritten, not treated as an error: the temp file is opened
/// with `truncate(true)`, not created exclusively. Because `OpenOptionsExt::mode` only
/// applies when `open` actually creates the file — not when it reuses that leftover one —
/// the permissions are set explicitly after opening too, so a leftover `.tmp` with looser
/// permissions from a previous run still ends up private.
pub fn save(path: &Path, state: &StateFile) -> io::Result<()> {
    let bytes = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    let tmp_path = PathBuf::from(format!("{}.tmp", path.display()));

    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    options.mode(0o600);
    let mut file = options.open(&tmp_path)?;
    #[cfg(unix)]
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);

    std::fs::rename(&tmp_path, path)?;

    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let dir_file = std::fs::File::open(dir)?;
    dir_file.sync_all()?;

    Ok(())
}

/// How long [`spawn_persister`] waits, after a change, for further changes before it
/// writes (decision 9).
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(100);

/// The upper bound on how stale `state.json` may get under a sustained change stream.
///
/// Decision 9's collection window restarts on every new change, and that alone has no
/// upper bound: a change arriving faster than [`SAVE_DEBOUNCE`] apart, indefinitely — a
/// busy daemon with several agents running publishes on `watch()` far more often than
/// every 100 ms in the ordinary case, not an exotic one — would starve the write forever
/// (fix wave 4, item 2). Once `SAVE_MAX_DELAY` has elapsed since the *first* unwritten
/// change of a burst, [`spawn_persister`] writes unconditionally instead of waiting for
/// quiet.
///
/// Ten times `SAVE_DEBOUNCE` (one second): long enough that an ordinary burst — a name
/// typed character by character, twenty renames in a tight loop, decision 9's own
/// examples — still coalesces into a single write the way decision 9 intends, short
/// enough that the worst case this bounds (a SIGKILL, an OOM kill, or a crash landing
/// mid-burst) loses at most one second of window state since the last write. A clean
/// shutdown loses nothing regardless, because decision 11's final flush is the backstop;
/// this bound is only about how far behind the *live* file can fall while the daemon
/// keeps running.
pub const SAVE_MAX_DELAY: Duration = Duration::from_millis(SAVE_DEBOUNCE.as_millis() as u64 * 10);

/// Subscribes to `manager`'s window list and keeps `path` current (decision 9).
///
/// Every publish on [`crate::manager::WindowManager::watch`] is one *debounced* write,
/// not one write each: on a change this waits [`SAVE_DEBOUNCE`], restarting the wait on
/// every further change, so a burst — a rename typed character by character, twenty
/// renames in a tight loop — reaches disk as a single write of the *final* state rather
/// than one write per edit. That restart has no ceiling of its own, so [`SAVE_MAX_DELAY`]
/// (fix wave 4, item 2) bounds it: a change stream hotter than `SAVE_DEBOUNCE` apart,
/// sustained, still forces a write once `SAVE_MAX_DELAY` has elapsed since the first
/// unwritten change, so `state.json` cannot fall arbitrarily far behind a busy daemon.
/// The wait is skipped entirely, and this task returns at once with no write of its own,
/// the moment `shutdown` is cancelled: a debounce must never hold shutdown up, and
/// [`crate::lifecycle::run`]'s own flush after this task has stopped is what guarantees
/// the last change reaches disk (decision 11) — this loop's job is only to keep the file
/// *reasonably* current while the daemon is up.
///
/// The snapshot itself, [`crate::manager::WindowManager::state_snapshot`], is a clone
/// taken under the manager lock with no I/O under it (decision 9); only the write that
/// follows runs on [`tokio::task::spawn_blocking`]. A write is skipped when its
/// serialized bytes equal the last ones actually written, so a change that round-trips
/// to identical bytes (a rename back to the same name, a status flap that settles where
/// it started) costs no I/O. A failed write is logged at `warn` and left for the next
/// change to retry — this task never retries on its own, since decision 11's final flush
/// is the backstop for any write this loop never gets to.
pub fn spawn_persister(
    manager: Arc<crate::manager::WindowManager>,
    path: PathBuf,
    shutdown: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut changes = manager.watch();
        let mut last_written: Option<Vec<u8>> = None;
        loop {
            tokio::select! {
                _ = shutdown.cancelled() => return,
                changed = changes.changed() => {
                    if changed.is_err() {
                        // The manager itself is gone; nothing left to persist for.
                        return;
                    }
                }
            }

            // Collect further changes for SAVE_DEBOUNCE, restarting that wait on each
            // one — a fresh `sleep` future is constructed every time this inner loop
            // runs, which is what makes it restart rather than merely continue a clock
            // that started on the first change — but never later than SAVE_MAX_DELAY
            // after the *first* change of this burst (fix wave 4, item 2): `max_delay`
            // is a single `sleep` future, pinned once before the loop starts, so unlike
            // the per-change window it does not restart and so puts a hard ceiling on
            // how long a sustained stream of changes can hold the write back.
            let max_delay = tokio::time::sleep(SAVE_MAX_DELAY);
            tokio::pin!(max_delay);
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => return,
                    () = tokio::time::sleep(SAVE_DEBOUNCE) => break,
                    () = &mut max_delay => break,
                    changed = changes.changed() => {
                        if changed.is_err() {
                            return;
                        }
                    }
                }
            }

            let state = manager.state_snapshot();
            let bytes = match serde_json::to_vec_pretty(&state) {
                Ok(bytes) => bytes,
                Err(error) => {
                    tracing::warn!(%error, "failed to serialize state for persistence");
                    continue;
                }
            };
            if last_written.as_deref() == Some(bytes.as_slice()) {
                continue;
            }

            let save_path = path.clone();
            let save_state = state.clone();
            match tokio::task::spawn_blocking(move || save(&save_path, &save_state)).await {
                Ok(Ok(())) => last_written = Some(bytes),
                Ok(Err(error)) => {
                    tracing::warn!(
                        %error,
                        path = %path.display(),
                        "failed to save state file; will retry on the next change"
                    );
                }
                Err(error) => {
                    tracing::warn!(%error, "state save task panicked; will retry on the next change");
                }
            }
        }
    })
}
