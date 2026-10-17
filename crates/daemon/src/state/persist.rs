//! `save` and `spawn_persister` — the write half of this module, split from `load`
//! (the read half) and from `mod.rs` (the data model both sides share). A submodule of
//! `state` (AGENTS.md rule 8), pulling `StateFile` in via `use super::*`.

use super::*;
use std::sync::Mutex;
use tokio::sync::{mpsc, oneshot};

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

/// How long a create waits for its id to reach `state.json` before it fails
/// (`ManagerConfig::state_flush_timeout`). A write is one serialize, a write of a few
/// kilobytes and two `sync_all` calls (F_FULLFSYNC on macOS): about 8 ms measured, and
/// it can queue behind one debounced write already in flight. Five seconds is far over
/// both on a loaded disk; only a stuck writer reaches it, and then the create fails with
/// a message instead of hanging.
pub const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

/// One "write now and tell me" request: answered `Ok` once a write that covers every
/// change made before the request was sent is on disk (or was already), `Err` with the
/// write's error otherwise.
type FlushAck = oneshot::Sender<Result<(), String>>;

/// Asks the persister for an immediate write, through the persister's own loop, so
/// `state.json` still has exactly one writer and no write ever lands out of order.
///
/// `WindowManager::create` and its siblings use this to get a spent window id onto
/// disk before the window is published: after a crash the next daemon rebuilds
/// `next_id` from the file alone, so an id only the debounce knew about would be handed
/// to another window, and a client still focused on it would type into that window.
#[derive(Clone)]
pub struct FlushHandle(mpsc::UnboundedSender<FlushAck>);

impl FlushHandle {
    /// Waits for the write. Holds no lock; unbounded on its own, so the caller bounds
    /// it (`ManagerConfig::state_flush_timeout`).
    pub async fn flush(&self) -> anyhow::Result<()> {
        let (ack, answer) = oneshot::channel();
        self.0
            .send(ack)
            .map_err(|_| anyhow::anyhow!("the state file's writer has stopped"))?;
        answer
            .await
            .map_err(|_| anyhow::anyhow!("the state file's writer stopped before saving"))?
            .map_err(anyhow::Error::msg)
    }
}

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
/// A [`FlushHandle`] request skips the debounce: it is installed on `manager` here,
/// before this returns, so no create can run between the spawn and the handle existing.
/// Requests that arrive together share one write. The watch is subscribed here too,
/// rather than inside the task, so a change made right after this call is never missed.
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
    start(manager, path, shutdown, Arc::new(Mutex::new(None)))
}

fn start(
    manager: Arc<crate::manager::WindowManager>,
    path: PathBuf,
    shutdown: CancellationToken,
    restoring: Arc<Mutex<Option<StateFile>>>,
) -> tokio::task::JoinHandle<()> {
    let (flush_tx, mut flushes) = mpsc::unbounded_channel();
    manager.set_state_flusher(FlushHandle(flush_tx));
    let mut changes = manager.watch();
    let on_disk_next_id = crate::lock(&restoring)
        .as_ref()
        .map_or(0, |loaded| loaded.next_id);
    tokio::spawn(async move {
        let mut writer = Writer {
            manager,
            path,
            last_written: None,
            restoring,
            on_disk_next_id,
        };
        // `false` once every `FlushHandle` is gone (a later persister replaced this
        // one's), so a closed channel is not polled in a loop.
        let mut flush_open = true;
        loop {
            let mut acks = Vec::new();
            tokio::select! {
                _ = shutdown.cancelled() => return,
                changed = changes.changed() => {
                    if changed.is_err() {
                        // The manager itself is gone; nothing left to persist for.
                        return;
                    }
                    let collected = collect_burst(
                        &mut changes,
                        &mut flushes,
                        &mut flush_open,
                        &shutdown,
                        &mut acks,
                    );
                    if !collected.await {
                        return;
                    }
                }
                ack = flushes.recv(), if flush_open => match ack {
                    Some(ack) => acks.push(ack),
                    None => {
                        flush_open = false;
                        continue;
                    }
                },
            }
            // Every request already queued shares this write.
            while let Ok(ack) = flushes.try_recv() {
                acks.push(ack);
            }
            let result = writer.write().await;
            for ack in acks {
                let _ = ack.send(result.clone());
            }
        }
    })
}

/// [`spawn_persister`] for a daemon whose restore is still under way: started right
/// after `WindowManager::restore`, before the runs are restored, so a window opened by
/// that restore gets its id saved like any other (`manager/id_flush.rs`).
///
/// Until [`RestoreGate::finish`], the window table is half-restored: the run restore
/// removes windows, rewrites their run records and opens new ones in steps, and a crash
/// between two of them would leave a file the next daemon trusts. So until then a write
/// is `loaded`, the file exactly as this daemon read it, with only `next_id` raised to the
/// manager's, and only when it is higher than the one on disk: a restore that opens
/// nothing writes nothing, as before. After `finish`, writes are the live table again.
pub fn spawn_persister_during_restore(
    manager: Arc<crate::manager::WindowManager>,
    path: PathBuf,
    shutdown: CancellationToken,
    loaded: StateFile,
) -> (tokio::task::JoinHandle<()>, RestoreGate) {
    let restoring = Arc::new(Mutex::new(Some(loaded)));
    let task = start(manager, path, shutdown, restoring.clone());
    (task, RestoreGate(restoring))
}

/// Ends [`spawn_persister_during_restore`]'s restore mode. Not ended on drop: a startup
/// that fails before its restore is done must not write a half-restored table either.
pub struct RestoreGate(Arc<Mutex<Option<StateFile>>>);

impl RestoreGate {
    pub fn finish(self) {
        *crate::lock(&self.0) = None;
    }
}

/// Collect further changes for SAVE_DEBOUNCE, restarting that wait on each one — a fresh
/// `sleep` future is constructed every time this loop runs, which is what makes it
/// restart rather than merely continue a clock that started on the first change — but
/// never later than SAVE_MAX_DELAY after the *first* change of this burst (fix wave 4,
/// item 2): `max_delay` is a single `sleep` future, pinned once before the loop starts,
/// so unlike the per-change window it does not restart and so puts a hard ceiling on how
/// long a sustained stream of changes can hold the write back. A flush request ends the
/// wait at once. `false` means stop: shutdown, or the manager is gone.
async fn collect_burst(
    changes: &mut tokio::sync::watch::Receiver<Vec<proto::WindowInfo>>,
    flushes: &mut mpsc::UnboundedReceiver<FlushAck>,
    flush_open: &mut bool,
    shutdown: &CancellationToken,
    acks: &mut Vec<FlushAck>,
) -> bool {
    let max_delay = tokio::time::sleep(SAVE_MAX_DELAY);
    tokio::pin!(max_delay);
    loop {
        tokio::select! {
            _ = shutdown.cancelled() => return false,
            () = tokio::time::sleep(SAVE_DEBOUNCE) => return true,
            () = &mut max_delay => return true,
            changed = changes.changed() => {
                if changed.is_err() {
                    return false;
                }
            }
            ack = flushes.recv(), if *flush_open => match ack {
                Some(ack) => {
                    acks.push(ack);
                    return true;
                }
                None => *flush_open = false,
            },
        }
    }
}

/// The persister's one write path, for its debounced writes and its flushes alike.
struct Writer {
    manager: Arc<crate::manager::WindowManager>,
    path: PathBuf,
    last_written: Option<Vec<u8>>,
    /// The file as loaded while the restore runs ([`spawn_persister_during_restore`]),
    /// `None` once it is finished or for [`spawn_persister`].
    restoring: Arc<Mutex<Option<StateFile>>>,
    /// The `next_id` of the last write, or of the loaded file before any.
    on_disk_next_id: u32,
}

impl Writer {
    /// What to write now, or `None` for nothing: during the restore, the loaded file
    /// with only `next_id` moved, and only when it moved past the file's.
    fn state(&self) -> Option<StateFile> {
        let next_id = self.manager.next_id();
        match &*crate::lock(&self.restoring) {
            Some(loaded) => (next_id > self.on_disk_next_id).then(|| StateFile {
                next_id,
                ..loaded.clone()
            }),
            None => Some(self.manager.state_snapshot()),
        }
    }

    async fn write(&mut self) -> Result<(), String> {
        let Some(state) = self.state() else {
            return Ok(());
        };
        let next_id = state.next_id;
        let bytes = match serde_json::to_vec_pretty(&state) {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(%error, "failed to serialize state for persistence");
                return Err(format!("could not serialize the state file: {error}"));
            }
        };
        if self.last_written.as_deref() == Some(bytes.as_slice()) {
            return Ok(());
        }

        let save_path = self.path.clone();
        match tokio::task::spawn_blocking(move || save(&save_path, &state)).await {
            Ok(Ok(())) => {
                self.last_written = Some(bytes);
                self.on_disk_next_id = next_id;
                Ok(())
            }
            Ok(Err(error)) => {
                tracing::warn!(
                    %error,
                    path = %self.path.display(),
                    "failed to save state file; will retry on the next change"
                );
                Err(format!("could not save {}: {error}", self.path.display()))
            }
            Err(error) => {
                tracing::warn!(%error, "state save task panicked; will retry on the next change");
                Err(format!("the state save task failed: {error}"))
            }
        }
    }
}
